use anyhow::{Context, Result};
use std::{
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, SystemTime},
};

#[path = "../../../platform/browser.rs"]
mod browser;

#[derive(Clone)]
pub struct Commands {
    pub executable: PathBuf,
    pub config: PathBuf,
}

impl Commands {
    pub fn resolve(executable: Option<PathBuf>, config: PathBuf) -> Result<Self> {
        let current = std::env::current_exe()?;
        let directory = current
            .parent()
            .context("desktop executable directory missing")?;
        let name = if cfg!(windows) {
            "jianxin.exe"
        } else {
            "jianxin"
        };
        let executable = executable.unwrap_or_else(|| {
            let sibling = directory.join(name);
            let bundled = directory
                .parent()
                .unwrap_or(directory)
                .join("Resources")
                .join(name);
            if sibling.is_file() {
                sibling
            } else if bundled.is_file() {
                bundled
            } else {
                name.into()
            }
        });
        Ok(Self { executable, config })
    }
    fn command(&self) -> Command {
        let mut command = Command::new(&self.executable);
        command
            .arg("--config")
            .arg(&self.config)
            .stdin(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        command
    }
    pub fn daemon(&self) -> Result<()> {
        let output = self
            .command()
            .arg("daemon")
            .output()
            .context("cannot start mail daemon")?;
        anyhow::ensure!(output.status.success(), "mail daemon failed to start");
        Ok(())
    }
    pub fn stop_daemon(&self) -> Result<()> {
        let output = self.command().args(["daemon", "--stop"]).output()
            .context("cannot stop background mail polling")?;
        anyhow::ensure!(output.status.success(), "background mail polling did not stop");
        Ok(())
    }
}

/// A reader worker owns the child so repeated tray clicks reuse a running Web
/// service. It runs independently of mail polling and snapshot loading.
pub struct Reader {
    commands: Commands,
    service: Option<(Child, String, Option<SystemTime>)>,
}

impl Reader {
    pub fn new(commands: Commands) -> Self {
        Self { commands, service: None }
    }

    pub fn open(&mut self, message: Option<&str>) -> Result<()> {
        if let Some(id) = message {
            let output = self.commands.command().arg("open").arg("--").arg(id)
                .output().context("无法启动邮件阅读器")?;
            anyhow::ensure!(output.status.success(), "邮件阅读器启动失败");
            return Ok(());
        }
        self.ensure_service()?;
        let (_, url, _) = self.service.as_ref().context("阅读器地址不可用")?;
        browser::open(url).with_context(|| format!("请手动打开 {url}"))
    }

    fn ensure_service(&mut self) -> Result<()> {
        let config_modified = std::fs::metadata(&self.commands.config)
            .and_then(|metadata| metadata.modified()).ok();
        if let Some((child, _, previous_config)) = self.service.as_mut() {
            if child.try_wait()?.is_none() && *previous_config == config_modified { return Ok(()); }
        }
        if let Some((mut previous, _, _)) = self.service.take() {
            // Existing tabs keep working when a config change needs a new
            // service. Reap the old process when its idle timeout expires.
            std::thread::spawn(move || { let _ = previous.wait(); });
        }
        let mut child = self.commands.command()
            .args(["web", "--port", "0", "--idle-seconds", "1800"])
            .stdout(Stdio::piped()).stderr(Stdio::null())
            .spawn().context("无法启动本地 Web 服务，请检查同目录的 jianxin 程序")?;
        let stdout = child.stdout.take().context("无法读取阅读器地址")?;
        let (ready, address) = mpsc::channel();
        std::thread::spawn(move || {
            let mut url = String::new();
            let result = BufReader::new(stdout).read_line(&mut url).map(|_| url);
            let _ = ready.send(result);
        });
        let result = (|| -> Result<String> {
            let url = address.recv_timeout(Duration::from_secs(10))
                .context("本地 Web 服务未及时响应，请重试")??;
            let url = url.trim();
            anyhow::ensure!(is_reader_url(url), "本地 Web 服务启动失败，未返回有效地址");
            anyhow::ensure!(child.try_wait()?.is_none(), "本地 Web 服务启动后退出，请重试");
            Ok(url.to_owned())
        })();
        match result {
            Ok(url) => { self.service = Some((child, url, config_modified)); Ok(()) }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(error)
            }
        }
    }
}

fn is_reader_url(url: &str) -> bool {
    url.strip_prefix("http://127.0.0.1:").and_then(|value| value.strip_suffix('/'))
        .and_then(|port| port.parse::<u16>().ok()).is_some_and(|port| port > 0)
}

pub fn instance_lock(data_dir: &Path) -> Result<Option<std::fs::File>> {
    std::fs::create_dir_all(data_dir)?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(data_dir.join("desktop.lock"))?;
    match fs2::FileExt::try_lock_exclusive(&file) {
        Ok(()) => Ok(Some(file)),
        Err(error)
            if error.kind() == std::io::ErrorKind::WouldBlock
                || error.raw_os_error() == fs2::lock_contended_error().raw_os_error() =>
        {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reader_accepts_only_a_loopback_service_address() {
        assert!(is_reader_url("http://127.0.0.1:55385/"));
        for address in ["", "http://127.0.0.1:0/", "http://example.com/", "http://127.0.0.1:80@evil.test/", "http://127.0.0.1:80/path"] {
            assert!(!is_reader_url(address));
        }
    }
    #[test]
    fn second_instance_is_rejected_until_the_owner_exits() {
        let directory =
            std::env::temp_dir().join(format!("jianxin-tray-lock-{}", jianxin_domain::new_id()));
        let owner = instance_lock(&directory).unwrap().unwrap();
        assert!(instance_lock(&directory).unwrap().is_none());
        drop(owner);
        let successor = instance_lock(&directory).unwrap().unwrap();
        drop(successor);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
