//! Loopback reader with server-synced read flags. Email HTML stays sandboxed.
use std::collections::HashSet;
use std::io::Write;
use std::path::Path;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use jianxin_core::App;
use jianxin_domain::{join_addresses, Message};
use tiny_http::{Header, Method, Response, Server};

#[path = "web_ui.rs"]
mod ui;
#[path = "web_agent.rs"]
mod web_agent;
#[path = "web_reply.rs"]
mod web_reply;

#[path = "notification_hook.rs"]
mod notification_hook;
#[path = "web_config.rs"]
mod web_config;
#[path = "web_listener.rs"]
mod web_listener;
#[path = "web_refresh.rs"]
mod web_refresh;

const PAGE_SIZE: u32 = 50;
const STYLE: &str = include_str!("web.css");

pub fn resolve_message(app: &App, selector: Option<&str>, hint: &str) -> Result<Message> {
    if let Some(selector) = selector {
        let cfg = app.resolve_account(Some(selector))?;
        return Ok(app.mail().resolve_message(&cfg.to_account().id, hint)?);
    }
    let mut matches = Vec::new();
    for cfg in app.account_configs() {
        // Full ids take precedence over prefixes.
        if let Ok(message) = app.mail().get(hint) {
            if message.account_id == cfg.to_account().id {
                return Ok(message);
            }
        }
        if let Ok(message) = app.mail().resolve_message(&cfg.to_account().id, hint) {
            matches.push(message);
        }
    }
    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => bail!("没有找到邮件；请使用完整邮件 ID，或用 -a 指定邮箱"),
        _ => bail!("邮件 ID 前缀不唯一；请使用完整 ID 或用 -a 指定邮箱"),
    }
}

pub fn message_url(host: &str, message: Option<&Message>) -> String {
    let mut url = ammonia::url::Url::parse(&format!("http://{host}/")).expect("loopback address");
    if let Some(message) = message {
        url.path_segments_mut()
            .expect("HTTP path")
            .extend(["message", &message.id]);
        url.query_pairs_mut()
            .append_pair("account", &message.account_id)
            .append_pair("folder", &message.folder_id);
        url.set_fragment(Some(&format!("mail-{}", message.id)));
    }
    url.into()
}

pub fn serve(
    app: App,
    config_path: &Path,
    selector: Option<&str>,
    port: u16,
    hint: Option<&str>,
    import_config: bool,
    idle_seconds: u64,
    control: crate::ServiceControl,
) -> Result<()> {
    let server = Server::http(("127.0.0.1", port))
        .map_err(|e| anyhow::anyhow!("无法启动本地 Web 服务: {e}"))?;
    serve_with_server(
        app,
        config_path,
        selector,
        server,
        hint,
        import_config,
        idle_seconds,
        control,
    )
}

/// Desktop sessions reuse a high loopback port, including remembered fallback ports.
pub fn serve_desktop(app: App, config_path: &Path, control: crate::ServiceControl) -> Result<()> {
    let server = web_listener::bind(&app.config().data_dir())?;
    serve_with_server(app, config_path, None, server, None, false, 0, control)
}

