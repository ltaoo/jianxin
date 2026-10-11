#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod commands;
mod config_crypto;
mod config_encryption;
#[cfg(target_os = "macos")]
mod macos;
mod model;
mod panel;
mod placement;
mod theme;
mod startup;

use anyhow::Result;
use clap::Parser;
use commands::Commands;
use config_encryption::EncryptionWindow;
use gpui_kit::*;
use model::Snapshot;
use panel::{ClosedAt, Panel};
use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    sync::mpsc,
    time::Duration,
};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem},
    MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};

gpui_kit::assets::icon_assets!(PanelAssets, [
    Check, ChevronRight, Close, LoaderCircle,
    WindowClose, WindowMaximize, WindowMinimize, WindowRestore,
]);

#[derive(Parser)]
#[command(version, about = "简信：三端托盘与最新未读邮件面板")]
struct Options {
    #[arg(long)]
    config: Option<PathBuf>,
    /// Override the companion jianxin CLI executable.
    #[arg(long)]
    cli: Option<PathBuf>,
    /// Use an already-running watcher, without starting the mail daemon.
    #[arg(long)]
    no_daemon: bool,
    /// Show the panel immediately; by default only the tray icon is shown.
    #[arg(long)]
    show: bool,
}

enum Request {
    Refresh,
    Quit,
    Open(Option<String>),
}
enum Update {
    Snapshot(Snapshot),
    Error(String),
    ReaderError(String),
    ActionComplete,
    QuitComplete,
}

struct Tray {
    icon: TrayIcon,
    reader: MenuItem,
    refresh: MenuItem,
    encryption: MenuItem,
    quit: MenuItem,
}

impl Tray {
    fn new() -> Result<Self> {
        let menu = Menu::new();
        let reader = MenuItem::new("打开邮件阅读器", true, None);
        let refresh = MenuItem::new("刷新未读列表", true, None);
        let encryption = MenuItem::new("配置加密", true, None);
        let quit = MenuItem::new("退出应用", true, None);
        menu.append_items(&[&reader, &refresh, &encryption, &quit])?;
        let image = image::load_from_memory_with_format(
            include_bytes!("../../../assets/jianxin-tray.png"),
            image::ImageFormat::Png,
        )?
            .into_rgba8();
        let (width, height) = image.dimensions();
        let icon = tray_icon::Icon::from_rgba(image.into_raw(), width, height)?;
        let builder = TrayIconBuilder::new()
            .with_icon(icon)
            .with_tooltip("简信 · 正在加载未读邮件")
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false);
        #[cfg(target_os = "macos")]
        let builder = builder.with_autosave_name("com.jianxin.mail.tray");
        let icon = builder.build()?;
        #[cfg(target_os = "macos")]
        macos::configure(&icon)?;
        Ok(Self {
            icon,
            reader,
            refresh,
            encryption,
            quit,
        })
    }
}

struct Desktop {
    tray: Tray,
    window: Option<AnyWindowHandle>,
    panel: Option<Entity<Panel>>,
    encryption: Rc<RefCell<EncryptionWindow>>,
    closed_at: ClosedAt,
    snapshot: Snapshot,
    requests: mpsc::SyncSender<Request>,
    action_error: Option<String>,
    _instance_lock: std::fs::File,
}

impl Desktop {
    fn show(&mut self, anchor: Option<tray_icon::Rect>, toggle: bool, cx: &mut App) {
        if let Some(window) = self.window {
            if window
                .update(cx, |_, window, _| {
                    if toggle {
                        window.remove_window();
                    } else {
                        window.activate_window();
                    }
                })
                .is_ok()
            {
                if toggle {
                    self.window = None;
                    self.panel = None;
                }
                return;
            }
            self.window = None;
            self.panel = None;
        }
        // A tray click may deactivate the panel before its click event arrives.
        if toggle
            && self
                .closed_at
                .borrow()
                .is_some_and(|at| at.elapsed() < Duration::from_millis(250))
        {
            return;
        }
        let tokens = theme::Tokens::load();
        let (bounds, display_id) = placement::placement(
            cx,
            anchor,
            tokens.length("layout", "notification").as_f32(),
            tokens.length("layout", "trayHeight").as_f32(),
            tokens.length("spacing", "2").as_f32(),
        );
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            display_id,
            titlebar: None,
            kind: WindowKind::PopUp,
            focus: true,
            show: true,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            app_id: Some("com.jianxin.mail".into()),
            ..Default::default()
        };
        let snapshot = self.snapshot.clone();
        let requests = self.requests.clone();
        let closed_at = self.closed_at.clone();
        let encryption = self.encryption.clone();
        match gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| Panel::new(snapshot, requests, closed_at, encryption, window, cx))
        }) {
            Ok((window, panel)) => {
                panel.update(cx, |panel, cx| {
                    panel.action_error = self.action_error.clone();
                    cx.notify();
                });
                self.window = Some(window);
                self.panel = Some(panel);
                cx.activate(true);
                let _ = self.requests.try_send(Request::Refresh);
            }
            Err(error) => {
                eprintln!("Cannot open unread panel: {error}");
            }
        }
    }
    fn apply(&mut self, update: Update, cx: &mut App) {
        match update {
            Update::Snapshot(snapshot) => {
                if snapshot.error.is_none() {
                    let _ = self
                        .tray
                        .icon
                        .set_tooltip(Some(format!("简信 · {} 封未读邮件", snapshot.total)));
                    #[cfg(target_os = "macos")]
                    self.tray.icon.set_title(if snapshot.total > 0 {
                        Some(snapshot.total.to_string())
                    } else {
                        None
                    });
                }
                self.snapshot = snapshot;
            }
            Update::Error(error) => {
                self.action_error = Some(error);
            }
            Update::ReaderError(error) => {
                self.action_error = Some(error);
                self.show(None, false, cx);
            }
            Update::QuitComplete => { cx.quit(); return; }
            Update::ActionComplete => {
                self.action_error = None;
            }
        }
        if let Some(panel) = &self.panel {
            panel.update(cx, |panel, cx| {
                panel.snapshot = self.snapshot.clone();
                panel.action_error = self.action_error.clone();
                cx.notify();
            });
        }
    }
}

