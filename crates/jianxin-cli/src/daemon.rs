//! Background process management for `jianxin daemon`.

use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::Serialize;

const PID_FILE_NAME: &str = "jianxin-daemon.pid";
const LOG_FILE_NAME: &str = "jianxin-daemon.log";
const LAUNCH_AGENT_LABEL: &str = "com.jianxin.mail-daemon";

pub struct Options<'a> {
    pub config_path: &'a Path,
    pub data_dir: &'a Path,
    pub selector: Option<&'a str>,
    pub interval: Option<u64>,
    pub stop: bool,
    pub status: bool,
}

#[derive(Debug, Serialize)]
pub struct Report {
    status: String,
    pid: Option<u32>,
    control_file: PathBuf,
    log_file: PathBuf,
}

impl Report {
    pub fn human_message(&self) -> String {
        match (self.status.as_str(), self.pid) {
            ("started", Some(pid)) => format!(
                "daemon started (pid {pid}); log: {}",
                self.log_file.display()
            ),
            ("started", None) => format!("daemon started; log: {}", self.log_file.display()),
            ("running", Some(pid)) => format!(
                "daemon is running (pid {pid}); log: {}",
                self.log_file.display()
            ),
            ("running", None) => format!(
                "daemon is managed and starting; log: {}",
                self.log_file.display()
            ),
            ("stopped", Some(pid)) => format!("daemon stopped (pid {pid})"),
            ("stopped", None) => "daemon stopped".to_string(),
            ("not_running", _) => "daemon is not running".to_string(),
            _ => self.status.clone(),
        }
    }
}

pub fn run(options: Options<'_>) -> Result<Report> {
    fs::create_dir_all(options.data_dir).with_context(|| {
        format!(
            "cannot create daemon data directory {}",
            options.data_dir.display()
        )
    })?;

    if cfg!(target_os = "macos") {
        run_launch_agent(options)
    } else {
        run_background_process(options)
    }
}

/// Restart an existing poller without enabling a service the user has stopped.
pub fn restart_if_running(config_path: &Path, data_dir: &Path) -> Result<bool> {
    if cfg!(target_os = "macos") {
        let Some(detail) = launch_agent_detail()? else { return Ok(false); };
        let current_config = parse_launch_config(&detail).and_then(|path| fs::canonicalize(path).ok());
        if current_config.as_deref() != Some(fs::canonicalize(config_path)?.as_path()) {
            return Ok(false);
        }
        if jianxin_config::AppConfig::load(config_path)?.data_dir() != data_dir {
            bail!("data directory changed; restart the Web service before restarting the mail daemon");
        }
        let previous_pid = parse_launch_pid(&detail);
        kickstart_launch_agent()?;
        thread::sleep(Duration::from_millis(500));
        let pid = launch_agent_detail()?.and_then(|detail| parse_launch_pid(&detail));
        if pid.is_none() || pid == previous_pid {
            bail!("mail daemon did not restart");
        }
        return Ok(true);
    }
    // ponytail: other platforms restart with config defaults; persist CLI overrides if needed.
    let options = Options { config_path, data_dir, selector: None, interval: None, stop: false, status: false };
    if run(Options { status: true, ..options })?.pid.is_none() {
        return Ok(false);
    }
    run(Options { stop: true, ..options })?;
    run(options)?;
    Ok(true)
}

fn watch_arguments(
    config_path: &Path,
    selector: Option<&str>,
    interval: Option<u64>,
) -> Vec<OsString> {
    let mut arguments = vec![
        OsString::from("--config"),
        config_path.as_os_str().to_os_string(),
    ];
    if let Some(selector) = selector {
        arguments.push(OsString::from("--account"));
        arguments.push(OsString::from(selector));
    }
    arguments.push(OsString::from("watch"));
    arguments.push(OsString::from("--notify"));
    if let Some(interval) = interval {
        arguments.push(OsString::from("--interval"));
        arguments.push(OsString::from(interval.to_string()));
    }
    arguments
}

