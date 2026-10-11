use anyhow::{Context, Result};
use std::process::{Command, Stdio};
use windows_sys::Win32::{
    System::Console::{GetConsoleProcessList, GetConsoleWindow},
    UI::WindowsAndMessaging::{ShowWindow, SW_HIDE},
};

#[path = "../../../platform/windows/startup.rs"]
mod startup;

/// A newly allocated Explorer console has only this process attached. Keep
/// no-argument TUI behavior when invoked from an existing terminal instead.
pub fn handle_double_click() -> bool {
    if std::env::args_os().len() != 1 {
        return false;
    }
    let mut process = 0;
    let owns_console = unsafe { GetConsoleProcessList(&mut process, 1) == 1 };
    if !owns_console {
        return false;
    }
    unsafe { ShowWindow(GetConsoleWindow(), SW_HIDE); }
    if let Err(error) = launch() {
        startup::report_error(&format!("无法启动简信托盘：{error:#}"));
        std::process::exit(1);
    }
    true
}

fn launch() -> Result<()> {
    let executable = std::env::current_exe()?;
    let desktop = executable.parent().context("无法定位程序目录")?.join("jianxin-desktop.exe");
    Command::new(desktop)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .spawn().context("请把 jianxin.exe 和 jianxin-desktop.exe 放在同一目录")?;
    Ok(())
}