fn serve_with_server(
    app: App,
    config_path: &Path,
    selector: Option<&str>,
    server: Server,
    hint: Option<&str>,
    import_config: bool,
    idle_seconds: u64,
    control: crate::ServiceControl,
) -> Result<()> {
    let api_token = app
        .config()
        .general
        .api
        .token
        .as_ref()
        .map(|token| token.resolve())
        .transpose()?;
    if api_token.as_ref().is_some_and(|token| {
        !token.is_empty() && !token.bytes().all(|byte| byte.is_ascii_graphic())
    }) {
        bail!("general.api.token 必须为不含空格或控制字符的 ASCII 字符串");
    }
    let app = Arc::new(app);
    let selected = hint
        .map(|id| resolve_message(&app, selector, id))
        .transpose()?;
    let port = server
        .server_addr()
        .to_ip()
        .context("missing server address")?
        .port();
    let host = format!("127.0.0.1:{port}");
    let root = String::new();
    let mut url = message_url(&host, selected.as_ref());
    if import_config && selected.is_none() {
        url.push_str("#config-import");
    }
    println!("{url}");
    std::io::stdout().flush()?;
    if let Some(ready) = &control.ready {
        let _ = ready.send(url.clone());
    }
    let mut last_request = Instant::now();
    let mut memory_released = false;
    let active_writes = Arc::new(AtomicUsize::new(0));
    let refreshes = Arc::new(web_refresh::Refresher::default());
    let mut agent = web_agent::Manager::with_runtime(app, control.runtime);
    while !control.shutdown.load(Ordering::SeqCst) {
        let app = agent.app();
        if idle_seconds > 0
            && last_request.elapsed() >= Duration::from_secs(idle_seconds)
            && active_writes.load(Ordering::SeqCst) == 0
        {
            break;
        }
        let Some(mut request) = server.recv_timeout(Duration::from_millis(250))? else {
            if !memory_released
                && last_request.elapsed() >= Duration::from_secs(30)
                && active_writes.load(Ordering::SeqCst) == 0
            {
                app.db().release_idle_memory()?;
                memory_released = true;
            }
            continue;
        };
        memory_released = false;
        let request_host = request
            .headers()
            .iter()
            .find(|h| h.field.equiv("Host"))
            .map(|h| h.value.as_str());
        let hook = request.url().split('?').next() == Some("/api/notifications");
        let route = authorized_route(request_host, &host, &root, request.url())
            .filter(|route| route.split('?').next() != Some("api/notifications"));
        if hook {
            if let Some(reply) = notification_hook::authorize(&request, api_token.as_deref(), &host)
            {
                respond(request, reply);
                continue;
            }
            if request.method() != &Method::Post {
                respond(
                    request,
                    Reply::json(405, serde_json::json!({"error": "系统推送接口仅支持 POST"})),
                );
                continue;
            }
        }
        if request.method() == &Method::Post {
            let origin = request
                .headers()
                .iter()
                .find(|h| h.field.equiv("Origin"))
                .map(|h| h.value.as_str());
            let marker = request
                .headers()
                .iter()
                .find(|h| h.field.equiv("X-Jianxin-Request"))
                .map(|h| h.value.as_str());
            if !hook && (route.is_none() || !write_authorized(origin, marker, &host)) {
                respond(
                    request,
                    Reply::json(
                        403,
                        serde_json::json!({"error": "请求校验失败，请刷新阅读页后重试"}),
                    ),
                );
                continue;
            }
            if route == Some("api/mailboxes/reload") {
                last_request = Instant::now();
                let reply = match Arc::get_mut(&mut agent) {
                    None => Reply::json(
                        409,
                        serde_json::json!({"error": "正在处理邮件或助手请求，请稍后刷新邮箱配置"}),
                    ),
                    Some(manager) => reload_mailboxes(manager, config_path, &root),
                };
                respond(request, reply);
                continue;
            }
            if route == Some("api/config/import") {
                last_request = Instant::now();
                let reply = match Arc::get_mut(&mut agent) {
                    None => Reply::json(
                        409,
                        serde_json::json!({"error": "正在处理邮件或授权，请稍后导入"}),
                    ),
                    Some(manager) => web_config::import(manager, config_path, request.as_reader()),
                };
                respond(request, reply);
                continue;
            }
            if route == Some("api/mailboxes/oauth-settings") {
                last_request = Instant::now();
                let reply = match Arc::get_mut(&mut agent) {
                    None => Reply::json(
                        409,
                        serde_json::json!({"error":"正在处理邮件或授权，请稍后保存授权设置"}),
                    ),
                    Some(manager) => {
                        web_config::oauth_settings(manager, config_path, request.as_reader())
                    }
                };
                respond(request, reply);
                continue;
            }
            if active_writes
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                    (count < 4).then_some(count + 1)
                })
                .is_err()
            {
                respond(
                    request,
                    Reply::json(
                        429,
                        serde_json::json!({"error": "正在处理其他请求，请稍后重试"}),
                    ),
                );
                continue;
            }
            last_request = Instant::now();
            let route = if hook {
                "api/notifications"
            } else {
                route.unwrap()
            }
            .to_string();
            let app = Arc::clone(&app);
            let active = Arc::clone(&active_writes);
            let agent = Arc::clone(&agent);
            let refreshes = Arc::clone(&refreshes);
            let config_path = config_path.to_path_buf();
            std::thread::spawn(move || {
                let reply = if route == "api/config/source" {
                    web_config::source(request.as_reader())
                } else if route == "api/config/preview" {
                    web_config::preview(&app, request.as_reader())
                } else if route == "api/notifications" {
                    notification_hook::handle(&mut request, &*agent.runtime)
                } else if route.split('?').next() == Some("api/mailboxes/oauth") {
                    agent.oauth(&route, true, &config_path, &active)
                } else if route.split('?').next() == Some("api/mailboxes/refresh") {
                    refreshes.post(&app, &*agent.runtime, &route)
                } else if route.starts_with("api/agent/") {
                    agent.post(&route, &mut request, &active)
                } else if route.starts_with("api/reply/") {
                    web_reply::post(&app, &route, request.as_reader())
                } else {
                    update_flag_route(&app, &route, |cfg, id, flag, value| {
                        if flag == "starred" {
                            Ok(app.mail().star(cfg, id, value)?)
                        } else {
                            Ok(app.mail().mark_read(cfg, id, value)?)
                        }
                    })
                };
                respond(request, reply);
                active.fetch_sub(1, Ordering::SeqCst);
            });
            continue;
        }
        let reply = if route.is_none() {
            Reply::error(404, "页面不存在")
        } else if route.is_some_and(|route| {
            ["api/mailboxes/reload", "api/mailboxes/refresh"]
                .contains(&route.split('?').next().unwrap_or_default())
        }) {
            Reply::json(
                405,
                serde_json::json!({"error": "刷新邮箱需要 POST 请求"}),
            )
        } else if request.method() != &Method::Get && request.method() != &Method::Head {
            Reply::error(405, "仅支持阅读页面及 POST 邮件操作")
        } else {
            last_request = Instant::now();
            if route.unwrap().split('?').next() == Some("api/agent") {
                agent.get(route.unwrap())
            } else if route.unwrap().split('?').next() == Some("api/mailboxes/oauth") {
                agent.oauth(route.unwrap(), false, config_path, &active_writes)
            } else {
                render_route(&app, &root, route.unwrap()).unwrap_or_else(|_| {
                    if route.unwrap().starts_with("api/") {
                        Reply::json(
                            404,
                            serde_json::json!({"error": "邮件不存在或无法读取，请刷新重试"}),
                        )
                    } else {
                        Reply::error(404, "邮件不存在或无法读取")
                    }
                })
            }
        };
        respond(request, reply);
    }
    Ok(())
}

