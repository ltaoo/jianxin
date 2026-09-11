//! Compose, draft and send.
//!
//! The pipeline is `Compose -> Draft -> Outbox -> Provider -> Sent` (§29).
//! Sending is therefore two steps: `queue()` records the intent durably,
//! `flush()` delivers it. A crash or a dead network between the two leaves a
//! job the user can retry instead of a lost mail.

use eeeemail_config::AccountConfig;
use eeeemail_domain::{
    Address, Capability, ComposeMessage, Folder, FolderKind, MailError, MailErrorCode, Message,
    Result, SendResult,
};
use eeeemail_store as store;

use crate::App;

/// What happened while draining the outbox.
#[derive(Debug, Default)]
pub struct FlushReport {
    pub sent: usize,
    pub failed: usize,
    pub errors: Vec<(String, MailError)>,
}

pub struct ComposeService<'a> {
    app: &'a App,
}

impl<'a> ComposeService<'a> {
    pub fn new(app: &'a App) -> Self {
        Self { app }
    }

    /// Save a draft locally.
    ///
    /// The remote copy is best-effort: providers that cannot store drafts
    /// (or that are offline right now) still get a usable local draft.
    pub fn draft(&self, cfg: &AccountConfig, compose: &ComposeMessage) -> Result<Message> {
        let account = cfg.to_account();
        let folder = self.drafts_folder(cfg)?;
        let mut message = message_from_compose(&account.id, &folder, compose, true);

        let provider = self.app.provider(cfg)?;
        match provider.create_draft(&folder, compose) {
            Ok(remote) => {
                // Keep the provider's id so a later sync recognises the draft
                // instead of inserting a twin.
                message.remote_id = remote.remote_id;
                message.id = Message::deterministic_id(&account.id, &folder.id, &message.remote_id);
            }
            Err(e) if e.code == MailErrorCode::UnsupportedFeature => {
                tracing::debug!("provider cannot store drafts; keeping a local-only draft");
            }
            Err(e) => tracing::warn!(error = %e, "could not store draft on the provider"),
        }

        let conn = self.app.db().lock();
        store::message_repo::upsert_many(&conn, std::slice::from_ref(&message))?;
        Ok(message)
    }

    /// Queue a message for delivery without touching the network.
    pub fn queue(&self, cfg: &AccountConfig, compose: &ComposeMessage) -> Result<store::OutboxJob> {
        if !compose.has_recipients() {
            return Err(MailError::new(
                MailErrorCode::InvalidConfig,
                "no recipients — add at least one To/Cc/Bcc address",
            ));
        }

        let account = cfg.to_account();
        let folder = self.sent_folder(cfg)?;
        let message = message_from_compose(&account.id, &folder, compose, false);

        self.app.db().transaction(|conn| {
            store::message_repo::upsert_many(conn, std::slice::from_ref(&message))?;
            store::outbox_repo::enqueue(conn, &account.id, &message.id, compose)
        })
    }

    /// Queue *and* deliver, returning the provider's receipt.
    pub fn send(&self, cfg: &AccountConfig, compose: &ComposeMessage) -> Result<SendResult> {
        let job = self.queue(cfg, compose)?;
        self.deliver(cfg, &job)
    }

    /// Deliver everything queued for one account.
    pub fn flush(&self, cfg: &AccountConfig) -> Result<FlushReport> {
        let account = cfg.to_account();
        let pending = {
            let conn = self.app.db().lock();
            store::outbox_repo::list_pending(&conn)?
                .into_iter()
                .filter(|j| j.account_id == account.id)
                .collect::<Vec<_>>()
        };

        let mut report = FlushReport::default();
        for job in pending {
            match self.deliver(cfg, &job) {
                Ok(_) => report.sent += 1,
                Err(e) => {
                    report.failed += 1;
                    report.errors.push((job.id.clone(), e));
                }
            }
        }
        Ok(report)
    }

