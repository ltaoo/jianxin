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
    if path.starts_with("api/reply/") {
        return web_reply::prepare(app, path);
    }
    if path == "assets/app.js" {
        return Ok(asset(SCRIPT, "text/javascript; charset=utf-8"));
    }
    if path == "assets/app.css" {
        return Ok(asset(CSS, "text/css; charset=utf-8"));
    }
    if let Some(name) = path.strip_prefix("assets/mail-providers/") {
        let icon = match name {
            "gmail.svg" => include_str!("../../../assets/mail-providers/svg-exact/gmail.svg"),
            "outlook.svg" => include_str!("../../../assets/mail-providers/svg-exact/outlook.svg"),
            "qq.svg" => include_str!("../../../assets/mail-providers/svg-exact/qq.svg"),
            "foxmail.svg" => include_str!("../../../assets/mail-providers/svg-exact/foxmail.svg"),
            "netease163.svg" => include_str!("../../../assets/mail-providers/svg-exact/netease163.svg"),
            "netease126.svg" => include_str!("../../../assets/mail-providers/svg-exact/netease126.svg"),
            "yeah.svg" => include_str!("../../../assets/mail-providers/svg-exact/yeah.svg"),
            "aliyun.svg" => include_str!("../../../assets/mail-providers/svg-exact/aliyun.svg"),
            "sina.svg" => include_str!("../../../assets/mail-providers/svg-exact/sina.svg"),
            "mail139.svg" => include_str!("../../../assets/mail-providers/svg-exact/mail139.svg"),
            "icloud.svg" => include_str!("../../../assets/mail-providers/svg-exact/icloud.svg"),
            "zoho.svg" => include_str!("../../../assets/mail-providers/svg-exact/zoho.svg"),
            "yahoo.svg" => include_str!("../../../assets/mail-providers/svg-exact/yahoo.svg"),
            "fastmail.svg" => include_str!("../../../assets/mail-providers/svg-exact/fastmail.svg"),
            "proton.svg" => include_str!("../../../assets/mail-providers/svg-exact/proton.svg"),
            "tencent.svg" => include_str!("../../../assets/mail-providers/svg-exact/tencent.svg"),
            _ => bail!("unknown mailbox icon"),
        };
        return Ok(asset(icon, "image/svg+xml; charset=utf-8"));
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
            let oauth_config = match &cfg.auth {
                jianxin_config::AuthConfig::OAuth2 { client_id, token_endpoint, .. } => Some(serde_json::json!({
                    "client_id":client_id.as_deref().unwrap_or(""), "token_endpoint":token_endpoint.as_deref().unwrap_or(""),
                    "needs_setup":client_id.as_ref().is_none_or(|id| id.trim().is_empty())
                        || token_endpoint.as_ref().is_none_or(|endpoint| endpoint.trim().is_empty()),
                })),
                _ => None,
            };
            accounts.push(serde_json::json!({
                "id":account.id, "email":account.email, "folders":folders,
                "platform":cfg.platform, "provider":account.provider,
                "oauth_config":oauth_config,
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
    if path == "api/contacts" {
        let search = query.get("q").map(String::as_str).unwrap_or("");
        if search.len() > 256 { bail!("contact query too long"); }
        let allowed = app.account_configs().iter().map(|cfg| cfg.to_account().id).collect::<Vec<_>>();
        let contacts = reader_repo::contacts(&app.db().lock(), &allowed, search)?;
        return Ok(Reply::json(200, serde_json::json!({"contacts":contacts})));
    }
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
                starred: query.get("starred").is_some_and(|v| v == "1"),
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
        let mut thread =
            jianxin_store::thread_repo::get(&app.db().lock(), id)?.context("unknown thread")?;
        if !known_account(app, &thread.account_id) {
            bail!("unknown account");
        }
        let folder_id = query.get("folder").map(String::as_str).unwrap_or("");
        if !folder_id.is_empty() && !app.mail().folders(&thread.account_id)?.iter().any(|folder| folder.id == folder_id) {
            bail!("unknown folder");
        }
        let mut messages = reader_repo::headers_in_folder(&app.db().lock(), id, "", folder_id, PAGE_SIZE + 1, page * PAGE_SIZE)?;
        reader_repo::scope_thread(&app.db().lock(), &mut thread, folder_id)?;
        let more = messages.len() > PAGE_SIZE as usize;
        messages.truncate(PAGE_SIZE as usize);
        return Ok(Reply::json(
            200,
            serde_json::json!({"thread":thread,"messages":messages,"more":more,"page":page}),
        ));
    }
    if let Some(id) = path.strip_prefix("api/message/").and_then(|value| value.strip_suffix("/metadata")) {
        if !identifier(id) { bail!("invalid message"); }
        let message = app.mail().get(id)?;
        let cfg = app.account_configs().iter().find(|cfg| cfg.to_account().id == message.account_id).context("unknown account")?;
        let folder = app.mail().folders(&message.account_id)?.into_iter().find(|folder| folder.id == message.folder_id);
        return Ok(Reply::json(200, serde_json::json!({
            "id":message.id,"thread_id":message.thread_id,"remote_id":message.remote_id,
            "message_id_header":message.message_id_header,"immutable_id":message.immutable_id,
            "account_id":message.account_id,"account_email":cfg.email,"provider":cfg.to_account().provider,
            "folder_id":message.folder_id,"folder_name":folder.map(|folder|folder.name),
            "in_reply_to":message.in_reply_to,"references":message.references,
            "server_thread_key":message.server_thread_key,"thread_index_root":message.thread_index_root,
            "from":message.from,"to":message.to,"cc":message.cc,"reply_to":message.reply_to,"sent_at":message.sent_at,"received_at":message.received_at,"size":message.size
        })));
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
            csp: "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self'; connect-src 'self'; frame-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'".into(),
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
    fn folder_thread_reading_excludes_sent_replies_but_full_conversations_keep_them() {
        let (app, root) = super::super::tests::fixture();
        let sent = jianxin_domain::Folder::new(&root.account_id, "Sent", "Sent", jianxin_domain::FolderKind::Sent);
        let mut reply = root.clone();
        reply.id = "outgoing-reply".into(); reply.remote_id = "2".into(); reply.folder_id = sent.id.clone();
        reply.message_id_header = Some("<reply@example.com>".into()); reply.is_read = true;
        reply.from = vec![jianxin_domain::Address::new("test@example.com")];
        jianxin_store::folder_repo::upsert_many(&app.db().lock(), &[sent.clone()]).unwrap();
        jianxin_store::message_repo::upsert_many(&app.db().lock(), &[reply.clone()]).unwrap();
        let thread_id = root.thread_id.as_ref().unwrap();
        for (folder, expected_id) in [(&root.folder_id, &root.id), (&sent.id, &reply.id)] {
            let response = render(&app, "", &format!("api/thread/{thread_id}?folder={folder}")).unwrap();
            let value: serde_json::Value = serde_json::from_str(&response.body).unwrap();
            assert_eq!(value["messages"].as_array().unwrap().len(), 1);
            assert_eq!(value["messages"][0]["id"], *expected_id);
            assert_eq!(value["thread"]["message_count"], 1);
        }
        let full = render(&app, "", &format!("api/thread/{thread_id}")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&full.body).unwrap();
        assert_eq!(value["messages"].as_array().unwrap().len(), 2);
        assert_eq!(value["thread"]["message_count"], 2);
        assert!(render(&app, "", &format!("api/thread/{thread_id}?folder=missing")).is_err());
    }
    #[test]
    fn metadata_exposes_identity_and_sender_without_bodies_credentials_or_writes() {
        let (app, mut message) = super::super::tests::fixture();
        message.from = vec![jianxin_domain::Address::with_name("Sender Name", "sender@example.com")];
        message.reply_to = vec![jianxin_domain::Address::new("reply@example.com")];
        message.to = vec![jianxin_domain::Address::new("test@example.com")];
        message.cc = vec![jianxin_domain::Address::new("copy@example.com")];
        message.message_id_header = Some("<original@example.com>".into());
        message.references = vec!["<parent@example.com>".into()];
        message.provider_data = Some(serde_json::json!({"secret":"private-provider-data"}));
        jianxin_store::message_repo::upsert_many(&app.db().lock(), &[message.clone()]).unwrap();
        let response = render(&app, "", &format!("api/message/{}/metadata", message.id)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&response.body).unwrap();
        assert_eq!(value["id"], message.id);
        assert_eq!(value["thread_id"], message.thread_id.unwrap());
        assert_eq!(value["remote_id"], "1");
        assert_eq!(value["message_id_header"], "<original@example.com>");
        assert_eq!(value["folder_name"], "Inbox");
        assert_eq!(value["account_email"], "test@example.com");
        assert_eq!(value["from"][0]["email"], "sender@example.com");
        assert_eq!(value["from"][0]["name"], "Sender Name");
        assert_eq!(value["reply_to"][0]["email"], "reply@example.com");
        assert_eq!(value["to"][0]["email"], "test@example.com");
        assert_eq!(value["cc"][0]["email"], "copy@example.com");
        for forbidden in ["HTML body", "html_body", "text_body", "password", "private-provider-data"] {
            assert!(!response.body.contains(forbidden));
        }
        assert!(!app.mail().get(&message.id).unwrap().is_read);
        assert!(render(&app, "", "api/message/missing/metadata").is_err());
        assert!(render(&app, "", "api/message/invalid!id/metadata").is_err());
    }
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
            assert!(shell.csp.contains("img-src 'self'"));
        }
        let script = render(&app, "/secret", "assets/app.js").unwrap();
        assert!(script.body.contains("Timeless"));
        assert_eq!(script.content_type, "text/javascript; charset=utf-8");
        assert!(render(&app, "/secret", "assets/../../config.json").is_err());
        for platform in ["gmail", "outlook", "qq", "foxmail", "netease163", "netease126", "yeah", "aliyun", "sina", "mail139", "icloud", "zoho", "yahoo", "fastmail", "proton", "tencent"] {
            let icon = render(&app, "/secret", &format!("assets/mail-providers/{platform}.svg")).unwrap();
            assert_eq!(icon.content_type, "image/svg+xml; charset=utf-8");
            assert!(icon.body.starts_with("<svg"));
            assert!(!icon.body.contains("<image"));
        }
        assert!(render(&app, "/secret", "assets/mail-providers/unknown.svg").is_err());
        assert!(render(&app, "/secret", "assets/mail-providers/../../../config.json").is_err());
    }
    #[test]
    fn mailbox_api_reads_shared_status_without_exposing_raw_errors_or_tui_shortcuts() {
        let directory =
            std::env::temp_dir().join(format!("jianxin-web-status-{}", jianxin_domain::new_id()));
        let config: jianxin_config::AppConfig = serde_json::from_value(serde_json::json!({
            "general": { "data_dir": directory },
            "accounts": [{ "email": "status@example.com", "platform": "qq", "auth": { "type": "password", "password": "private-password" } }]
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
            assert_eq!(value["accounts"][0]["platform"], "qq");
            assert_eq!(value["accounts"][0]["provider"], "generic-imap");
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
