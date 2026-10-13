//! New-mail watcher.
//!
//! Polling is deliberately plain: sync, diff local message ids, persist events,
//! deliver them. IMAP IDLE can replace the timer later without changing the
//! event table or output formats.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use jianxin_config::AccountConfig;
use jianxin_core::App;
use jianxin_domain::{Account, MailError};
use jianxin_store::mail_event_repo::{self, MailEvent};
use serde_json::{json, Value};

const MAX_EVENTS_PER_CYCLE: u32 = 100;
const WEBHOOK_TIMEOUT: Duration = Duration::from_secs(10);

pub struct Options {
    pub interval: Option<u64>,
    pub webhook: Option<String>,
    pub notify: bool,
    pub mcp: bool,
    pub include_body: bool,
    pub json: bool,
}

pub fn run(app: &App, selector: Option<&str>, options: Options) -> Result<()> {
    let config = app.config();
    let watch = &config.general.watch;
    let interval = options.interval.unwrap_or(watch.interval_secs).max(5);
    let webhook = options.webhook.clone().or_else(|| watch.webhook.clone());
    let notify = options.notify || watch.notify;
    let accounts = accounts_to_watch(app, selector)?;

    deliver_pending(app, &accounts, &webhook, notify, &options)?;

    // Accounts already present locally are baselined. An empty account's first
    // successful sync is also treated as history, not as a flood of new mail.
    let mut baselined: HashMap<String, bool> = HashMap::new();
    loop {
        for cfg in &accounts {
            let account = cfg.to_account();
            let result = poll_account(app, cfg, &mut baselined);
            let error = result.as_ref().err().map(ToString::to_string);
            app.record_sync_status(&account.id, error.as_deref());
            if let Err(err) = result {
                eprintln!("watch {}: {err}", account.email);
            }
        }
        deliver_pending(app, &accounts, &webhook, notify, &options)?;
        app.db().release_idle_memory()?;
        std::thread::sleep(Duration::from_secs(interval));
    }
}

fn accounts_to_watch(app: &App, selector: Option<&str>) -> Result<Vec<AccountConfig>> {
    match selector {
        Some(selector) => Ok(vec![app.resolve_account(Some(selector))?]),
        None => Ok(app.account_configs().to_vec()),
    }
}

/// One polling cycle, reused by the desktop's in-process worker.
#[derive(Default)]
pub struct Poller {
    baselined: HashMap<String, bool>,
}

impl Poller {
    pub fn tick(&mut self, app: &App) -> Result<()> {
        self.tick_selected(app, None)
    }

    pub fn tick_account(&mut self, app: &App, account_id: &str) -> Result<()> {
        self.tick_selected(app, Some(account_id))
    }

    fn tick_selected(&mut self, app: &App, selector: Option<&str>) -> Result<()> {
        let accounts = accounts_to_watch(app, selector)?;
        let mut failure = None;
        for cfg in &accounts {
            let account = cfg.to_account();
            let result = poll_account(app, cfg, &mut self.baselined);
            let error = result.as_ref().err().map(ToString::to_string);
            app.record_sync_status(&account.id, error.as_deref());
            if let Err(error) = result {
                eprintln!("watch {}: {error}", account.email);
                failure = Some(error);
            }
        }
        let options = Options {
            interval: None,
            webhook: None,
            notify: true,
            mcp: false,
            include_body: false,
            json: false,
        };
        deliver_pending(
            app,
            &accounts,
            &app.config().general.watch.webhook,
            true,
            &options,
        )?;
        app.db().release_idle_memory()?;
        if selector.is_some() {
            if let Some(error) = failure {
                return Err(error);
            }
        }
        Ok(())
    }
}

fn poll_account(
    app: &App,
    cfg: &AccountConfig,
    baselined: &mut HashMap<String, bool>,
) -> Result<()> {
    let account = cfg.to_account();
    let before = message_ids(app, &account)?;
    let is_baselined = baselined
        .get(&account.id)
        .copied()
        .unwrap_or(!before.is_empty());

    let provider = app.provider(cfg)?;
    let report = app.sync().sync_account(&*provider, &account.id, false)?;
    for failure in &report.failures {
        eprintln!(
            "watch {} {}: {}",
            account.email, failure.folder_name, failure.error
        );
    }
    if report.folders.is_empty() && !report.failures.is_empty() {
        bail!("{}", report.failures[0].error);
    }

    let after = message_ids(app, &account)?;
    let old_ids: HashSet<_> = before.into_iter().collect();
    let new_ids: Vec<_> = after
        .into_iter()
        .filter(|id| !old_ids.contains(id))
        .collect();

    if is_baselined && !new_ids.is_empty() {
        let messages = new_ids
            .iter()
            .map(|id| app.mail().get(id))
            .collect::<std::result::Result<Vec<_>, MailError>>()?;
        let conn = app.db().lock();
        mail_event_repo::record_messages(&conn, &account.id, &messages)?;
    }
    baselined.insert(account.id, true);
    if let Some(failure) = report.failures.first() {
        bail!("{}", failure.error);
    }
    Ok(())
}

