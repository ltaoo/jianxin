#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod commands;
mod model;
#[cfg(target_os = "macos")]
mod macos;
mod panel;
mod placement;
mod theme;

use anyhow::{Context as _, Result};
use clap::Parser;
use commands::Commands;
use gpui_kit::*;
use model::Snapshot;
use panel::{ClosedAt, Panel};
use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    sync::{mpsc, Arc},
    time::Duration,
};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem},
    MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};

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
    /// Show the panel immediately (also useful when the desktop has no tray host).
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
    ActionComplete,
    QuitComplete,
}

struct Tray {
    icon: TrayIcon,
    show: MenuItem,
    reader: MenuItem,
    refresh: MenuItem,
    quit: MenuItem,
}

impl Tray {
    fn new() -> Result<Self> {
        let menu = Menu::new();
        let show = MenuItem::new("查看未读邮件", true, None);
        let reader = MenuItem::new("打开邮件阅读器", true, None);
        let refresh = MenuItem::new("刷新未读列表", true, None);
        let quit = MenuItem::new("退出应用", true, None);
        menu.append_items(&[&show, &reader, &refresh, &quit])?;
        let image = image::load_from_memory(include_bytes!("../../../assets/jianxin-icon.png"))?
            .thumbnail(32, 32)
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
            show,
            reader,
            refresh,
            quit,
        })
    }
}

struct Desktop {
    tray: Tray,
    window: Option<AnyWindowHandle>,
    panel: Option<Entity<Panel>>,
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
        match gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| Panel::new(snapshot, requests, closed_at, window, cx))
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

fn main() -> Result<()> {
    let options = Options::parse();
    let config_path = std::fs::canonicalize(
        options
            .config
            .unwrap_or_else(jianxin_config::default_config_path),
    )
    .context("找不到邮箱配置；先运行 jianxin init 并配置邮箱")?;
    let config = jianxin_config::AppConfig::load(&config_path)?;
    let Some(instance_lock) = commands::instance_lock(&config.data_dir())? else {
        std::fs::write(config.data_dir().join("desktop-show"), b"show")?;
        eprintln!("简信托盘已运行");
        return Ok(());
    };
    let show_request = config.data_dir().join("desktop-show");
    #[cfg(target_os = "macos")]
    let diagnostics = config.data_dir().join("desktop-tray-status.json");
    if show_request.exists() {
        std::fs::remove_file(&show_request)?;
    }
    let commands = Commands::resolve(options.cli, config_path)?;
    let app = Arc::new(jianxin_core::App::new(config)?);
    let (requests, receiver) = mpsc::sync_channel(8);
    let (updates, incoming) = mpsc::channel();
    std::thread::spawn(move || {
        if !options.no_daemon && commands.daemon().is_err() {
            let _ = updates.send(Update::Error(
                "后台收信启动失败，请运行 jianxin daemon 检查后重试。仍可查看本地邮件。".into(),
            ));
        }
        loop {
            let snapshot = model::load_snapshot(&app).unwrap_or_else(|_| Snapshot {
                loaded: true,
                error: Some("未读邮件加载失败，请点击刷新重试".into()),
                ..Default::default()
            });
            if updates.send(Update::Snapshot(snapshot)).is_err() {
                break;
            }
            match receiver.recv_timeout(Duration::from_secs(5)) {
                Ok(Request::Quit) => {
                    if commands.stop_daemon().is_ok() {
                        let _ = updates.send(Update::QuitComplete);
                        break;
                    }
                    let _ = updates.send(Update::Error("后台收信未能停止，请点击退出重试".into()));
                }
                Ok(Request::Open(message)) => {
                    if commands.open(message.as_deref()).is_err() {
                        let _ = updates.send(Update::Error(
                            "邮件阅读器启动失败，请检查 jianxin 程序后重试".into(),
                        ));
                    } else {
                        let _ = updates.send(Update::ActionComplete);
                    }
                }
                Ok(Request::Refresh) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
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
                        eprintln!("Cannot create tray: {error}");
                        cx.update(|cx| cx.quit());
                        return;
                    }
                };
                #[cfg(target_os = "macos")]
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
                            if event.id == *desktop.tray.show.id() {
                                desktop.show(None, false, cx);
                            }
                            if event.id == *desktop.tray.reader.id() {
                                let _ = desktop.requests.try_send(Request::Open(None));
                            }
                            if event.id == *desktop.tray.refresh.id() {
                                let _ = desktop.requests.try_send(Request::Refresh);
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
