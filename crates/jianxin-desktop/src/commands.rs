use anyhow::{Context, Result};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

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
    pub fn open(&self, message: Option<&str>) -> Result<()> {
        let mut command = self.command();
        if let Some(id) = message {
            command.arg("open").arg("--").arg(id);
        } else {
            command.args(["web", "--port", "0", "--idle-seconds", "1800", "--open"]);
        }
        if message.is_some() {
            let output = command.output().context("cannot start mail reader")?;
            anyhow::ensure!(output.status.success(), "mail reader failed to start");
        } else {
            let mut child = command
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .context("cannot start mail reader")?;
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Ok(())
    }
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