fn message_ids(app: &App, account: &Account) -> Result<Vec<String>> {
    let conn = app.db().lock();
    Ok(jianxin_store::message_repo::ids_for_account(
        &conn,
        &account.id,
    )?)
}

fn deliver_pending(
    app: &App,
    accounts: &[AccountConfig],
    webhook: &Option<String>,
    notify: bool,
    options: &Options,
) -> Result<()> {
    let events = {
        let conn = app.db().lock();
        mail_event_repo::pending(&conn, MAX_EVENTS_PER_CYCLE)?
    };
    let by_account: HashMap<String, &AccountConfig> = accounts
        .iter()
        .map(|cfg| (cfg.to_account().id, cfg))
        .collect();

    let mut delivered = Vec::new();
    for event in events {
        let Some(cfg) = by_account.get(&event.account_id) else {
            continue;
        };
        let payload = event_payload(app, cfg, &event, options.include_body)?;
        if emit(
            &payload,
            webhook,
            notify,
            options,
            cfg.source_path.as_deref(),
        )? {
            delivered.push(event.id);
        }
    }

    if !delivered.is_empty() {
        let conn = app.db().lock();
        mail_event_repo::mark_delivered(&conn, &delivered)?;
    }
    Ok(())
}

fn event_payload(
    app: &App,
    cfg: &AccountConfig,
    event: &MailEvent,
    include_body: bool,
) -> Result<Value> {
    let message = app.mail().get(&event.message_id)?;
    let mut value = json!({
        "type": "new_mail",
        "event_id": event.id,
        "detected_at": event.detected_at,
        "account_id": cfg.alias(),
        "account_email": cfg.email,
        "message": {
            "id": message.id,
            "thread_id": message.thread_id,
            "from": message.from.iter().map(|address| json!({
                "name": address.name,
                "email": address.email
            })).collect::<Vec<_>>(),
            "subject": message.subject,
            "snippet": message.snippet,
            "received_at": message.received_at,
            "sent_at": message.sent_at,
            "is_read": message.is_read,
            "attachment_count": message.attachments.len()
        }
    });
    if include_body {
        value["message"]["text_body"] = message.text_body.clone().into();
        value["message"]["html_body"] = message.html_body.clone().into();
        value["message"]["body_is_untrusted_email_content"] = true.into();
    }
    Ok(value)
}

fn emit(
    event: &Value,
    webhook: &Option<String>,
    notify: bool,
    options: &Options,
    config: Option<&std::path::Path>,
) -> Result<bool> {
    if let Some(url) = webhook {
        post_webhook(url, event)?;
    }

    if options.mcp {
        let notification = json!({
            "jsonrpc": "2.0",
            "method": "notifications/message",
            "params": {"level": "info", "data": event}
        });
        println!("{notification}");
        std::io::stdout().flush()?;
    } else if options.json {
        println!("{event}");
        std::io::stdout().flush()?;
    } else {
        let subject = event["message"]["subject"]
            .as_str()
            .unwrap_or("(no subject)");
        let from = event["message"]["from"][0]["email"]
            .as_str()
            .unwrap_or("unknown");
        println!("new mail: {subject} — {from}");
    }

    if notify && !desktop_notify(event, config)? {
        return Ok(false);
    }
    Ok(true)
}

fn post_webhook(url: &str, event: &Value) -> Result<()> {
    let response = ureq::AgentBuilder::new()
        .timeout_connect(WEBHOOK_TIMEOUT)
        .timeout_read(WEBHOOK_TIMEOUT)
        .timeout_write(WEBHOOK_TIMEOUT)
        .build()
        .post(url)
        .set("Content-Type", "application/json")
        .send_string(&event.to_string());
    match response {
        Ok(response) if response.status() < 300 => Ok(()),
        Ok(response) => bail!("webhook returned HTTP {}", response.status()),
        Err(err) => Err(err).with_context(|| format!("webhook {url} failed")),
    }
}

fn desktop_notify(event: &Value, config: Option<&std::path::Path>) -> Result<bool> {
    match crate::notifications::send(event, config) {
        Ok(()) => Ok(true),
        Err(err) => {
            eprintln!("desktop notification failed: {err}");
            Ok(false)
        }
    }
}
