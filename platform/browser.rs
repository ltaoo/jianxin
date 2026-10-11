use anyhow::Result;

/// Open an address using the user's default browser without a console helper.
pub fn open(url: &str) -> Result<()> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
        let operation: Vec<u16> = "open".encode_utf16().chain(Some(0)).collect();
        let address: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(), operation.as_ptr(), address.as_ptr(),
                std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL,
            )
        } as isize;
        anyhow::ensure!(result > 32, "无法打开默认浏览器（系统错误 {result}）");
    }
    #[cfg(not(windows))]
    {
        use std::process::{Command, Stdio};
        let program = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        let status = Command::new(program).arg(url)
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status()?;
        anyhow::ensure!(status.success(), "无法打开默认浏览器");
    }
    Ok(())
}
