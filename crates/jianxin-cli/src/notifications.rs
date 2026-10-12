//! Native local notifications. A failed request leaves the watch event pending.
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

#[cfg(not(target_os = "macos"))]
const APP_ID: &str = "com.jianxin.mail";

fn content(event: &Value) -> Value {
    json!({
        "id": event["event_id"].as_str().unwrap_or("jianxin-test"),
        "thread": event["message"]["thread_id"].as_str().unwrap_or(""),
        "title": event["message"]["subject"].as_str().unwrap_or("(no subject)"),
        "subtitle": event["account_email"].as_str().unwrap_or("简信"),
        "body": event["message"]["from"][0]["email"].as_str().unwrap_or("unknown"),
        "account": event["account_email"].as_str().unwrap_or(""),
        "message_id": event["message"]["id"].as_str().unwrap_or(""),
    })
}

pub fn send(event: &Value, config: Option<&std::path::Path>) -> Result<()> {
    let mut payload = content(event);
    if let Some(config) = config {
        payload["config"] = std::fs::canonicalize(config)?.to_string_lossy().into_owned().into();
    }
    send_payload(&payload)
}

/// Shared native delivery for mail events and the authenticated HTTP hook.
pub fn send_payload(payload: &Value) -> Result<()> {
    #[cfg(target_os = "macos")]
    return helper(&["--notify", &payload.to_string()]);

    #[cfg(target_os = "windows")]
    return send_windows(payload);

    #[cfg(target_os = "linux")]
    {
        let mut notification = notify_rust::Notification::new();
        let body = format!("{}\n{}", payload["subtitle"].as_str().unwrap(),
            payload["body"].as_str().unwrap());
        #[cfg(target_os = "linux")]
        let body = plain_markup(&body);
        notification.appname("简信").summary(payload["title"].as_str().unwrap()).body(&body);
        #[cfg(target_os = "linux")]
        {
            notification.icon("jianxin").hint(notify_rust::Hint::DesktopEntry("jianxin".into()));
        }
        let target = click_arguments(payload);
        if target.is_some() {
            #[cfg(target_os = "linux")]
            notification.action("default", "打开邮件");
            notification.action("open-mail", "查看邮件");
        }
        let handle = notification.show().context("system notification request failed")?;
        if let Some(arguments) = target {
            let executable = std::env::current_exe()?;
            // Wait off the polling thread. Dismissal/expiry never opens the reader.
            std::thread::spawn(move || {
                let _ = handle.wait_for_response(move |response: &notify_rust::NotificationResponse| {
                    if should_open(response) {
                        let mut command = std::process::Command::new(executable);
                        command.args(arguments).stdin(std::process::Stdio::null())
                            .stdout(std::process::Stdio::null());
                        match command.spawn() {
                            Ok(mut child) => { let _ = child.wait(); }
                            Err(_) => eprintln!("无法打开通知对应的邮件，请检查 jianxin 程序"),
                        }
                    }
                });
            });
        }
        Ok(())
    }
}

