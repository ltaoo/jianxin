//! Timeless/jxui app shell and read-only, bounded JSON projections.
use super::*;
use jianxin_store::reader_repo::{self, Filter};

const SCRIPT: &str = include_str!("../../../frontend/dist/app.js");
const CSS: &str = include_str!("../../../frontend/dist/app.css");

fn asset(body: &str, content_type: &'static str) -> Reply {
    Reply {
        status: 200,
        body: body.into(),
        content_type,
        csp: "default-src 'none'; frame-ancestors 'none'".into(),
    }
}

fn identifier(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
}

pub(super) fn render(app: &App, root: &str, route: &str) -> Result<Reply> {
    let parsed = ammonia::url::Url::parse(&format!("http://127.0.0.1/{route}"))?;
    let path = parsed.path().trim_start_matches('/');
    if path == "assets/app.js" {
        return Ok(asset(SCRIPT, "text/javascript; charset=utf-8"));
    }
    if path == "assets/app.css" {
        return Ok(asset(CSS, "text/css; charset=utf-8"));
    }
    if path == "api/mailboxes" {
        let mut accounts = Vec::new();
        for cfg in app.account_configs() {
            let account = cfg.to_account();
            let folders = app.mail().folders(&account.id)?;
            let status = jianxin_core::read_account_sync_status(
                app.config(),
                &account.id,
                !folders.is_empty(),
            );
            accounts.push(serde_json::json!({
                "id":account.id, "email":account.email, "folders":folders,
                "status":{"state":status,"label":status.label(),"tone":status.tone()}
            }));
        }
        let allowed_accounts = app
            .account_configs()
            .iter()
            .map(|cfg| cfg.to_account().id)
            .collect::<Vec<_>>();
        let counts = reader_repo::mail_counts(&app.db().lock(), &allowed_accounts)?;
        return Ok(Reply::json(
            200,
            serde_json::json!({"accounts":accounts,"counts":counts}),
        ));
    }
    let query: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
    let page = query
        .get("page")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0)
        .min(100_000);
    if path == "api/threads" {
        let account = query.get("account").map(String::as_str).unwrap_or("");
        let folder = query.get("folder").map(String::as_str).unwrap_or("");
        if !account.is_empty() && !known_account(app, account) {
            bail!("unknown account");
        }
        if !folder.is_empty() {
            let accounts = app
                .account_configs()
                .iter()
                .map(|c| c.to_account().id)
                .collect::<Vec<_>>();
            let mut valid = false;
            for id in accounts {
                if (account.is_empty() || account == id)
                    && app.mail().folders(&id)?.iter().any(|f| f.id == folder)
                {
                    valid = true;
                    break;
                }
            }
            if !valid {
                bail!("unknown folder");
            }
        }
        let search = query.get("q").map(String::as_str).unwrap_or("");
        if search.len() > 512 {
            bail!("query too long");
        }
        let allowed_accounts = app
            .account_configs()
            .iter()
            .map(|c| c.to_account().id)
            .collect::<Vec<_>>();
        let mut threads = reader_repo::conversations(
            &app.db().lock(),
            &Filter {
                account_id: account,
                allowed_accounts: &allowed_accounts,
                folder_id: folder,
                unread: query.get("unread").is_some_and(|v| v == "1"),
                query: search,
                limit: PAGE_SIZE + 1,
                offset: page * PAGE_SIZE,
            },
        )?;
        threads.retain(|t| known_account(app, &t.account_id));
        let more = threads.len() > PAGE_SIZE as usize;
        threads.truncate(PAGE_SIZE as usize);
        let threads = threads
            .iter()
            .map(|t| {
                let mut value = serde_json::to_value(t).unwrap();
                value["account_email"] = account_email(app, &t.account_id).into();
                value
            })
            .collect::<Vec<_>>();
        return Ok(Reply::json(
            200,
            serde_json::json!({"threads":threads,"page":page,"more":more}),
        ));
    }
    if let Some(id) = path.strip_prefix("api/thread/") {
        if !identifier(id) {
            bail!("invalid thread");
        }
        let thread =
            jianxin_store::thread_repo::get(&app.db().lock(), id)?.context("unknown thread")?;
        if !known_account(app, &thread.account_id) {
            bail!("unknown account");
        }
        let mut messages =
            reader_repo::headers(&app.db().lock(), id, "", PAGE_SIZE + 1, page * PAGE_SIZE)?;
        let more = messages.len() > PAGE_SIZE as usize;
        messages.truncate(PAGE_SIZE as usize);
        return Ok(Reply::json(
            200,
            serde_json::json!({"thread":thread,"messages":messages,"more":more,"page":page}),
        ));
    }
    if let Some(id) = path.strip_prefix("api/message/") {
        if !identifier(id) {
            bail!("invalid message");
        }
        let messages = reader_repo::headers(&app.db().lock(), "", id, 1, 0)?;
        let message = messages.first().context("unknown message")?;
        if !known_account(app, &message.account_id) {
            bail!("unknown account");
        }
        return Ok(Reply::json(
            200,
            serde_json::json!({"subject":message.subject,"messages":messages,"more":false,"page":0}),
        ));
    }
    if path.starts_with("body/") {
        return render_legacy_route(app, root, route);
    }
    if path.is_empty()
        || path == "components"
        || ["message/", "thread/"]
            .iter()
            .any(|prefix| path.strip_prefix(prefix).is_some_and(identifier))
    {
        return Ok(Reply {
            status:200, content_type:"text/html; charset=utf-8",
            csp: "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; frame-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'".into(),
            body:format!("<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>简信 · 邮件</title><link rel=\"stylesheet\" href=\"{root}/assets/app.css\"><script defer src=\"{root}/assets/app.js\"></script></head><body><a class=\"jx-skip\" href=\"#mail-content\">跳到邮件内容</a><div id=\"app\" data-root=\"{root}\"><p class=\"jx-startup\">正在加载本地邮件…</p></div><noscript>请启用 JavaScript 以使用邮件阅读器。</noscript></body></html>")
        });
    }
    bail!("unknown route")
}

