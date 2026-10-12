//! Single-instance routing for reopen and notification clicks.
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OpenRequest {
    pub message: Option<String>,
    pub account: Option<String>,
}

pub fn lock(data_dir: &Path) -> Result<Option<File>> {
    std::fs::create_dir_all(data_dir)?;
    let file = OpenOptions::new()
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

pub fn enqueue(data_dir: &Path, request: &OpenRequest) -> Result<()> {
    let directory = data_dir.join("desktop-requests");
    std::fs::create_dir_all(&directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
    }
    let path = directory.join(format!("{}.json", uuid::Uuid::new_v4()));
    let temporary = path.with_extension("tmp");
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(&serde_json::to_vec(request)?)?;
    drop(file);
    std::fs::rename(temporary, path)?;
    Ok(())
}

pub fn drain(data_dir: &Path) -> Vec<OpenRequest> {
    let Ok(entries) = std::fs::read_dir(data_dir.join("desktop-requests")) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .take(64)
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                return None;
            }
            let result = (|| -> Result<OpenRequest> {
                anyhow::ensure!(entry.file_type()?.is_file(), "not a request file");
                let mut bytes = Vec::new();
                File::open(&path)?.take(8193).read_to_end(&mut bytes)?;
                anyhow::ensure!(bytes.len() <= 8192, "request too large");
                Ok(serde_json::from_slice(&bytes)?)
            })();
            let _ = std::fs::remove_file(path);
            result.ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requests_preserve_routing_and_lock_releases_with_owner() {
        let directory =
            std::env::temp_dir().join(format!("jianxin-instance-{}", uuid::Uuid::new_v4()));
        let owner = lock(&directory).unwrap().unwrap();
        assert!(lock(&directory).unwrap().is_none());
        let request = OpenRequest {
            message: Some("--quoted mail".into()),
            account: Some("mail@example.com".into()),
        };
        enqueue(&directory, &request).unwrap();
        assert_eq!(drain(&directory), vec![request]);
        assert!(drain(&directory).is_empty());
        drop(owner);
        assert!(lock(&directory).unwrap().is_some());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
