//! Visible startup errors for executables launched from Explorer.
pub fn report_error(message: &str) {
    let log = std::env::temp_dir().join("jianxin-startup-error.log");
    let _ = std::fs::write(&log, message);
    eprintln!("{message}");
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
        let body: Vec<u16> = format!("{message}\n\n日志：{}", log.display()).encode_utf16().chain(Some(0)).collect();
        let title: Vec<u16> = "简信 · 启动失败".encode_utf16().chain(Some(0)).collect();
        MessageBoxW(std::ptr::null_mut(), body.as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR);
    }
}
