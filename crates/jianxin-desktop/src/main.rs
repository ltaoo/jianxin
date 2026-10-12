#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
#[path = "../../../platform/browser.rs"]
mod browser;
mod event_loop;
mod instance;
#[cfg(target_os = "macos")]
mod macos;
mod model;
#[allow(dead_code)]
mod notifications;
mod options;
mod shutdown;
mod startup;
mod tray;
#[allow(dead_code)]
#[path = "../../jianxin-cli/src/watch.rs"]
mod watch;

use anyhow::{Context, Result};
use instance::OpenRequest;
use jianxin_core::App;
use jianxin_web::{RuntimeHooks, ServiceControl};
use std::{
    cell::RefCell,
    path::Path,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};

enum Update {
    Snapshot(model::Snapshot),
    Error(String),
}
struct DesktopHooks {
    sync: mpsc::Sender<()>,
    snapshot: mpsc::Sender<()>,
    enabled: bool,
}
impl RuntimeHooks for DesktopHooks {
    fn config_changed(&self, _config: &Path, _data: &Path) -> Result<bool> {
        self.snapshot.send(())?;
        if self.enabled {
            self.sync.send(())?;
        }
        Ok(self.enabled)
    }
    fn notify(&self, payload: &serde_json::Value) -> Result<()> {
        notifications::send_payload(payload)
    }
}

fn main() {
    std::panic::set_hook(Box::new(|panic| {
        startup::report_error(&format!("桌面程序遇到异常，无法继续运行：{panic}"))
    }));
    if let Err(error) = run() {
        startup::report_error(&format!("简信无法启动：{error:#}"));
        std::process::exit(1);
    }
}

fn reload_accounts(
    app: &mut App,
    path: &Path,
    modified: &mut Option<std::time::SystemTime>,
) -> Result<()> {
    let current = std::fs::metadata(path)?.modified()?;
    if *modified != Some(current) {
        let loaded = jianxin_config::AppConfig::load(path)?;
        let mut config = app.config().clone();
        config.accounts = loaded.accounts;
        app.reload_config(config)?;
        *modified = Some(current);
    }
    Ok(())
}