// macOS uses launchd so the watcher survives terminal closure and is restarted
// if it exits unexpectedly.
fn run_launch_agent(options: Options<'_>) -> Result<Report> {
    let control_file = launch_agent_path()?;
    let log_file = options.data_dir.join(LOG_FILE_NAME);
    let current = launch_agent_status(&control_file, &log_file)?;

    if options.stop {
        return stop_launch_agent(current, control_file, log_file);
    }
    if options.status {
        return Ok(current.unwrap_or_else(|| Report {
            status: "not_running".to_string(),
            pid: None,
            control_file,
            log_file,
        }));
    }
    if let Some(report) = current {
        if report.pid.is_some() {
            return Ok(report);
        }
        kickstart_launch_agent()?;
        thread::sleep(Duration::from_millis(500));
        return launch_agent_status(&control_file, &log_file)?
            .ok_or_else(|| anyhow::anyhow!("launchd lost the jianxin service after kickstart"));
    }

    let executable =
        fs::canonicalize(std::env::current_exe().context("cannot locate the jianxin executable")?)
            .context("cannot resolve the jianxin executable")?;
    let config_path = fs::canonicalize(options.config_path)
        .with_context(|| format!("cannot resolve config {}", options.config_path.display()))?;
    let arguments = watch_arguments(&config_path, options.selector, options.interval);
    let plist = launch_agent_plist(&executable, &arguments, &log_file);

    if let Some(parent) = control_file.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    write_private_file(&control_file, plist.as_bytes())?;
    let _ = open_log(&log_file)?;

    let domain = launch_domain()?;
    let output = Command::new("launchctl")
        .args(["bootstrap", &domain])
        .arg(&control_file)
        .output()
        .context("cannot register the jianxin LaunchAgent")?;
    if !output.status.success() {
        bail!(
            "cannot register the jianxin LaunchAgent: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    kickstart_launch_agent()?;
    thread::sleep(Duration::from_millis(500));
    let mut report = launch_agent_status(&control_file, &log_file)?.ok_or_else(|| {
        anyhow::anyhow!(
            "launchd did not retain the jianxin service; see {}",
            log_file.display()
        )
    })?;
    if report.pid.is_none() {
        bail!(
            "launchd registered jianxin but did not start it; see {}",
            log_file.display()
        );
    }
    report.status = "started".to_string();

    let stale_pid_file = options.data_dir.join(PID_FILE_NAME);
    if stale_pid_file.exists() {
        let _ = fs::remove_file(stale_pid_file);
    }
    Ok(report)
}

fn stop_launch_agent(
    current: Option<Report>,
    control_file: PathBuf,
    log_file: PathBuf,
) -> Result<Report> {
    let Some(current) = current else {
        if control_file.exists() {
            fs::remove_file(&control_file)
                .with_context(|| format!("cannot remove {}", control_file.display()))?;
        }
        return Ok(Report {
            status: "not_running".to_string(),
            pid: None,
            control_file,
            log_file,
        });
    };

    let domain = launch_domain()?;
    let service = format!("{domain}/{LAUNCH_AGENT_LABEL}");
    let output = Command::new("launchctl")
        .args(["bootout", &service])
        .output()
        .context("cannot stop the jianxin LaunchAgent")?;
    if !output.status.success() {
        bail!(
            "cannot stop the jianxin LaunchAgent: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    if control_file.exists() {
        fs::remove_file(&control_file)
            .with_context(|| format!("cannot remove {}", control_file.display()))?;
    }
    Ok(Report {
        status: "stopped".to_string(),
        pid: current.pid,
        control_file,
        log_file,
    })
}

fn launch_agent_status(control_file: &Path, log_file: &Path) -> Result<Option<Report>> {
    let Some(detail) = launch_agent_detail()? else { return Ok(None); };
    let pid = parse_launch_pid(&detail);
    Ok(Some(Report {
        status: if pid.is_some() { "running" } else { "not_running" }.to_string(),
        pid,
        control_file: control_file.to_path_buf(),
        log_file: log_file.to_path_buf(),
    }))
}

fn launch_agent_detail() -> Result<Option<String>> {
    let domain = launch_domain()?;
    let service = format!("{domain}/{LAUNCH_AGENT_LABEL}");
    let output = Command::new("launchctl")
        .args(["print", &service])
        .output()
        .context("cannot query the jianxin LaunchAgent")?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned()))
}

fn kickstart_launch_agent() -> Result<()> {
    let service = format!("{}/{LAUNCH_AGENT_LABEL}", launch_domain()?);
    let output = Command::new("launchctl")
        .args(["kickstart", "-k", &service])
        .output()
        .context("cannot kickstart the jianxin LaunchAgent")?;
    if !output.status.success() {
        bail!(
            "cannot kickstart the jianxin LaunchAgent: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn launch_domain() -> Result<String> {
    let output = Command::new("id")
        .arg("-u")
        .output()
        .context("cannot determine the current user id")?;
    if !output.status.success() {
        bail!("cannot determine the current user id");
    }
    let uid = String::from_utf8(output.stdout)
        .context("current user id is not UTF-8")?
        .trim()
        .to_string();
    Ok(format!("gui/{uid}"))
}

fn launch_agent_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("cannot locate the user home directory")?;
    Ok(home
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{LAUNCH_AGENT_LABEL}.plist")))
}

fn launch_agent_plist(executable: &Path, arguments: &[OsString], log_file: &Path) -> String {
    let mut program_arguments = format!(
        "        <string>{}</string>\n",
        xml_escape(&executable.to_string_lossy())
    );
    for argument in arguments {
        program_arguments.push_str(&format!(
            "        <string>{}</string>\n",
            xml_escape(&argument.to_string_lossy())
        ));
    }
    let log_file = xml_escape(&log_file.to_string_lossy());
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n\
<dict>\n\
    <key>Label</key>\n\
    <string>{LAUNCH_AGENT_LABEL}</string>\n\
    <key>ProgramArguments</key>\n\
    <array>\n{program_arguments}    </array>\n\
    <key>RunAtLoad</key>\n\
    <true/>\n\
    <key>KeepAlive</key>\n\
    <true/>\n\
    <key>ProcessType</key>\n\
    <string>Background</string>\n\
    <key>ThrottleInterval</key>\n\
    <integer>10</integer>\n\
    <key>StandardOutPath</key>\n\
    <string>{log_file}</string>\n\
    <key>StandardErrorPath</key>\n\
    <string>{log_file}</string>\n\
</dict>\n\
</plist>\n"
    )
}

fn parse_launch_pid(detail: &str) -> Option<u32> {
    detail.lines().find_map(|line| {
        let (key, value) = line.trim().split_once(" = ")?;
        (key == "pid").then(|| value.parse().ok()).flatten()
    })
}

fn parse_launch_config(detail: &str) -> Option<PathBuf> {
    let mut arguments = detail.lines()
        .skip_while(|line| line.trim() != "arguments = {")
        .skip(1)
        .map(str::trim)
        .take_while(|line| *line != "}");
    arguments.find(|argument| *argument == "--config")?;
    arguments.next().filter(|path| !path.is_empty()).map(PathBuf::from)
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

// Other platforms use a detached process and a PID file.
fn run_background_process(options: Options<'_>) -> Result<Report> {
    let control_file = options.data_dir.join(PID_FILE_NAME);
    let log_file = options.data_dir.join(LOG_FILE_NAME);
    let running_pid = read_pid(&control_file).filter(|pid| process_is_running(*pid));

    if options.stop {
        return stop_background_process(running_pid, control_file, log_file);
    }
    if options.status {
        let status = if running_pid.is_some() {
            "running"
        } else {
            "not_running"
        };
        return Ok(Report {
            status: status.to_string(),
            pid: running_pid,
            control_file,
            log_file,
        });
    }
    if let Some(pid) = running_pid {
        return Ok(Report {
            status: "running".to_string(),
            pid: Some(pid),
            control_file,
            log_file,
        });
    }

    if control_file.exists() {
        fs::remove_file(&control_file)
            .with_context(|| format!("cannot remove stale pid file {}", control_file.display()))?;
    }
    start_background_process(options, control_file, log_file)
}

fn start_background_process(
    options: Options<'_>,
    control_file: PathBuf,
    log_file: PathBuf,
) -> Result<Report> {
    let executable = std::env::current_exe().context("cannot locate the jianxin executable")?;
    let arguments = watch_arguments(options.config_path, options.selector, options.interval);
    let log = open_log(&log_file)?;
    let stderr = log
        .try_clone()
        .context("cannot clone the daemon log file")?;

    let mut command = if cfg!(unix) {
        let mut command = Command::new("nohup");
        command.arg(&executable);
        command
    } else {
        Command::new(&executable)
    };
    command
        .args(arguments)
        .current_dir(options.data_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr));

    let mut child = command
        .spawn()
        .context("cannot start the background mail poller")?;
    thread::sleep(Duration::from_millis(250));
    if let Some(status) = child
        .try_wait()
        .context("cannot inspect the background mail poller")?
    {
        bail!(
            "background mail poller exited immediately with {status}; see {}",
            log_file.display()
        );
    }

    let pid = child.id();
    write_private_file(&control_file, format!("{pid}\n").as_bytes())?;
    Ok(Report {
        status: "started".to_string(),
        pid: Some(pid),
        control_file,
        log_file,
    })
}

fn stop_background_process(
    running_pid: Option<u32>,
    control_file: PathBuf,
    log_file: PathBuf,
) -> Result<Report> {
    let Some(pid) = running_pid else {
        if control_file.exists() {
            fs::remove_file(&control_file).with_context(|| {
                format!("cannot remove stale pid file {}", control_file.display())
            })?;
        }
        return Ok(Report {
            status: "not_running".to_string(),
            pid: None,
            control_file,
            log_file,
        });
    };

    let status = if cfg!(unix) {
        Command::new("kill").arg(pid.to_string()).status()
    } else {
        Command::new("taskkill")
            .args(["/PID", &pid.to_string()])
            .status()
    }
    .context("cannot stop the background mail poller")?;
    if !status.success() {
        bail!("failed to stop daemon process {pid}: {status}");
    }
    fs::remove_file(&control_file)
        .with_context(|| format!("cannot remove pid file {}", control_file.display()))?;
    Ok(Report {
        status: "stopped".to_string(),
        pid: Some(pid),
        control_file,
        log_file,
    })
}

fn open_log(path: &Path) -> Result<std::fs::File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .with_context(|| format!("cannot open daemon log {}", path.display()))
}

fn write_private_file(path: &Path, contents: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("cannot write {}", path.display()))?;
    file.write_all(contents)?;
    Ok(())
}