fn reload_mailboxes(manager: &mut web_agent::Manager, config_path: &Path, root: &str) -> Reply {
    if manager.reload_accounts(config_path).is_err() {
        return Reply::json(
            400,
            serde_json::json!({"error": "邮箱配置刷新失败，请检查配置文件，当前配置已保留"}),
        );
    }
    let daemon = match manager
        .runtime
        .config_changed(config_path, &manager.app().config().data_dir())
    {
        Ok(true) => serde_json::json!({"status": "restarted"}),
        Ok(false) => serde_json::json!({"status": "not_running"}),
        Err(error) => {
            eprintln!("cannot restart mail daemon: {error}");
            let message = if error.to_string().starts_with("data directory changed") {
                "邮箱配置已刷新，但数据目录已改变，请重启 Web 服务后再刷新"
            } else {
                "邮箱配置已刷新，但后台收信重启失败，请检查后台服务后重试"
            };
            serde_json::json!({"status": "failed", "error": message})
        }
    };
    let result = (|| -> Result<serde_json::Value> {
        let reply = render_route(manager.app(), root, "api/mailboxes")?;
        let mut value: serde_json::Value = serde_json::from_str(&reply.body)?;
        value["daemon"] = daemon;
        Ok(value)
    })();
    match result {
        Ok(value) => Reply::json(200, value),
        Err(_) => Reply::json(
            500,
            serde_json::json!({"error": "邮箱配置已刷新，但列表读取失败，请重试"}),
        ),
    }
}

fn respond(request: tiny_http::Request, reply: Reply) {
    let mut response = Response::from_string(if request.method() == &Method::Head {
        String::new()
    } else {
        reply.body
    })
    .with_status_code(reply.status);
    if reply.status == 401 {
        response.add_header(
            Header::from_bytes("WWW-Authenticate", "Bearer realm=\"jianxin\"").unwrap(),
        );
    }
    if reply.status == 405 && request.url().split('?').next() == Some("/api/notifications") {
        response.add_header(Header::from_bytes("Allow", "POST").unwrap());
    }
    for (name, value) in [
        ("Content-Type", reply.content_type),
        ("Cache-Control", "no-store"),
        ("Referrer-Policy", "no-referrer"),
        ("X-Content-Type-Options", "nosniff"),
        ("Content-Security-Policy", reply.csp.as_str()),
    ] {
        response.add_header(Header::from_bytes(name, value).unwrap());
    }
    let _ = request.respond(response);
}

fn write_authorized(origin: Option<&str>, marker: Option<&str>, host: &str) -> bool {
    marker == Some("1") && origin.is_none_or(|origin| origin == format!("http://{host}"))
}

fn update_flag_route(
    app: &App,
    route: &str,
    persist: impl FnOnce(&jianxin_config::AccountConfig, &str, &str, bool) -> Result<()>,
) -> Reply {
    let result = (|| -> Result<serde_json::Value> {
        let parsed = ammonia::url::Url::parse(&format!("http://127.0.0.1/{route}"))?;
        let id = parsed
            .path()
            .strip_prefix("/flags/")
            .context("invalid flag endpoint")?;
        if id.is_empty()
            || id.len() > 64
            || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
        {
            bail!("invalid message id");
        }
        let query: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        let flag = match (query.contains_key("read"), query.contains_key("starred")) {
            (true, false) => "read",
            (false, true) => "starred",
            _ => bail!("expected one flag"),
        };
        let value = match query.get(flag).map(String::as_str) {
            Some("1") => true,
            Some("0") => false,
            _ => bail!("invalid flag value"),
        };
        let message = app.mail().get(id)?;
        let cfg = app
            .account_configs()
            .iter()
            .find(|cfg| cfg.to_account().id == message.account_id)
            .context("unknown account")?;
        let previous = if flag == "starred" {
            jianxin_store::reader_repo::headers(&app.db().lock(), "", id, 1, 0)?
                .first()
                .context("unknown message")?
                .is_starred
        } else {
            message.is_read
        };
        if previous != value {
            persist(cfg, id, flag, value)?;
        }
        let updated = app.mail().get(id)?;
        let unread = updated
            .thread_id
            .as_ref()
            .and_then(|id| app.mail().thread(&updated.account_id, id).ok())
            .map(|detail| detail.thread.unread_count);
        Ok(
            serde_json::json!({"id": id, "is_read": updated.is_read, "is_starred": updated.is_starred, "thread_unread": unread}),
        )
    })();
    match result {
        Ok(value) => Reply::json(200, value),
        Err(_) => Reply::json(
            502,
            serde_json::json!({"error": "状态未同步成功，请检查邮箱连接或登录凭据，再点击按钮重试"}),
        ),
    }
}

