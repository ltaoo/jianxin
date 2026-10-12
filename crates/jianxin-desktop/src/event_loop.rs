//! Native message pumps; there is no renderer or GPU event loop.
use anyhow::Result;
use std::time::Duration;

#[cfg(target_os = "macos")]
pub fn run(mut start: impl FnMut() -> Result<()>, mut tick: impl FnMut() -> bool) -> Result<()> {
    use objc2::{rc::autoreleasepool, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSEventMask};
    use objc2_foundation::{NSDate, NSDefaultRunLoopMode};
    let marker = MainThreadMarker::new().expect("native event loop on main thread");
    let application = NSApplication::sharedApplication(marker);
    application.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    application.finishLaunching();
    start()?;
    loop {
        let quit = autoreleasepool(|_| {
            let deadline =
                NSDate::dateWithTimeIntervalSinceNow(Duration::from_millis(100).as_secs_f64());
            if let Some(event) = application.nextEventMatchingMask_untilDate_inMode_dequeue(
                NSEventMask::Any,
                Some(&deadline),
                unsafe { NSDefaultRunLoopMode },
                true,
            ) {
                application.sendEvent(&event);
            }
            tick()
        });
        if quit {
            return Ok(());
        }
    }
}

#[cfg(target_os = "windows")]
pub fn run(mut start: impl FnMut() -> Result<()>, mut tick: impl FnMut() -> bool) -> Result<()> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE, WM_QUIT,
    };
    start()?;
    loop {
        unsafe {
            let mut message: MSG = std::mem::zeroed();
            while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                if message.message == WM_QUIT {
                    return Ok(());
                }
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        if tick() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(target_os = "linux")]
pub fn run(mut start: impl FnMut() -> Result<()>, mut tick: impl FnMut() -> bool) -> Result<()> {
    start()?;
    while !tick() {
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}