fn read_pid(path: &Path) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn process_is_running(pid: u32) -> bool {
    let status = if cfg!(unix) {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
    } else {
        Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
    };
    matches!(status, Ok(status) if status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_child_always_enables_notifications() {
        let arguments = watch_arguments(Path::new("/tmp/config.json"), Some("work"), Some(60));
        let arguments: Vec<_> = arguments
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            arguments,
            [
                "--config",
                "/tmp/config.json",
                "--account",
                "work",
                "watch",
                "--notify",
                "--interval",
                "60",
            ]
        );
    }

    #[test]
    fn launch_pid_is_parsed() {
        let detail = "state = running\n\tpid = 12345\n";
        assert_eq!(parse_launch_pid(detail), Some(12345));
        let detail = "arguments = {\n/bin/jianxin\n--config\n/tmp/mail {work}/config.json\nwatch\n}\npid = 12345\n";
        assert_eq!(parse_launch_config(detail), Some(PathBuf::from("/tmp/mail {work}/config.json")));
        assert_eq!(parse_launch_config("arguments = {\n--config\n}\n"), None);
        assert_eq!(parse_launch_config("environment = {\n--config\n/tmp/untrusted\n}\n"), None);
    }

    #[test]
    fn plist_escapes_program_arguments() {
        let plist = launch_agent_plist(
            Path::new("/Applications/A&B/jianxin"),
            &[OsString::from("a<b")],
            Path::new("/tmp/a&b.log"),
        );
        assert!(plist.contains("A&amp;B"));
        assert!(plist.contains("a&lt;b"));
        assert!(plist.contains("a&amp;b.log"));
    }
}