fn authorized_route<'a>(
    request_host: Option<&str>,
    host: &str,
    root: &str,
    url: &'a str,
) -> Option<&'a str> {
    if request_host != Some(host) {
        return None;
    }
    url.strip_prefix(&format!("{root}/"))
}

struct Reply {
    status: u16,
    body: String,
    csp: String,
    content_type: &'static str,
}
impl Reply {
    fn page(body: String) -> Self {
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let script = include_str!("web-reader.js");
        let body = body.replace(
            "</body>",
            &format!("<script nonce=\"{nonce}\">{script}</script></body>"),
        );
        Self { status: 200, body, csp: format!("default-src 'none'; connect-src 'self'; script-src 'nonce-{nonce}'; style-src 'unsafe-inline'; frame-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'"), content_type: "text/html; charset=utf-8" }
    }
    fn json(status: u16, value: serde_json::Value) -> Self {
        Self {
            status,
            body: value.to_string(),
            csp: "default-src 'none'; frame-ancestors 'none'".into(),
            content_type: "application/json; charset=utf-8",
        }
    }
    fn error(status: u16, message: &str) -> Self {
        let mut reply = Self::page(shell(
            "简信",
            &format!("<main><h1>{}</h1></main>", escape(message)),
        ));
        reply.status = status;
        reply
    }
}

fn render_route(app: &App, root: &str, route: &str) -> Result<Reply> {
    ui::render(app, root, route)
}

fn render_legacy_route(app: &App, root: &str, route: &str) -> Result<Reply> {
    let parsed = ammonia::url::Url::parse(&format!("http://127.0.0.1/{route}"))?;
    let query: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
    let path = parsed.path().trim_start_matches('/');
    if path.is_empty() {
        let page = query
            .get("page")
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0)
            .min(100_000);
        let threads = app
            .mail()
            .threads_all(PAGE_SIZE + 1, page * PAGE_SIZE, false)?;
        let more = threads.len() > PAGE_SIZE as usize;
        let mut content = format!("<header><a class=brand href=\"{root}/\">简信 <small>本地阅读</small></a><p>本地邮件阅读 · 后台同步后刷新此页</p></header><main><h1>全部对话</h1><p class=muted>同一 thread 的邮件聚合为一条，点击主题查看首封邮件和后续回复。</p><ul class=messages>");
        for thread in threads.iter().take(PAGE_SIZE as usize) {
            content.push_str(&format!("<li class=\"{}\"><div class=meta><span>{}</span><time>{}</time></div><div class=conversation-row><a class=subject href=\"{root}/thread/{}\">{}</a><span class=message-count>MSGS {}</span></div><p class=muted>{} 封未读</p></li>",
                if thread.unread_count == 0 { "read" } else { "unread" }, escape(&account_email(app, &thread.account_id)), date(thread.last_message_at), thread.id,
                escape(&thread.subject), thread.message_count, thread.unread_count));
        }
        if threads.is_empty() {
            content
                .push_str("<li>暂无本地邮件。先运行 <code>jianxin sync</code>，再刷新此页。</li>");
        }
        content.push_str("</ul><nav aria-label=分页>");
        if page > 0 {
            content.push_str(&format!("<a href=\"{root}/?page={}\">上一页</a>", page - 1));
        }
        if more {
            content.push_str(&format!("<a href=\"{root}/?page={}\">下一页</a>", page + 1));
        }
        content.push_str("</nav></main>");
        return Ok(Reply::page(shell("全部对话 · 简信", &content)));
    }
    let (kind, id) = path.split_once('/').context("unknown route")?;
    if !matches!(kind, "message" | "body" | "thread")
        || id.len() > 64
        || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        bail!("unknown route");
    }
    let images = !query.get("images").is_some_and(|value| value == "0");
    let text = query.get("text").is_some_and(|value| value == "1");
    if kind == "thread" {
        let detail = app
            .account_configs()
            .iter()
            .find_map(|cfg| app.mail().thread(&cfg.to_account().id, id).ok())
            .context("unknown conversation")?;
        let mut content = format!("{}<main class=reader><h1>{}</h1><p class=muted>MSGS {} · <span data-thread-unread>{}</span> 封未读 · 首封邮件在前</p>{}", reader_header(root), escape(&detail.thread.subject), detail.thread.message_count, detail.thread.unread_count, reader_options(root, kind, id, images, text));
        for (index, message) in detail.messages.iter().enumerate() {
            content.push_str(&format!(
                "<details class=conversation-message id=\"mail-{}\" {}><summary>{} · {} · {}</summary>{}</details>",
                message.id,
                if index == 0 { "open" } else { "" },
                if index == 0 {
                    "首封邮件".into()
                } else {
                    format!("回复 {}", index)
                },
                escape(&join_addresses(&message.from)), date(message.sort_ts()),
                reader_message(app, root, message, images, text, index == 0)
            ));
        }
        content.push_str("</main>");
        return Ok(Reply::page(shell(&detail.thread.subject, &content)));
    }
    let message = app.mail().get(id)?;
    if !app
        .account_configs()
        .iter()
        .any(|c| c.to_account().id == message.account_id)
    {
        bail!("unknown account");
    }
    if kind == "body" {
        return Ok(render_body(&message, images, text));
    }
    let mut content = format!(
        "{}<main class=reader><h1>{}</h1>",
        reader_header(root),
        escape(subject(&message))
    );
    if let Some(thread_id) = &message.thread_id {
        content.push_str(&format!(
            "<a href=\"{root}/thread/{thread_id}#mail-{id}\">查看完整对话</a>"
        ));
    }
    content.push_str(&reader_options(root, kind, id, images, text));
    content.push_str(&reader_message(app, root, &message, images, text, true));
    content.push_str("</main>");
    Ok(Reply::page(shell(subject(&message), &content)))
}

