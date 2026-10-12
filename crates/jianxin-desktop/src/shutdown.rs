//! Quit must finish even when network calls or native destructors are stuck.
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

static REQUESTED: AtomicBool = AtomicBool::new(false);
static FALLBACK_ARMED: AtomicBool = AtomicBool::new(false);
const EXIT_DEADLINE: Duration = Duration::from_secs(2);

pub fn requested() -> bool {
    REQUESTED.load(Ordering::Acquire)
}

pub fn request_exit() {
    REQUESTED.store(true, Ordering::Release);
    arm_fallback(EXIT_DEADLINE);
}

pub fn arm_exit_fallback() {
    arm_fallback(EXIT_DEADLINE);
}

fn arm_fallback(deadline: Duration) {
    if !FALLBACK_ARMED.swap(true, Ordering::AcqRel) {
        std::thread::spawn(move || {
            std::thread::sleep(deadline);
            // All mail polling is owned by this process. OS process termination
            // also releases its sockets and instance lock; no daemon RPC is needed.
            force_exit();
        });
    }
}

fn force_exit() -> ! {
    // Bypass native atexit handlers too: they may be the stalled cleanup.
    #[cfg(unix)]
    unsafe {
        libc::_exit(0);
    }
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};
        TerminateProcess(GetCurrentProcess(), 0);
    }
    #[allow(unreachable_code)]
    std::process::exit(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{process::Command, time::Instant};

    #[test]
    fn blocked_cleanup_cannot_prevent_exit() {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "shutdown::tests::blocked_cleanup_fixture",
                "--ignored",
            ])
            .spawn()
            .unwrap();
        let started = Instant::now();
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                assert!(started.elapsed() < Duration::from_secs(2));
                break;
            }
            if started.elapsed() > Duration::from_secs(2) {
                let _ = child.kill();
                let _ = child.wait();
                panic!("quit waited indefinitely for blocked cleanup");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    #[ignore = "subprocess fixture; intentionally blocks until exit fallback fires"]
    fn blocked_cleanup_fixture() {
        REQUESTED.store(true, Ordering::Release);
        arm_fallback(Duration::from_millis(150));
        assert!(requested());
        // Simulates a permanently stuck server join or native tray destructor.
        loop {
            std::thread::park();
        }
    }
}