fn main() {
    std::panic::set_hook(Box::new(|panic| {
        startup::report_error(&format!("桌面程序遇到异常，无法继续运行：{panic}"));
    }));
    if let Err(error) = run() {
        startup::report_error(&format!("简信无法启动：{error:#}"));
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let options = Options::parse();
    let config_path = startup::config_path(options.config)?;
    let config = startup::load_or_create(&config_path)?;
    let config_path = std::fs::canonicalize(config_path)?;
    let Some(instance_lock) = commands::instance_lock(&config.data_dir())? else {
        if options.show {
            std::fs::write(config.data_dir().join("desktop-show"), b"show")?;
        }
        eprintln!("简信托盘已运行");
        return Ok(());
    };
    let show_request = config.data_dir().join("desktop-show");
    #[cfg(target_os = "macos")]
    let diagnostics = config.data_dir().join("desktop-tray-status.json");
    #[cfg(windows)]
    let diagnostics = config.data_dir().join("desktop-tray-status.json");
    if show_request.exists() {
        std::fs::remove_file(&show_request)?;
    }
    let commands = Commands::resolve(options.cli, config_path.clone())?;
    let mut app = jianxin_core::App::new(config)?;
    let (requests, receiver) = mpsc::sync_channel(8);
    let (updates, incoming) = mpsc::channel();
    let (reader_requests, reader_incoming) = mpsc::sync_channel::<Option<String>>(8);
    let (poll_requests, poll_incoming) = mpsc::sync_channel(8);
    let mut reader = commands::Reader::new(commands.clone());
    let reader_updates = updates.clone();
    std::thread::spawn(move || {
        for message in reader_incoming {
            let update = match reader.open(message.as_deref()) {
                Ok(()) => Update::ActionComplete,
                Err(error) => Update::ReaderError(format!("邮件阅读器打开失败：{error:#}")),
            };
            if reader_updates.send(update).is_err() { break; }
        }
    });
    let dispatch_updates = updates.clone();
    std::thread::spawn(move || {
        for request in receiver {
            let result = match request {
                Request::Open(message) => reader_requests.try_send(message).is_ok(),
                request => poll_requests.try_send(request).is_ok(),
            };
            if !result {
                let _ = dispatch_updates.send(Update::Error("操作正在进行，请稍后重试".into()));
            }
        }
    });
    std::thread::spawn(move || {
        let mut config_modified = std::fs::metadata(&commands.config).and_then(|metadata| metadata.modified()).ok();
        if !options.no_daemon && !app.account_configs().is_empty() && commands.daemon().is_err() {
            let _ = updates.send(Update::Error(
                "后台收信启动失败，请运行 jianxin daemon 检查后重试。仍可查看本地邮件。".into(),
            ));
        }
        loop {
            let modified = std::fs::metadata(&commands.config).and_then(|metadata| metadata.modified()).ok();
            if modified != config_modified {
                let reload = (|| -> Result<()> {
                    let loaded = jianxin_config::AppConfig::load(&commands.config)?;
                    let mut config = app.config().clone();
                    config.accounts = loaded.accounts;
                    app.reload_config(config)?;
                    Ok(())
                })();
                if reload.is_ok() {
                    config_modified = modified;
                    if !options.no_daemon && !app.account_configs().is_empty() && commands.daemon().is_err() {
                        let _ = updates.send(Update::Error("邮箱已导入，但后台收信启动失败，请稍后重试".into()));
                    }
                } else {
                    let _ = updates.send(Update::Error("邮箱配置更新失败，当前邮箱已保留，请检查配置后重试".into()));
                }
            }
            let snapshot = model::load_snapshot(&app).unwrap_or_else(|_| Snapshot {
                loaded: true,
                error: Some("未读邮件加载失败，请点击刷新重试".into()),
                ..Default::default()
            });
            if updates.send(Update::Snapshot(snapshot)).is_err() {
                break;
            }
            match poll_incoming.recv_timeout(Duration::from_secs(5)) {
                Ok(Request::Quit) => {
                    if commands.stop_daemon().is_ok() {
                        let _ = updates.send(Update::QuitComplete);
                        break;
                    }
                    let _ = updates.send(Update::Error("后台收信未能停止，请点击退出重试".into()));
                }
                Ok(Request::Open(_)) => unreachable!("reader requests use their own worker"),
                Ok(Request::Refresh) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    gpui_kit::application()
        .with_quit_mode(QuitMode::Explicit)
        .with_assets(PanelAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            theme::Tokens::load().apply(cx.window_appearance(), cx);
            #[cfg(target_os = "macos")]
            if let Some(marker) = objc2::MainThreadMarker::new() {
                objc2_app_kit::NSApplication::sharedApplication(marker)
                    .setActivationPolicy(objc2_app_kit::NSApplicationActivationPolicy::Accessory);
            }
            // Defer tray construction until the native event loop is running (macOS).
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let desktop = cx.update(|cx| -> Result<_> {
                    let mut desktop = Desktop {
                        tray: Tray::new()?,
                        window: None,
                        panel: None,
                        encryption: Rc::new(RefCell::new(EncryptionWindow::new(config_path))),
                        closed_at: Rc::new(RefCell::new(None)),
                        snapshot: Snapshot::default(),
                        requests,
                        action_error: None,
                        _instance_lock: instance_lock,
                    };
                    if options.show {
                        desktop.show(None, false, cx);
                    }
                    Ok(desktop)
                });
                let mut desktop = match desktop {
                    Ok(desktop) => desktop,
                    Err(error) => {
                        startup::report_error(&format!("无法创建托盘图标：{error:#}"));
                        cx.update(|cx| cx.quit());
                        return;
                    }
                };
                #[cfg(target_os = "macos")]
                let mut diagnostic_ticks = 0;
                #[cfg(windows)]
                let mut diagnostic_ticks = 0;
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(100))
                        .await;
                    let quit = cx.update(|cx| {
                        if show_request.exists() {
                            let _ = std::fs::remove_file(&show_request);
                            desktop.show(None, false, cx);
                        }
                        #[cfg(target_os = "macos")]
                        if diagnostic_ticks < 30 {
                            diagnostic_ticks += 1;
                            if diagnostic_ticks % 10 == 0 { macos::record_status(&desktop.tray.icon, &diagnostics); }
                        }
                        #[cfg(windows)]
                        if diagnostic_ticks < 30 {
                            diagnostic_ticks += 1;
                            if diagnostic_ticks % 10 == 0 {
                                let rect = desktop.tray.icon.rect().map(|rect| serde_json::json!({
                                    "x": rect.position.x, "y": rect.position.y,
                                    "width": rect.size.width, "height": rect.size.height,
                                }));
                                let status = serde_json::json!({
                                    "pid": std::process::id(), "tray_created": true,
                                    "panel_open": desktop.window.is_some(), "rect": rect,
                                });
                                if let Ok(bytes) = serde_json::to_vec_pretty(&status) {
                                    let _ = std::fs::write(&diagnostics, bytes);
                                }
                            }
                        }
                        for update in incoming.try_iter() {
                            desktop.apply(update, cx);
                        }
                        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
                            if let TrayIconEvent::Click {
                                id,
                                rect,
                                button: MouseButton::Left,
                                button_state: MouseButtonState::Up,
                                ..
                            } = event
                            {
                                if id == *desktop.tray.icon.id() {
                                    desktop.show(Some(rect), true, cx);
                                }
                            }
                        }
                        while let Ok(event) = MenuEvent::receiver().try_recv() {
                            if event.id == *desktop.tray.quit.id() {
                                let _ = desktop.requests.try_send(Request::Quit);
                            }
                            if event.id == *desktop.tray.reader.id() {
                                let _ = desktop.requests.try_send(Request::Open(None));
                            }
                            if event.id == *desktop.tray.refresh.id() {
                                let _ = desktop.requests.try_send(Request::Refresh);
                            }
                            if event.id == *desktop.tray.encryption.id() {
                                if desktop.encryption.borrow_mut().show(cx).is_err() {
                                    desktop.apply(
                                        Update::Error("配置加密窗口打开失败，重试打开".into()),
                                        cx,
                                    );
                                }
                            }
                        }
                        false
                    });
                    if quit {
                        break;
                    }
                }
            })
            .detach();
        });
    Ok(())
}