pub fn manage(register: bool, test: bool) -> Result<()> {
    if test {
        send(&json!({"event_id": format!("jianxin-test-{}", uuid::Uuid::new_v4()),
            "account_email": "简信通知测试", "message": {"subject": "简信已接入系统通知",
                "from": [{"email": "新邮件到达后将在这里提醒你"}]}}), None)?;
        println!("notification accepted by the system; check your notification center");
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    return helper(&[if register { "--register-notifications" } else { "--notification-status" }]);
    #[cfg(not(target_os = "macos"))]
    {
        if register { register_identity()?; }
        println!("{}", json!({"app_id": APP_ID, "platform": std::env::consts::OS,
            "registered": identity_registered(), "permission": "controlled by desktop notification settings"}));
        Ok(())
    }
}

/// Routing data stays separate from display text; all arguments are passed directly.
#[cfg(any(test, not(target_os = "macos")))]
fn click_arguments(payload: &Value) -> Option<Vec<String>> {
    let account = payload["account"].as_str().filter(|value| !value.is_empty())?;
    let message = payload["message_id"].as_str().filter(|value| !value.is_empty())?;
    let mut arguments = Vec::new();
    if let Some(config) = payload["config"].as_str().filter(|value| !value.is_empty()) {
        arguments.extend(["--config".into(), config.into()]);
    }
    arguments.extend(["--account".into(), account.into(), "open".into(), "--".into(), message.into()]);
    Some(arguments)
}

#[cfg(target_os = "linux")]
fn should_open(response: &notify_rust::NotificationResponse) -> bool {
    matches!(response, notify_rust::NotificationResponse::Default)
        || matches!(response, notify_rust::NotificationResponse::Action(action) if action == "open-mail" || action == "default")
}

#[cfg(target_os = "macos")]
fn helper(arguments: &[&str]) -> Result<()> {
    let app = std::env::var_os("JIANXIN_MACOS_APP").map(std::path::PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join("Applications/Jianxin.app")))
        .filter(|path| path.is_dir())
        .unwrap_or_else(|| "/Applications/Jianxin.app".into());
    let executable = app.join("Contents/MacOS/Jianxin");
    if !executable.is_file() {
        bail!("install Jianxin.app first: bash scripts/build-macos-app.sh --install");
    }
    let mut child = std::process::Command::new(executable).args(arguments).spawn()
        .context("cannot start Jianxin notification helper")?;
    // A hung system service must not stop mail polling indefinitely.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(40);
    loop {
        if let Some(status) = child.try_wait()? {
            if !status.success() { bail!("Jianxin notification helper failed ({status}); run jianxin notifications --register"); }
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("Jianxin notification helper timed out");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[cfg(target_os = "linux")]
fn plain_markup(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(not(target_os = "macos"))]
fn identity_registered() -> bool {
    #[cfg(target_os = "windows")]
    return std::process::Command::new("reg")
        .args(["query", &format!(r"HKCU\Software\Classes\AppUserModelId\{APP_ID}"), "/v", "DisplayName"])
        .output().map(|output| output.status.success()).unwrap_or(false);
    #[cfg(target_os = "linux")]
    return dirs::data_local_dir().map(|data| data.join("applications/jianxin.desktop").is_file())
        .unwrap_or(false);
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    false
}

#[cfg(not(target_os = "macos"))]
fn register_identity() -> Result<()> {
    let data = dirs::data_local_dir().context("cannot locate application data directory")?;
    let icon = data.join("jianxin/jianxin.png");
    std::fs::create_dir_all(icon.parent().unwrap())?;
    std::fs::write(&icon, include_bytes!("../../../assets/jianxin-notification.png"))?;
    #[cfg(target_os = "windows")]
    {
        let executable = std::env::current_exe()?;
        let command = format!("\"{}\" notification-open \"%1\"", executable.display());
        for (key, name, value) in [
            (r"HKCU\Software\Classes\jianxin", None, "URL:Jianxin mail"),
            (r"HKCU\Software\Classes\jianxin", Some("URL Protocol"), ""),
            (r"HKCU\Software\Classes\jianxin\shell\open\command", None, command.as_str()),
        ] {
            let mut registry = std::process::Command::new("reg");
            registry.args(["add", key]);
            if let Some(name) = name { registry.args(["/v", name]); } else { registry.arg("/ve"); }
            let status = registry.args(["/t", "REG_SZ", "/d", value, "/f"]).status()?;
            anyhow::ensure!(status.success(), "cannot register notification click handler");
        }
        let key = format!(r"HKCU\Software\Classes\AppUserModelId\{APP_ID}");
        for (name, value) in [("DisplayName", "简信".to_string()), ("IconUri", icon.display().to_string())] {
            let status = std::process::Command::new("reg").args(["add", &key, "/v", name,
                "/t", "REG_SZ", "/d", &value, "/f"]).status()?;
            if !status.success() { bail!("cannot register Windows notification identity"); }
        }
    }
    #[cfg(target_os = "linux")]
    {
        let applications = data.join("applications");
        std::fs::create_dir_all(&applications)?;
        let icons = data.join("icons/hicolor/256x256/apps");
        std::fs::create_dir_all(&icons)?;
        std::fs::copy(&icon, icons.join("jianxin.png"))?;
        let executable = std::env::current_exe()?;
        let exec = executable.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"")
            .replace('`', "\\`").replace('$', "\\$").replace('%', "%%");
        std::fs::write(applications.join("jianxin.desktop"), format!(
            "[Desktop Entry]\nType=Application\nName=简信\nExec=\"{exec}\" tui\nIcon=jianxin\nTerminal=true\nCategories=Network;Email;\nX-GNOME-UsesNotifications=true\n"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn notification_contains_routing_metadata_without_message_body() {
        let payload = content(&json!({"event_id": "event-1", "account_email": "a@example.com",
            "message": {"id": "m1", "subject": "quotes \" and <html>", "thread_id": "t1",
                "text_body": "private body", "from": [{"email": "b@example.com"}]}}));
        assert_eq!(payload["id"], "event-1");
        assert_eq!(payload["title"], "quotes \" and <html>");
        assert_eq!(payload["thread"], "t1");
        assert_eq!(payload["account"], "a@example.com");
        assert_eq!(payload["message_id"], "m1");
        assert!(!payload.to_string().contains("private body"));
    }
    #[test]
    fn missing_metadata_has_safe_defaults() {
        let payload = content(&json!({}));
        assert_eq!(payload["title"], "(no subject)");
        assert_eq!(payload["body"], "unknown");
        assert!(click_arguments(&payload).is_none());
    }
    #[test]
    fn clicks_preserve_mailbox_message_and_custom_config_without_shell_parsing() {
        let payload = json!({"account":"a@example.com", "message_id":"--message", "config":"/path with spaces/config.json"});
        assert_eq!(click_arguments(&payload).unwrap(),
            ["--config", "/path with spaces/config.json", "--account", "a@example.com", "open", "--", "--message"]);
        assert!(click_arguments(&json!({"account":"a@example.com"})).is_none());
        assert!(click_arguments(&json!({"message_id":"m1"})).is_none());
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct NotificationTarget {
    config: std::path::PathBuf,
    account: String,
    message_id: String,
}

fn target_directory() -> Result<std::path::PathBuf> {
    Ok(dirs::data_local_dir().context("cannot locate notification data")?.join("jianxin/notification-targets"))
}

fn read_target(directory: &std::path::Path, link: &str) -> Result<NotificationTarget> {
    let url = ammonia::url::Url::parse(link)?;
    anyhow::ensure!(url.scheme() == "jianxin" && url.host_str() == Some("mail")
        && url.username().is_empty() && url.password().is_none() && url.port().is_none()
        && url.query().is_none() && url.fragment().is_none(), "invalid notification link");
    let token = uuid::Uuid::parse_str(url.path().strip_prefix('/').context("invalid notification path")?)?;
    let target: NotificationTarget = serde_json::from_slice(&std::fs::read(directory.join(format!("{token}.json")))?)?;
    anyhow::ensure!(!target.account.is_empty() && !target.message_id.is_empty(), "incomplete notification target");
    Ok(target)
}

pub fn open_notification(link: &str) -> Result<()> {
    let target = read_target(&target_directory()?, link)?;
    let config = jianxin_config::AppConfig::load(&target.config)?;
    let app = jianxin_core::App::new(config)?;
    crate::web::open_message(&app, &target.config, Some(&target.account), &target.message_id, false)
}

#[cfg(target_os = "windows")]
fn send_windows(payload: &Value) -> Result<()> {
    use windows::{core::HSTRING, Data::Xml::Dom::XmlDocument, UI::Notifications::{ToastNotification, ToastNotificationManager}};
    register_identity()?;
    let escape = |value: &str| value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;");
    let mut activation = String::new();
    if click_arguments(payload).is_some() {
        let directory = target_directory()?;
        std::fs::create_dir_all(&directory)?;
        let token = uuid::Uuid::new_v4();
        let target = NotificationTarget {
            config: payload["config"].as_str().map(std::path::PathBuf::from).unwrap_or_else(jianxin_config::default_config_path),
            account: payload["account"].as_str().unwrap().into(),
            message_id: payload["message_id"].as_str().unwrap().into(),
        };
        let file = std::fs::OpenOptions::new().write(true).create_new(true).open(directory.join(format!("{token}.json")))?;
        serde_json::to_writer(file, &target)?;
        activation = format!(" activationType=\"protocol\" launch=\"jianxin://mail/{token}\"");
    }
    let xml = format!("<toast{activation}><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text><text>{}</text></binding></visual></toast>",
        escape(payload["title"].as_str().unwrap_or("简信")), escape(payload["subtitle"].as_str().unwrap_or("")), escape(payload["body"].as_str().unwrap_or("")));
    let document = XmlDocument::new()?;
    document.LoadXml(&HSTRING::from(xml))?;
    let toast = ToastNotification::CreateToastNotification(&document)?;
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID))?.Show(&toast)?;
    Ok(())
}

#[cfg(test)]
mod routing_tests {
    use super::*;
    #[test]
    fn persisted_target_survives_sender_exit_and_rejects_foreign_links() {
        let directory = std::env::temp_dir().join(format!("jianxin-notification-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let token = uuid::Uuid::new_v4();
        let target = NotificationTarget { config: "/config with spaces/mail.json".into(), account: "a@example.com".into(), message_id: "mail-1".into() };
        std::fs::write(directory.join(format!("{token}.json")), serde_json::to_vec(&target).unwrap()).unwrap();
        let loaded = read_target(&directory, &format!("jianxin://mail/{token}")).unwrap();
        assert_eq!(loaded.account, target.account);
        assert_eq!(loaded.message_id, target.message_id);
        assert_eq!(loaded.config, target.config);
        for link in [format!("https://mail/{token}"), format!("jianxin://other/{token}"), format!("jianxin://mail/{token}?config=evil"), format!("jianxin://mail/{token}#evil"), "jianxin://mail/../../config".into(), "jianxin://mail/not-a-token".into()] {
            assert!(read_target(&directory, &link).is_err(), "{link}");
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}