fn known_account(app: &App, id: &str) -> bool {
    app.account_configs()
        .iter()
        .any(|c| c.to_account().id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shell_loads_local_timeless_assets_and_keeps_body_isolated() {
        let (app, message) = super::super::tests::fixture();
        for route in [
            "".to_string(),
            format!("message/{}", message.id),
            format!("thread/{}", message.thread_id.unwrap()),
        ] {
            let shell = render(&app, "/secret", &route).unwrap();
            assert!(shell.body.contains("/secret/assets/app.js"));
            assert!(!shell.body.contains("HTML body"));
            assert!(shell.csp.contains("frame-src 'self'"));
        }
        let script = render(&app, "/secret", "assets/app.js").unwrap();
        assert!(script.body.contains("Timeless"));
        assert_eq!(script.content_type, "text/javascript; charset=utf-8");
        assert!(render(&app, "/secret", "assets/../../config.json").is_err());
    }
    #[test]
    fn mailbox_api_reads_shared_status_without_exposing_raw_errors_or_tui_shortcuts() {
        let directory =
            std::env::temp_dir().join(format!("jianxin-web-status-{}", jianxin_domain::new_id()));
        let config: jianxin_config::AppConfig = serde_json::from_value(serde_json::json!({
            "general": { "data_dir": directory },
            "accounts": [{ "email": "status@example.com", "auth": { "type": "password", "password": "private-password" } }]
        })).unwrap();
        let app = App::with_database(
            config.clone(),
            jianxin_store::Database::open_in_memory().unwrap(),
        )
        .unwrap();
        let id = app.account_configs()[0].to_account().id;
        for (error, expected) in [
            (None, "synced"),
            (
                Some("AUTHENTICATION_FAILED private-password"),
                "login_failed",
            ),
            (Some("network private-password"), "sync_failed"),
        ] {
            jianxin_core::record_account_sync_status(&config, &id, error);
            let response = render(&app, "/secret", "api/mailboxes").unwrap();
            let value: serde_json::Value = serde_json::from_str(&response.body).unwrap();
            assert_eq!(value["accounts"][0]["status"]["state"], expected);
            assert!(!response.body.contains("private-password"));
            assert!(!response.body.contains("按 f"));
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn api_filters_are_bounded_and_never_return_credentials_or_bodies() {
        let (app, message) = super::super::tests::fixture();
        let result = render(&app, "/secret", "api/mailboxes").unwrap();
        assert!(!result.body.contains("password"));
        let result = render(&app, "/secret", "api/threads?q=subject").unwrap();
        let value: serde_json::Value = serde_json::from_str(&result.body).unwrap();
        assert_eq!(value["threads"].as_array().unwrap().len(), 1);
        assert!(!result.body.contains("HTML body"));
        let result = render(&app, "/secret", "api/threads?q=nonexistent").unwrap();
        assert!(result.body.contains("\"threads\":[]"));
        assert!(render(&app, "/secret", "api/threads?account=unknown").is_err());
        assert!(render(&app, "/secret", "api/threads?folder=unknown").is_err());
        let result = render(
            &app,
            "/secret",
            &format!("api/thread/{}", message.thread_id.unwrap()),
        )
        .unwrap();
        assert!(!result.body.contains("html_body"));
        assert!(!result.body.contains("HTML body"));
        assert!(result.body.contains(&message.id));
        let body = render(&app, "/secret", &format!("body/{}?images=0", message.id)).unwrap();
        assert!(body.body.contains("HTML body"));
        assert!(!body.csp.contains("allow-scripts"));
    }
}