fn reader_header(root: &str) -> String {
    format!("<header><a class=brand href=\"{root}/\">简信 <small>本地阅读</small></a><a href=\"{root}/\">全部对话</a></header>")
}

fn reader_options(root: &str, kind: &str, id: &str, images: bool, text: bool) -> String {
    format!("<nav aria-label=阅读选项><a href=\"{root}/{kind}/{id}?text={}&amp;images={}\">{}</a><a href=\"{root}/{kind}/{id}?text={}&amp;images={}\">{}</a></nav><p class=notice>{}邮件脚本和表单始终禁用。点击正文链接会打开新标签页。</p>",
        u8::from(!text), u8::from(images), if text { "HTML 视图" } else { "纯文本视图" },
        u8::from(text), u8::from(!images), if images { "屏蔽远程图片" } else { "加载远程图片" },
        if images { "已允许 HTTPS 图片，发件人可能获知你已阅读。" } else { "远程图片已屏蔽，避免阅读追踪。" })
}

fn reader_message(
    app: &App,
    root: &str,
    message: &Message,
    images: bool,
    text: bool,
    expanded: bool,
) -> String {
    let body_url = format!(
        "{root}/body/{}?images={}&amp;text={}",
        message.id,
        u8::from(images),
        u8::from(text)
    );
    let source = if expanded {
        format!("src=\"{body_url}\"")
    } else {
        String::new()
    };
    format!("<article class=mail-item data-message-id=\"{}\" data-read=\"{}\" data-read-url=\"{root}/flags/{}\"><div class=read-controls><span data-read-state>{}</span><button type=button data-read-button>{}</button><span data-read-status role=status aria-live=polite></span></div><dl><dt>发件人</dt><dd>{}</dd><dt>收件人</dt><dd>{}</dd><dt>邮箱</dt><dd>{}</dd><dt>时间</dt><dd>{}</dd></dl><iframe title=\"邮件正文\" scrolling=\"no\" sandbox=\"allow-same-origin allow-popups allow-popups-to-escape-sandbox\" referrerpolicy=\"no-referrer\" data-src=\"{body_url}\" {source}></iframe></article>",
        message.id, u8::from(message.is_read), message.id, if message.is_read { "已读" } else { "未读" }, if message.is_read { "标为未读" } else { "标为已读" },
        escape(&join_addresses(&message.from)), escape(&join_addresses(&message.to)), escape(&account_email(app, &message.account_id)), date(message.sort_ts()))
}

fn render_body(message: &Message, images: bool, text: bool) -> Reply {
    let body = if !text {
        message
            .html_body
            .as_ref()
            .filter(|html| !html.trim().is_empty())
            .map(|html| sanitize(html))
    } else {
        None
    }
    .unwrap_or_else(|| {
        format!(
            "<pre>{}</pre>",
            escape(
                message
                    .text_body
                    .as_deref()
                    .unwrap_or("这封邮件没有纯文本正文，请切换 HTML 视图。")
            )
        )
    });
    Reply {
        status: 200,
        content_type: "text/html; charset=utf-8",
        body: format!("<!doctype html><html><head><meta charset=utf-8><meta name=viewport content=\"width=device-width, initial-scale=1\"><style>body{{margin:1rem;font:16px/1.6 system-ui,sans-serif;overflow-wrap:anywhere;color:#20252b;background:white}}img{{max-width:100%;height:auto}}pre{{white-space:pre-wrap;font:inherit}}table{{max-width:100%}}</style></head><body><div id=jianxin-mail-content style=\"display:flow-root\">{body}</div></body></html>"),
        csp: format!("default-src 'none'; style-src 'unsafe-inline'; img-src {}; upgrade-insecure-requests; base-uri 'none'; form-action 'none'; sandbox allow-same-origin allow-popups allow-popups-to-escape-sandbox", if images { "https:" } else { "'none'" }),
    }
}