    /// Deliver one job: load, claim, send, reconcile.
    fn deliver(&self, cfg: &AccountConfig, job: &store::OutboxJob) -> Result<SendResult> {
        let account = cfg.to_account();
        let provider = self.app.provider(cfg)?;
        provider.require(Capability::Send)?;

        // Network work happens outside any transaction: a slow SMTP server must
        // not hold the database lock.
        let compose = store::outbox_repo::load_payload(&self.app.db().lock(), &job.id)?;

        if !store::outbox_repo::claim(&self.app.db().lock(), &job.id)? {
            return Err(MailError::new(
                MailErrorCode::SendFailed,
                "job was claimed by another worker",
            ));
        }

        match provider.send_message(&compose) {
            Ok(result) => {
                // One lock, reused: `Database::lock` is a std Mutex and is not
                // reentrant, so a second `lock()` here would deadlock.
                let conn = self.app.db().lock();
                store::outbox_repo::mark_sent(&conn, &job.id)?;
                store::message_repo::set_read(&conn, &job.message_id, true)?;
                store::message_repo::set_deleted(&conn, &job.message_id, false)?;
                if let Some(sent) =
                    store::folder_repo::find_by_kind(&conn, &account.id, FolderKind::Sent)?
                {
                    let remote = result
                        .remote_id
                        .clone()
                        .unwrap_or_else(|| format!("sent-{}", job.message_id));
                    store::message_repo::move_to_folder(
                        &conn,
                        &job.message_id,
                        &sent.id,
                        Some(&remote),
                    )?;
                }
                Ok(result)
            }
            Err(e) => {
                let conn = self.app.db().lock();
                if e.code.is_retriable() && job.retry_count < MAX_RETRIES {
                    store::outbox_repo::retry(&conn, &job.id)?;
                } else {
                    store::outbox_repo::mark_failed(&conn, &job.id, &e.to_string())?;
                }
                Err(e)
            }
        }
    }

    /// Requeue every failed job for one account.
    pub fn retry_failed(&self, cfg: &AccountConfig) -> Result<usize> {
        let account = cfg.to_account();
        let conn = self.app.db().lock();
        let mut n = 0;
        for job in store::outbox_repo::list_by_status(&conn, store::OutboxStatus::Failed)? {
            if job.account_id == account.id && store::outbox_repo::retry(&conn, &job.id)? {
                n += 1;
            }
        }
        Ok(n)
    }

    pub fn pending(&self, cfg: &AccountConfig) -> Result<Vec<store::OutboxJob>> {
        let account = cfg.to_account();
        let conn = self.app.db().lock();
        Ok(store::outbox_repo::list_for_account(&conn, &account.id)?
            .into_iter()
            .filter(|j| {
                j.status == store::OutboxStatus::Pending || j.status == store::OutboxStatus::Failed
            })
            .collect())
    }

    /// Drafts folder from the database, or a synthetic one when the account has
    /// never been synced.
    fn drafts_folder(&self, cfg: &AccountConfig) -> Result<Folder> {
        self.folder_of_kind(cfg, FolderKind::Drafts, "Drafts")
    }

    fn sent_folder(&self, cfg: &AccountConfig) -> Result<Folder> {
        self.folder_of_kind(cfg, FolderKind::Sent, "Sent")
    }

    fn folder_of_kind(
        &self,
        cfg: &AccountConfig,
        kind: FolderKind,
        fallback_remote: &str,
    ) -> Result<Folder> {
        let account = cfg.to_account();
        let conn = self.app.db().lock();
        if let Some(f) = store::folder_repo::find_by_kind(&conn, &account.id, kind)? {
            return Ok(f);
        }
        // Not synced yet — create a placeholder row so the local copy still has
        // a home. The next sync replaces it with the real one.
        let folder = Folder::new(&account.id, fallback_remote, fallback_remote, kind);
        store::folder_repo::upsert_many(&conn, std::slice::from_ref(&folder))?;
        Ok(folder)
    }
}

/// Give up rather than hammering a permanently failing mailbox (§30).
const MAX_RETRIES: u32 = 5;

