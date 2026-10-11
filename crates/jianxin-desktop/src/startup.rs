use anyhow::{Context, Result};
use jianxin_config::AppConfig;
use std::{io::Write, path::{Path, PathBuf}};

#[path = "../../../platform/windows/startup.rs"]
mod error;
pub use error::report_error;

pub fn config_path(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit { return Ok(path); }
    if std::env::var_os("JIANXIN_CONFIG").is_some() {
        return Ok(jianxin_config::default_config_path());
    }
    let executable = std::env::current_exe()?;
    let portable = executable.parent().context("无法定位程序目录")?.join("config.json");
    Ok(if portable.is_file() { portable } else { jianxin_config::default_config_path() })
}

/// An unconfigured desktop is still a usable empty tray. Never overwrite an
/// existing config, including when two first-run processes race to create it.
pub fn load_or_create(path: &Path) -> Result<AppConfig> {
    if !path.try_exists()? {
        if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(path) {
            Ok(mut file) => file.write_all(b"{\"accounts\": []}\n")?,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {},
            Err(error) => return Err(error.into()),
        }
    }
    AppConfig::load(path).with_context(|| format!("无法读取邮箱配置 {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_config_starts_empty_and_existing_invalid_config_is_preserved() {
        let directory = std::env::temp_dir().join(format!("jianxin-startup-{}", jianxin_domain::new_id()));
        let path = directory.join("config.json");
        assert!(load_or_create(&path).unwrap().accounts.is_empty());
        std::fs::write(&path, b"invalid").unwrap();
        assert!(load_or_create(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"invalid");
        std::fs::remove_dir_all(directory).unwrap();
    }
}
