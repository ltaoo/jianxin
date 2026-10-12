//! CLI entrypoints around the shared embedded Web service.
use anyhow::{bail, Context, Result};
use jianxin_core::App;
use jianxin_web::{resolve_message, RuntimeHooks, ServiceControl};
use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Command, Stdio},
    sync::{mpsc, Arc},
};

struct CliHooks;
impl RuntimeHooks for CliHooks {
    fn config_changed(&self, config: &Path, data: &Path) -> Result<bool> {
        crate::daemon::restart_if_running(config, data)
    }
    fn notify(&self, payload: &serde_json::Value) -> Result<()> {
        crate::notifications::send_payload(payload)
    }
    fn mcp_command(&self) -> Option<serde_json::Value> {
        Some(
            serde_json::json!({"command":std::env::current_exe().ok()?.to_string_lossy(),"args":["agent"]}),
        )
    }
}

pub fn run(
    app: App,
    config: &Path,
    selector: Option<&str>,
    port: u16,
    hint: Option<&str>,
    open: bool,
    import_config: bool,
    idle_seconds: u64,
) -> Result<()> {
    let (ready, incoming) = mpsc::channel::<String>();
    if open {
        std::thread::spawn(move || {
            if let Ok(url) = incoming.recv() {
                let _ = open_browser(&url);
            }
        });
    }
    let control = ServiceControl {
        runtime: Arc::new(CliHooks),
        ready: Some(ready),
        ..Default::default()
    };
    jianxin_web::serve(
        app,
        config,
        selector,
        port,
        hint,
        import_config,
        idle_seconds,
        control,
    )
}

pub fn open_message(
    app: &App,
    config: &Path,
    selector: Option<&str>,
    hint: &str,
    no_open: bool,
) -> Result<()> {
    let message = resolve_message(app, selector, hint)?;
    let config = std::fs::canonicalize(config)?;
    let mut child = Command::new(std::env::current_exe()?)
        .arg("--config")
        .arg(config)
        .args(["web", "--port", "0", "--idle-seconds", "1800", "--message"])
        .arg(&message.id)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("无法启动本地邮件阅读服务")?;
    let mut url = String::new();
    BufReader::new(child.stdout.take().context("missing reader output")?).read_line(&mut url)?;
    let url = url.trim();
    if !url.starts_with("http://127.0.0.1:") {
        let _ = child.wait();
        bail!("本地阅读服务启动失败；运行 jianxin web 查看错误");
    }
    println!("{url}");
    if !no_open {
        open_browser(url)?;
    }
    Ok(())
}

// Invoked only by the user's explicit open command, never by tests.
fn open_browser(url: &str) -> Result<()> {
    let mut command = if cfg!(target_os = "macos") {
        Command::new("open")
    } else if cfg!(target_os = "windows") {
        let mut c = Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else {
        Command::new("xdg-open")
    };
    if !command
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?
        .success()
    {
        bail!("无法打开浏览器，请手动打开上方网址");
    }
    Ok(())
}
