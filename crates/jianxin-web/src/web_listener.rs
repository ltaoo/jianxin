//! Stable, remembered loopback ports for the desktop application.
use std::{error::Error, io, path::Path};

use anyhow::{bail, Context, Result};
use tiny_http::Server;

const DEFAULT_PORT: u16 = 65250;
const LAST_PORT: u16 = 65265;
const PORT_FILE: &str = "desktop-web-port";

pub(super) fn bind(data_dir: &Path) -> Result<Server> {
    bind_with(data_dir, |port| Server::http(("127.0.0.1", port)))
}

fn bind_with<T>(
    data_dir: &Path,
    mut listen: impl FnMut(u16) -> std::result::Result<T, Box<dyn Error + Send + Sync>>,
) -> Result<T> {
    let path = data_dir.join(PORT_FILE);
    let saved = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| text.trim().parse::<u16>().ok())
        .filter(|port| (DEFAULT_PORT..=LAST_PORT).contains(port));
    let ports = saved
        .into_iter()
        .chain((DEFAULT_PORT..=LAST_PORT).filter(|port| Some(*port) != saved));
    for port in ports {
        match listen(port) {
            Ok(server) => {
                if saved != Some(port) {
                    std::fs::create_dir_all(data_dir).context("无法创建本地 Web 端口记录目录")?;
                    std::fs::write(&path, port.to_string()).context("无法保存本地 Web 端口")?;
                }
                return Ok(server);
            }
            Err(error)
                if error
                    .downcast_ref::<io::Error>()
                    .is_some_and(|error| error.kind() == io::ErrorKind::AddrInUse) => {}
            Err(error) => bail!("无法启动本地 Web 服务 (127.0.0.1:{port}): {error}"),
        }
    }
    bail!("本地 Web 端口 {DEFAULT_PORT}–{LAST_PORT} 均被占用，请关闭占用端口的程序后重试")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn occupied() -> Box<dyn Error + Send + Sync> {
        io::Error::from(io::ErrorKind::AddrInUse).into()
    }

    #[test]
    fn default_port_is_fixed_and_fallback_is_remembered_even_after_default_becomes_free() {
        let directory =
            std::env::temp_dir().join(format!("jianxin-web-port-{}", uuid::Uuid::new_v4()));
        assert_eq!(bind_with(&directory, Ok).unwrap(), DEFAULT_PORT);
        let mut attempts = Vec::new();
        let fallback = bind_with(&directory, |port| {
            attempts.push(port);
            if port == DEFAULT_PORT {
                Err(occupied())
            } else {
                Ok(port)
            }
        })
        .unwrap();
        assert_eq!(attempts, [DEFAULT_PORT, DEFAULT_PORT + 1]);
        assert_eq!(fallback, DEFAULT_PORT + 1);
        assert_eq!(bind_with(&directory, Ok).unwrap(), fallback);
        assert_eq!(
            std::fs::read_to_string(directory.join(PORT_FILE)).unwrap(),
            fallback.to_string()
        );
        // A newly occupied remembered port may move once and persist its replacement.
        assert_eq!(
            bind_with(&directory, |port| if port == fallback {
                Err(occupied())
            } else {
                Ok(port)
            })
            .unwrap(),
            DEFAULT_PORT
        );
        assert_eq!(bind_with(&directory, Ok).unwrap(), DEFAULT_PORT);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn invalid_records_are_ignored_and_other_bind_failures_do_not_change_ports() {
        let directory =
            std::env::temp_dir().join(format!("jianxin-web-port-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        for value in ["", "invalid", "0", "80", "65535"] {
            std::fs::write(directory.join(PORT_FILE), value).unwrap();
            assert_eq!(bind_with(&directory, Ok).unwrap(), DEFAULT_PORT);
        }
        let mut attempts = Vec::new();
        let result: Result<u16> = bind_with(&directory, |port| {
            attempts.push(port);
            Err(io::Error::from(io::ErrorKind::PermissionDenied).into())
        });
        assert!(result.is_err());
        assert_eq!(attempts, [DEFAULT_PORT]);
        assert_eq!(
            std::fs::read_to_string(directory.join(PORT_FILE)).unwrap(),
            DEFAULT_PORT.to_string()
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn exhaustion_reports_a_clear_error_instead_of_switching_to_a_random_port() {
        let directory =
            std::env::temp_dir().join(format!("jianxin-web-port-{}", uuid::Uuid::new_v4()));
        let mut attempts = Vec::new();
        let result: Result<u16> = bind_with(&directory, |port| {
            attempts.push(port);
            Err(occupied())
        });
        assert!(result.unwrap_err().to_string().contains("均被占用"));
        assert_eq!(attempts, (DEFAULT_PORT..=LAST_PORT).collect::<Vec<_>>());
        assert!(!directory.exists());
    }
}