fn sanitize(html: &str) -> String {
    ammonia::Builder::default()
        .rm_clean_content_tags(&["style"])
        .add_tags(&["style"])
        .add_generic_attributes(&[
            "style", "class", "dir", "align", "valign", "width", "height",
        ])
        .url_schemes(HashSet::from(["https", "http", "mailto"]))
        .url_relative(ammonia::UrlRelative::Deny)
        .set_tag_attribute_value("a", "target", "_blank")
        .link_rel(Some("noopener noreferrer"))
        .clean(html)
        .to_string()
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn subject(message: &Message) -> &str {
    if message.subject.is_empty() {
        "（无主题）"
    } else {
        &message.subject
    }
}
fn account_email(app: &App, id: &str) -> String {
    app.account_configs()
        .iter()
        .find(|cfg| cfg.to_account().id == id)
        .map(|cfg| cfg.email.clone())
        .unwrap_or_else(|| id.into())
}
fn date(ts: i64) -> String {
    use chrono::{Local, TimeZone};
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "未知时间".into())
}
fn shell(title: &str, content: &str) -> String {
    format!("<!doctype html><html lang=zh-CN><head><meta charset=utf-8><meta name=viewport content=\"width=device-width, initial-scale=1\"><title>{}</title><style>{STYLE}</style></head><body><a class=skip href=#content>跳到邮件内容</a><div id=content>{content}</div></body></html>", escape(title))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reader_deep_link_selects_exact_mailbox_folder_and_message() {
        let (_, message) = fixture();
        let url = ammonia::url::Url::parse(&message_url("127.0.0.1:8787", Some(&message))).unwrap();
        assert_eq!(url.path(), format!("/message/{}", message.id));
        let params: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(params.get("account"), Some(&message.account_id));
        assert_eq!(params.get("folder"), Some(&message.folder_id));
        assert_eq!(
            url.fragment(),
            Some(format!("mail-{}", message.id).as_str())
        );
        assert_eq!(
            message_url("127.0.0.1:8787", None),
            "http://127.0.0.1:8787/"
        );
    }

    #[test]
    fn read_updates_require_request_marker_and_same_origin() {
        assert!(write_authorized(
            Some("http://127.0.0.1:8787"),
            Some("1"),
            "127.0.0.1:8787"
        ));
        assert!(!write_authorized(
            Some("https://evil.example"),
            Some("1"),
            "127.0.0.1:8787"
        ));
        assert!(!write_authorized(Some("null"), Some("1"), "127.0.0.1:8787"));
        assert!(!write_authorized(None, None, "127.0.0.1:8787"));
    }

    #[test]
    fn starred_write_updates_headers_counts_and_filtered_list_only_after_success() {
        let (app, message) = fixture();
        let route = format!("flags/{}?starred=1", message.id);
        let failed = update_flag_route(&app, &route, |_, _, _, _| bail!("private failure"));
        assert_eq!(failed.status, 502);
        assert!(!failed.body.contains("private failure"));
        assert!(!app.mail().get(&message.id).unwrap().is_starred);
        for starred in [true, false] {
            let reply = update_flag_route(
                &app,
                &format!("flags/{}?starred={}", message.id, u8::from(starred)),
                |cfg, id, flag, value| {
                    assert_eq!(cfg.email, "test@example.com");
                    assert_eq!(flag, "starred");
                    jianxin_store::message_repo::set_starred(&app.db().lock(), id, value)?;
                    Ok(())
                },
            );
            assert_eq!(reply.status, 200);
            let reply: serde_json::Value = serde_json::from_str(&reply.body).unwrap();
            assert_eq!(reply["is_starred"], starred);
            assert_eq!(reply["is_read"], false);
            let counts: serde_json::Value =
                serde_json::from_str(&render_route(&app, "", "api/mailboxes").unwrap().body)
                    .unwrap();
            assert_eq!(counts["counts"]["starred"], u8::from(starred));
            let list: serde_json::Value = serde_json::from_str(
                &render_route(&app, "", "api/threads?starred=1")
                    .unwrap()
                    .body,
            )
            .unwrap();
            assert_eq!(
                list["threads"].as_array().unwrap().len(),
                usize::from(starred)
            );
            let detail: serde_json::Value = serde_json::from_str(
                &render_route(&app, "", &format!("api/message/{}", message.id))
                    .unwrap()
                    .body,
            )
            .unwrap();
            assert_eq!(detail["messages"][0]["is_starred"], starred);
        }
        assert_eq!(
            update_flag_route(
                &app,
                &format!("flags/{}?starred=0", message.id),
                |_, _, _, _| panic!("same state must not write")
            )
            .status,
            200
        );
        for query in ["starred=invalid", "starred=1&read=1", "", "other=1"] {
            assert_ne!(
                update_flag_route(
                    &app,
                    &format!("flags/{}?{query}", message.id),
                    |_, _, _, _| panic!("invalid request must not write")
                )
                .status,
                200
            );
        }
    }

    #[test]
    fn update_read_success_failure_and_invalid_requests() {
        let (app, message) = fixture();
        let reply = update_flag_route(
            &app,
            &format!("flags/{}?read=1", message.id),
            |_, _, _, _| bail!("secret test credentials"),
        );
        assert_eq!(reply.status, 502);
        assert!(!reply.body.contains("secret test"));
        assert!(!app.mail().get(&message.id).unwrap().is_read);
        let reply = update_flag_route(
            &app,
            &format!("flags/{}?read=1", message.id),
            |cfg, id, flag, read| {
                assert_eq!(flag, "read");
                assert_eq!(cfg.email, "test@example.com");
                assert!(read);
                let conn = app.db().lock();
                jianxin_store::message_repo::set_read(&conn, id, read)?;
                jianxin_store::thread_repo::recompute(&conn, message.thread_id.as_ref().unwrap())?;
                Ok(())
            },
        );
        assert_eq!(reply.status, 200);
        assert!(reply.body.contains("\"is_read\":true"));
        assert!(reply.body.contains("\"thread_unread\":0"));
        let reply = update_flag_route(
            &app,
            &format!("flags/{}?read=1", message.id),
            |_, _, _, _| panic!("same-state request must not repeat provider write"),
        );
        assert_eq!(reply.status, 200);
        for route in [
            "flags/missing?read=1".into(),
            format!("flags/{}?read=invalid", message.id),
            format!("body/{}?read=1", message.id),
        ] {
            assert_ne!(
                update_flag_route(&app, &route, |_, _, _, _| panic!(
                    "invalid request must not write"
                ))
                .status,
                200
            );
        }
        // GET rendering never marks a message read.
        jianxin_store::message_repo::set_read(&app.db().lock(), &message.id, false).unwrap();
        render_legacy_route(&app, "/secret", &format!("message/{}", message.id)).unwrap();
        assert!(!app.mail().get(&message.id).unwrap().is_read);
    }
    pub(super) fn fixture() -> (App, Message) {
        let config = serde_json::from_value(serde_json::json!({
            "accounts": [{"email": "test@example.com", "auth": {"type": "password", "password": "test"}}]
        })).unwrap();
        let app =
            App::with_database(config, jianxin_store::Database::open_in_memory().unwrap()).unwrap();
        let account_id = app.account_configs()[0].to_account().id;
        let folder = jianxin_domain::Folder::new(
            &account_id,
            "INBOX",
            "Inbox",
            jianxin_domain::FolderKind::Inbox,
        );
        let mut message = Message::new();
        message.account_id = account_id;
        message.folder_id = folder.id.clone();
        message.remote_id = "1".into();
        message.subject = "<script>subject & title</script>".into();
        message.html_body = Some("<table><tr><td>HTML body</td></tr></table>".into());
        jianxin_store::folder_repo::upsert_many(&app.db().lock(), &[folder]).unwrap();
        jianxin_store::message_repo::upsert_many(&app.db().lock(), &[message.clone()]).unwrap();
        {
            let conn = app.db().lock();
            jianxin_store::thread_repo::assign(
                &conn,
                &message.account_id.clone(),
                std::slice::from_mut(&mut message),
            )
            .unwrap();
            jianxin_store::thread_repo::recompute_for(&conn, &[message.clone()]).unwrap();
        }
        (app, message)
    }
    #[test]
    fn root_routes_require_the_exact_loopback_host() {
        let host = "127.0.0.1:65250";
        assert_eq!(authorized_route(Some(host), host, "", "/"), Some(""));
        assert_eq!(
            authorized_route(Some(host), host, "", "/api/agent?account=a"),
            Some("api/agent?account=a")
        );
        assert!(authorized_route(Some("evil.example:65250"), host, "", "/").is_none());
        assert!(authorized_route(Some("127.0.0.1:65251"), host, "", "/").is_none());
        assert!(authorized_route(None, host, "", "/").is_none());
    }

    #[test]
    fn requires_loopback_host_and_secret_path() {
        let host = "127.0.0.1:8787";
        assert_eq!(
            authorized_route(Some(host), host, "/secret", "/secret/message/123"),
            Some("message/123")
        );
        assert!(authorized_route(Some("evil.example:8787"), host, "/secret", "/secret/").is_none());
        assert!(authorized_route(None, host, "/secret", "/secret/").is_none());
        assert!(authorized_route(Some(host), host, "/secret", "/wrong/").is_none());
        assert!(authorized_route(Some(host), host, "/secret", "/secret-extra/").is_none());
    }
    #[test]
    fn list_and_detail_use_escaped_metadata_and_sandbox() {
        let (app, message) = fixture();
        let list = render_legacy_route(&app, "/secret", "").unwrap();
        assert!(list
            .body
            .contains("&lt;script&gt;subject &amp; title&lt;/script&gt;"));
        assert!(list.body.contains(&format!(
            "/secret/thread/{}",
            message.thread_id.as_ref().unwrap()
        )));
        let detail =
            render_legacy_route(&app, "/secret", &format!("message/{}", message.id)).unwrap();
        assert!(detail
            .body
            .contains("sandbox=\"allow-same-origin allow-popups allow-popups-to-escape-sandbox\""));
        assert!(detail.body.contains("scrolling=\"no\""));
        assert!(detail.body.contains("ResizeObserver"));
        assert!(detail.csp.contains("script-src 'nonce-"));
        assert!(!detail.body.contains("HTML body")); // Body is never mixed into trusted UI.
        let body = render_legacy_route(&app, "/secret", &format!("body/{}", message.id)).unwrap();
        assert!(body.body.contains("HTML body"));
        assert!(body.csp.contains("img-src https:"));
        let blocked =
            render_legacy_route(&app, "/secret", &format!("body/{}?images=0", message.id)).unwrap();
        assert!(blocked.csp.contains("img-src 'none'"));
        let detail =
            render_legacy_route(&app, "/secret", &format!("message/{}?images=0", message.id))
                .unwrap();
        assert!(detail.body.contains("加载远程图片"));
        assert!(detail.body.contains("images=0&amp;text=0"));
        assert!(render_legacy_route(&app, "/secret", "message/../../config.json").is_err());
        assert!(render_legacy_route(&app, "/secret", "message/missing").is_err());
        assert_eq!(
            resolve_message(&app, None, &message.id).unwrap().id,
            message.id
        );
        assert_eq!(
            resolve_message(&app, Some("test@example.com"), &message.id)
                .unwrap()
                .id,
            message.id
        );
    }
    #[test]
    fn conversation_groups_replies_and_renders_oldest_first() {
        let (app, root) = fixture();
        let mut reply = root.clone();
        reply.id = jianxin_domain::new_id();
        reply.remote_id = "2".into();
        reply.sent_at = Some(200);
        {
            let conn = app.db().lock();
            jianxin_store::message_repo::upsert_many(&conn, &[reply.clone()]).unwrap();
            jianxin_store::thread_repo::recompute_for(&conn, &[reply.clone()]).unwrap();
        }
        let list = render_legacy_route(&app, "/secret", "").unwrap();
        assert_eq!(list.body.matches("<li class=").count(), 1);
        assert!(list.body.contains("MSGS 2"));
        let detail = render_legacy_route(
            &app,
            "/secret",
            &format!("thread/{}", root.thread_id.unwrap()),
        )
        .unwrap();
        assert_eq!(detail.body.matches("<iframe title=").count(), 2);
        assert_eq!(detail.body.matches(" src=\"").count(), 1);
        assert_eq!(
            detail
                .body
                .matches("<details class=conversation-message")
                .count(),
            2
        );
        assert!(detail.body.contains("首封邮件"));
        assert!(detail.body.contains("回复 1"));
        assert!(
            detail.body.find(&format!("/body/{}", root.id)).unwrap()
                < detail.body.find(&format!("/body/{}", reply.id)).unwrap()
        );
        assert!(!detail.body.contains("allow-scripts"));
    }

    #[test]
    fn strips_active_html_but_keeps_mail_layout() {
        let html = sanitize(
            r#"<meta http-equiv="refresh" content="0;url=https://evil.test"><script>alert(1)</script><form><input></form><iframe src="https://evil.test"></iframe><table style="color:red"><tr><td>hello</td></tr></table><style>td {padding:8px}</style><img src="https://example.test/pixel" onerror="evil()"><a href="javascript:evil()">bad</a><a href="https://example.test/">good</a>"#,
        );
        for forbidden in [
            "<script",
            "<meta",
            "<form",
            "<input",
            "<iframe",
            "onerror",
            "javascript:",
        ] {
            assert!(!html.contains(forbidden), "{html}");
        }
        assert!(html.contains("<table"));
        assert!(html.contains("style=\"color:red\""));
        assert!(html.contains("target=\"_blank\""));
        assert!(html.contains("noopener noreferrer"));
    }
    #[test]
    fn http_signature_images_upgrade_to_https_and_can_be_blocked() {
        let mut message = Message::new();
        message.html_body = Some(
            r#"<img src="http://thirdqq.qlogo.cn/avatar/0"><div style="background-image:url(http://example.test/logo.png)">signature</div>"#.into(),
        );
        for images in [true, false] {
            let reply = render_body(&message, images, false);
            assert!(reply.body.contains("http://thirdqq.qlogo.cn/avatar/0"));
            assert!(reply
                .body
                .contains("background-image:url(http://example.test/logo.png)"));
            assert!(reply
                .csp
                .split(';')
                .any(|directive| directive.trim() == "upgrade-insecure-requests"));
            assert!(reply.csp.contains(if images {
                "img-src https:"
            } else {
                "img-src 'none'"
            }));
            assert!(!reply.csp.contains("img-src http:"));
        }
    }

    #[test]
    fn text_is_escaped_and_remote_content_can_be_blocked() {
        let mut message = Message::new();
        message.text_body = Some("<script>secret</script>".into());
        let reply = render_body(&message, false, true);
        assert!(reply.body.contains("&lt;script&gt;"));
        assert!(reply.csp.contains("img-src 'none'"));
        assert!(reply.csp.contains("sandbox"));
        assert!(!reply.csp.contains("allow-scripts"));
        assert!(render_body(&message, true, false)
            .csp
            .contains("img-src https:"));
    }
}