/// Map a `ComposeMessage` onto the unified `Message` model.
fn message_from_compose(
    account_id: &str,
    folder: &Folder,
    compose: &ComposeMessage,
    is_draft: bool,
) -> Message {
    let mut m = Message::new();
    m.account_id = account_id.to_string();
    m.folder_id = folder.id.clone();
    // A local placeholder until the provider assigns a real id.
    m.remote_id = format!("local-{}", m.id);
    m.subject = compose.subject.clone();
    m.from = compose.from.clone().into_iter().collect();
    m.to = compose.to.clone();
    m.cc = compose.cc.clone();
    m.bcc = compose.bcc.clone();
    m.text_body = compose.text_body.clone();
    m.html_body = compose.html_body.clone();
    m.snippet = compose
        .text_body
        .as_deref()
        .map(|t| eeeemail_domain::make_snippet(t, 200));
    m.is_draft = is_draft;
    m.is_read = true;
    m.sent_at = Some(eeeemail_domain::now_ts());
    m.in_reply_to = compose.in_reply_to.clone();
    m.references = compose.references.clone();
    m.attachments = compose
        .attachments
        .iter()
        .map(|a| {
            eeeemail_domain::Attachment::new(
                &m.id,
                Some(a.filename.clone()),
                &a.mime_type,
                a.data.len() as u64,
            )
        })
        .collect();

    if m.from.is_empty() {
        m.from.push(Address::new("unknown@invalid"));
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use eeeemail_domain::{ComposeAttachment, MessagePage, ProviderCapabilities};
    use eeeemail_provider::{ListMessagesOptions, MailProvider, SearchQuery};

    /// Captures sent messages instead of delivering them, and can be told to
    /// fail so the retry path is testable.
    struct FakeSender {
        account_id: String,
        folders: Vec<Folder>,
        fail: bool,
        sent: std::sync::Mutex<Vec<ComposeMessage>>,
    }

    impl MailProvider for FakeSender {
        fn id(&self) -> eeeemail_domain::ProviderId {
            eeeemail_domain::ProviderId::GenericImap
        }
        fn account_id(&self) -> &str {
            &self.account_id
        }
        fn account_email(&self) -> &str {
            "a@example.com"
        }
        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities::imap()
        }
        fn list_folders(&self) -> Result<Vec<Folder>> {
            Ok(self.folders.clone())
        }
        fn list_messages(&self, _f: &Folder, _o: ListMessagesOptions) -> Result<MessagePage> {
            Ok(MessagePage::new(Vec::new()))
        }
        fn get_message(&self, _f: &Folder, _r: &str) -> Result<Message> {
            Err(MailError::new(MailErrorCode::MessageNotFound, "none"))
        }
        fn send_message(&self, m: &ComposeMessage) -> Result<SendResult> {
            if self.fail {
                return Err(MailError::new(MailErrorCode::SendFailed, "smtp refused"));
            }
            self.sent.lock().unwrap().push(m.clone());
            Ok(SendResult {
                remote_id: Some("77".to_string()),
                message_id_header: Some("<x@y>".to_string()),
                sent_at: 0,
            })
        }
        fn search(&self, _q: &SearchQuery) -> Result<MessagePage> {
            Ok(MessagePage::new(Vec::new()))
        }
    }

    fn compose() -> ComposeMessage {
        ComposeMessage {
            from: Some(Address::new("a@example.com")),
            to: vec![Address::new("bob@example.com")],
            cc: vec![],
            bcc: vec![],
            subject: "Hello".to_string(),
            text_body: Some("long body text".to_string()),
            html_body: None,
            attachments: vec![ComposeAttachment::new(
                "a.txt",
                "text/plain",
                b"hi".to_vec(),
            )],
            in_reply_to: None,
            references: vec![],
        }
    }

    /// Build an app whose provider is `provider`, by caching it first.
    fn app_with(provider: FakeSender) -> (App, AccountConfig, String) {
        let json = serde_json::json!({
            "accounts": [{
                "email": "a@example.com",
                "auth": {"type": "password", "password": "x"}
            }]
        });
        let cfg: eeeemail_config::AppConfig = serde_json::from_value(json).unwrap();
        let app =
            App::with_database(cfg, eeeemail_store::Database::open_in_memory().unwrap()).unwrap();
        let account_cfg = app.resolve_account(None).unwrap();
        let account_id = account_cfg.to_account().id;

        // Seed the Sent folder so reconciliation has somewhere to put the mail.
        {
            let conn = app.db().lock();
            let sent = Folder::new(&account_id, "Sent", "Sent", FolderKind::Sent);
            store::folder_repo::upsert_many(&conn, &[sent]).unwrap();
        }

        // Prime the provider cache with our fake.
        {
            use std::sync::Arc;
            let boxed: Box<dyn MailProvider> = Box::new(provider);
            let arc: Arc<dyn MailProvider> = Arc::from(boxed);
            app.providers
                .write()
                .unwrap()
                .insert(account_id.clone(), arc);
        }

        (app, account_cfg, account_id)
    }

    #[test]
    fn queue_survives_without_a_network() {
        let (app, cfg, _) = app_with(FakeSender {
            account_id: String::new(),
            folders: vec![],
            fail: true,
            sent: Default::default(),
        });

        // `queue` must not call the provider at all.
        let job = app.compose().queue(&cfg, &compose()).unwrap();
        assert_eq!(job.status, store::OutboxStatus::Pending);
        assert_eq!(app.compose().pending(&cfg).unwrap().len(), 1);
    }

    #[test]
    fn flush_delivers_and_reconciles() {
        let (app, cfg, account_id) = app_with(FakeSender {
            account_id: String::new(),
            folders: vec![],
            fail: false,
            sent: Default::default(),
        });

        let job = app.compose().queue(&cfg, &compose()).unwrap();
        let report = app.compose().flush(&cfg).unwrap();
        assert_eq!(report.sent, 1);
        assert_eq!(report.failed, 0);

        assert_eq!(
            app.db()
                .lock()
                .query_row("SELECT status FROM outbox WHERE id = ?1", [&job.id], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap(),
            "sent"
        );

        // The local row moved into Sent and is no longer a draft.
        let conn = app.db().lock();
        let m = store::message_repo::get(&conn, &job.message_id)
            .unwrap()
            .unwrap();
        assert!(!m.is_draft);
        assert!(m.is_read);
        let sent = store::folder_repo::find_by_kind(&conn, &account_id, FolderKind::Sent)
            .unwrap()
            .unwrap();
        assert_eq!(m.folder_id, sent.id);
    }

    #[test]
    fn attachment_bytes_survive_the_queue() {
        let (app, cfg, _) = app_with(FakeSender {
            account_id: String::new(),
            folders: vec![],
            fail: false,
            sent: Default::default(),
        });
        let job = app.compose().queue(&cfg, &compose()).unwrap();
        let loaded = store::outbox_repo::load_payload(&app.db().lock(), &job.id).unwrap();
        assert_eq!(loaded.attachments[0].data, b"hi".to_vec());
    }

    #[test]
    fn permanent_failure_is_not_retried_forever() {
        let (app, cfg, _) = app_with(FakeSender {
            account_id: String::new(),
            folders: vec![],
            fail: true,
            sent: Default::default(),
        });
        app.compose().queue(&cfg, &compose()).unwrap();
        let report = app.compose().flush(&cfg).unwrap();
        assert_eq!(report.failed, 1);

        // SendFailed is not retriable, so the job stays failed rather than
        // bouncing back to pending.
        let jobs = app.compose().pending(&cfg).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].status, store::OutboxStatus::Failed);
    }

    #[test]
    fn missing_recipients_is_rejected_before_queueing() {
        let (app, cfg, _) = app_with(FakeSender {
            account_id: String::new(),
            folders: vec![],
            fail: false,
            sent: Default::default(),
        });
        let mut bad = compose();
        bad.to.clear();
        let err = app.compose().queue(&cfg, &bad).unwrap_err();
        assert_eq!(err.code, MailErrorCode::InvalidConfig);
    }

    #[test]
    fn retry_failed_requeues() {
        let (app, cfg, _) = app_with(FakeSender {
            account_id: String::new(),
            folders: vec![],
            fail: true,
            sent: Default::default(),
        });
        app.compose().queue(&cfg, &compose()).unwrap();
        app.compose().flush(&cfg).unwrap();
        assert_eq!(app.compose().retry_failed(&cfg).unwrap(), 1);
        assert_eq!(
            app.compose().pending(&cfg).unwrap()[0].status,
            store::OutboxStatus::Pending
        );
    }
}