fn run() -> Result<()> {
    let options = options::Options::parse()?;
    let config_path = startup::config_path(options.config)?;
    let config = startup::load_or_create(&config_path)?;
    let config_path = std::fs::canonicalize(config_path)?;
    let app = App::new(config.clone())?;
    if std::env::var_os("JIANXIN_DESKTOP_HEADLESS").is_some() {
        return jianxin_web::serve(
            app,
            &config_path,
            None,
            0,
            None,
            false,
            0,
            ServiceControl::default(),
        );
    }
    let data_dir = config.data_dir();
    let Some(_instance_lock) = instance::lock(&data_dir)? else {
        if let Some(request) = options.open {
            instance::enqueue(&data_dir, &request)?;
        }
        return Ok(());
    };
    #[cfg(target_os = "macos")]
    if !options.no_daemon {
        stop_legacy_watcher(&config_path, &data_dir)?;
    }
    let shutdown = Arc::new(AtomicBool::new(false));
    let (sync, sync_incoming) = mpsc::channel();
    let (snapshot, snapshot_incoming) = mpsc::channel();
    let hooks = Arc::new(DesktopHooks {
        sync,
        snapshot,
        enabled: !options.no_daemon,
    });
    let (updates, incoming) = mpsc::channel();
    let (ready, address) = mpsc::channel();
    let server_control = ServiceControl {
        shutdown: shutdown.clone(),
        ready: Some(ready),
        runtime: hooks.clone(),
    };
    let server_config = config_path.clone();
    let server_updates = updates.clone();
    let server_shutdown = shutdown.clone();
    let server = std::thread::spawn(move || {
        if let Err(error) =
            jianxin_web::serve(app, &server_config, None, 0, None, false, 0, server_control)
        {
            eprintln!("Web service: {error:#}");
            let _ =
                server_updates.send(Update::Error("Web 服务已停止，请退出应用后重新启动".into()));
            server_shutdown.store(true, Ordering::SeqCst);
        }
    });
    let url = address
        .recv_timeout(Duration::from_secs(10))
        .context("Web 服务未能启动")?;
    let mut snapshot_app = App::new(config.clone())?;
    let snapshot_config = config_path.clone();
    let snapshot_shutdown = shutdown.clone();
    let snapshot_updates = updates.clone();
    let snapshot_worker = std::thread::spawn(move || {
        let mut modified = None;
        while !snapshot_shutdown.load(Ordering::SeqCst) {
            let result = reload_accounts(&mut snapshot_app, &snapshot_config, &mut modified)
                .and_then(|_| model::load_snapshot(&snapshot_app));
            let update = match result {
                Ok(snapshot) => Update::Snapshot(snapshot),
                Err(error) => {
                    eprintln!("unread menu: {error:#}");
                    Update::Error("未读邮件加载失败，请刷新重试".into())
                }
            };
            if snapshot_updates.send(update).is_err() {
                break;
            }
            match snapshot_incoming.recv_timeout(Duration::from_secs(5)) {
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                _ => while snapshot_incoming.try_recv().is_ok() {},
            }
        }
    });
    let mut watcher_app = App::new(config.clone())?;
    let watcher_config = config_path.clone();
    let watcher_shutdown = shutdown.clone();
    let watcher_updates = updates.clone();
    let watcher_snapshots = hooks.snapshot.clone();
    std::thread::spawn(move || {
        let mut modified = None;
        let mut poller = watch::Poller::default();
        let mut next_poll = Instant::now();
        while !watcher_shutdown.load(Ordering::SeqCst) {
            let previous = modified;
            if let Err(error) = reload_accounts(&mut watcher_app, &watcher_config, &mut modified) {
                eprintln!("reload watcher: {error:#}");
            }
            if previous != modified {
                next_poll = Instant::now();
            }
            if !options.no_daemon && Instant::now() >= next_poll {
                if let Err(error) = poller.tick(&watcher_app) {
                    eprintln!("desktop watcher: {error:#}");
                    let _ = watcher_updates.send(Update::Error(
                        "后台收信失败，请在阅读器中检查邮箱状态并重试".into(),
                    ));
                }
                let _ = watcher_snapshots.send(());
                next_poll = Instant::now()
                    + Duration::from_secs(watcher_app.config().general.watch.interval_secs.max(5));
            }
            match sync_incoming.recv_timeout(Duration::from_secs(1)) {
                Ok(()) => {
                    next_poll = Instant::now();
                    while sync_incoming.try_recv().is_ok() {}
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    });
    let (open, open_incoming) = mpsc::channel::<OpenRequest>();
    let reader_shutdown = shutdown.clone();
    let reader_updates = updates;
    let reader_config = config_path.clone();
    let mut reader_app = App::new(config)?;
    std::thread::spawn(move || {
        let mut modified = None;
        for request in open_incoming {
            if reader_shutdown.load(Ordering::SeqCst) {
                break;
            }
            let result = (|| -> Result<()> {
                reload_accounts(&mut reader_app, &reader_config, &mut modified)?;
                browser::open(&open_url(&reader_app, &url, &request)?)
            })();
            if let Err(error) = result {
                eprintln!("open reader: {error:#}");
                let _ = reader_updates.send(Update::Error(
                    "阅读器打开失败，请重试或手动打开本机阅读地址".into(),
                ));
            }
        }
    });
    if let Some(request) = options.open {
        open.send(request)?;
    }
    let desktop = Rc::new(RefCell::new(None::<tray::Tray>));
    let starting = desktop.clone();
    let mut status = None;
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    let mut diagnostic_ticks = 0;
    let result = event_loop::run(
        move || {
            *starting.borrow_mut() = Some(tray::Tray::new()?);
            Ok(())
        },
        || {
            if shutdown::requested() {
                return true;
            }
            let mut desktop = desktop.borrow_mut();
            let tray = desktop.as_mut().expect("tray initialized");
            for action in tray.actions() {
                match action {
                    tray::Action::Quit => {
                        shutdown::request_exit();
                        return true;
                    }
                    tray::Action::Refresh => {
                        status = None;
                        let _ = hooks.config_changed(&config_path, &data_dir);
                    }
                    tray::Action::Open(message) => {
                        let _ = open.send(OpenRequest {
                            message,
                            account: None,
                        });
                    }
                }
            }
            for request in instance::drain(&data_dir) {
                let _ = open.send(request);
            }
            let mut snapshot = tray.snapshot();
            for update in incoming.try_iter() {
                match update {
                    Update::Snapshot(value) => snapshot = value,
                    Update::Error(error) => status = Some(error),
                }
            }
            if let Err(error) = tray.update(snapshot, status.clone()) {
                eprintln!("tray menu: {error:#}");
            }
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            if diagnostic_ticks < 30 {
                diagnostic_ticks += 1;
                if diagnostic_ticks % 10 == 0 {
                    tray.record_status(&data_dir.join("desktop-tray-status.json"));
                }
            }
            shutdown.load(Ordering::SeqCst)
        },
    );
    // Arm before any native destructor or join: neither can hold quit hostage.
    shutdown::arm_exit_fallback();
    shutdown.store(true, Ordering::SeqCst);
    let _ = hooks.snapshot.send(());
    let _ = hooks.sync.send(());
    drop(desktop);
    drop(hooks);
    drop(open);
    let _ = server.join();
    let _ = snapshot_worker.join();
    result
}

fn open_url(app: &App, root: &str, request: &OpenRequest) -> Result<String> {
    let host = root
        .strip_prefix("http://")
        .and_then(|root| root.strip_suffix('/'))
        .context("阅读器地址无效")?;
    let message = request
        .message
        .as_deref()
        .map(|id| jianxin_web::resolve_message(app, request.account.as_deref(), id))
        .transpose()?;
    Ok(jianxin_web::message_url(host, message.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mail_menu_links_to_exact_mailbox_folder_and_mail_without_marking_read() {
        use jianxin_domain::{Folder, FolderKind, Message};
        let config = serde_json::from_value(serde_json::json!({"accounts":[{"email":"test@example.com","auth":{"type":"password","password":"unused"}}]})).unwrap();
        let app =
            App::with_database(config, jianxin_store::Database::open_in_memory().unwrap()).unwrap();
        let account = app.account_configs()[0].to_account().id;
        let folder = Folder::new(&account, "INBOX", "INBOX", FolderKind::Inbox);
        let message = Message {
            id: "menu-mail".into(),
            account_id: account.clone(),
            folder_id: folder.id.clone(),
            remote_id: "1".into(),
            subject: "Menu mail".into(),
            ..Default::default()
        };
        jianxin_store::folder_repo::upsert_many(&app.db().lock(), std::slice::from_ref(&folder))
            .unwrap();
        jianxin_store::message_repo::upsert_many(&app.db().lock(), std::slice::from_ref(&message))
            .unwrap();
        let root = "http://127.0.0.1:12345/";
        let url = open_url(
            &app,
            root,
            &OpenRequest {
                message: Some(message.id.clone()),
                account: None,
            },
        )
        .unwrap();
        let parsed = ammonia::url::Url::parse(&url).unwrap();
        assert_eq!(parsed.path(), "/message/menu-mail");
        assert_eq!(
            parsed
                .query_pairs()
                .find(|(key, _)| key == "account")
                .unwrap()
                .1,
            account
        );
        assert_eq!(
            parsed
                .query_pairs()
                .find(|(key, _)| key == "folder")
                .unwrap()
                .1,
            folder.id
        );
        assert_eq!(parsed.fragment(), Some("mail-menu-mail"));
        assert!(!app.mail().get(&message.id).unwrap().is_read);
        assert_eq!(open_url(&app, root, &OpenRequest::default()).unwrap(), root);
    }
}

#[cfg(target_os = "macos")]
fn stop_legacy_watcher(config: &Path, data_dir: &Path) -> Result<()> {
    use std::process::{Command, Stdio};
    let Some(home) = dirs::home_dir() else {
        return Ok(());
    };
    let plist = home.join("Library/LaunchAgents/com.jianxin.mail-daemon.plist");
    if !plist.is_file() {
        return Ok(());
    }
    let stored = Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", "Print :ProgramArguments:2"])
        .arg(&plist)
        .output()?;
    if !stored.status.success() {
        return Ok(());
    }
    let stored = String::from_utf8_lossy(&stored.stdout);
    if std::fs::canonicalize(stored.trim()).ok().as_deref() != Some(config) {
        return Ok(());
    }
    let uid = Command::new("id").arg("-u").output()?;
    let domain = format!(
        "gui/{}/com.jianxin.mail-daemon",
        String::from_utf8_lossy(&uid.stdout).trim()
    );
    let running = Command::new("launchctl")
        .args(["print", &domain])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if running.success() {
        let stopped = Command::new("launchctl")
            .args(["bootout", &domain])
            .status()?;
        anyhow::ensure!(stopped.success(), "旧收信服务未能停止，请退出旧版本后重试");
    }
    std::fs::rename(
        plist,
        data_dir.join(format!("legacy-mail-daemon-{}.plist", uuid::Uuid::new_v4())),
    )?;
    Ok(())
}
