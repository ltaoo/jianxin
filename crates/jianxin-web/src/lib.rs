use anyhow::Result;
use std::{
    path::Path,
    sync::{atomic::AtomicBool, mpsc::Sender, Arc},
};
mod web;
pub use web::{message_url, resolve_message, serve, serve_desktop};

/// Platform actions supplied by the host; the HTTP library never starts a CLI.
pub trait RuntimeHooks: Send + Sync {
    /// Return true after the host has finished refreshing exactly this account.
    fn refresh_account(&self, _account_id: &str) -> Result<bool> {
        Ok(false)
    }
    fn config_changed(&self, _config: &Path, _data: &Path) -> Result<bool> {
        Ok(false)
    }
    fn notify(&self, _payload: &serde_json::Value) -> Result<()> {
        anyhow::bail!("system notifications unavailable")
    }
    fn mcp_command(&self) -> Option<serde_json::Value> {
        None
    }
}
pub struct NoHooks;
impl RuntimeHooks for NoHooks {}

#[derive(Clone)]
pub struct ServiceControl {
    pub shutdown: Arc<AtomicBool>,
    pub ready: Option<Sender<String>>,
    pub runtime: Arc<dyn RuntimeHooks>,
}
impl Default for ServiceControl {
    fn default() -> Self {
        Self {
            shutdown: Arc::new(AtomicBool::new(false)),
            ready: None,
            runtime: Arc::new(NoHooks),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpStream,
        sync::{atomic::Ordering, mpsc},
        time::Duration,
    };
    #[test]
    fn mailbox_refresh_http_requires_post_and_request_marker_and_targets_only_one_account() {
        #[derive(Default)]
        struct Hooks(std::sync::Mutex<Vec<String>>);
        impl RuntimeHooks for Hooks {
            fn refresh_account(&self, account_id: &str) -> Result<bool> {
                self.0.lock().unwrap().push(account_id.into());
                Ok(true)
            }
        }
        let config = serde_json::from_value(serde_json::json!({"accounts":[
            {"email":"first@example.com","auth":{"type":"password","password":"unused"}},
            {"email":"second@example.com","auth":{"type":"password","password":"unused"}}
        ]}))
        .unwrap();
        let app = jianxin_core::App::with_database(
            config,
            jianxin_store::Database::open_in_memory().unwrap(),
        )
        .unwrap();
        let account = app.account_configs()[1].to_account().id;
        let hooks = Arc::new(Hooks::default());
        let (ready, address) = mpsc::channel();
        let control = ServiceControl {
            ready: Some(ready),
            runtime: hooks.clone(),
            ..Default::default()
        };
        let shutdown = control.shutdown.clone();
        let worker = std::thread::spawn(move || {
            serve(
                app,
                Path::new("/unused-test-config"),
                None,
                0,
                None,
                false,
                0,
                control,
            )
        });
        let url = address.recv_timeout(Duration::from_secs(5)).unwrap();
        let host = url.strip_prefix("http://").unwrap().trim_end_matches('/');
        let mut responses = Vec::new();
        for (method, marker) in [
            ("GET", ""),
            ("POST", ""),
            ("POST", "X-Jianxin-Request: 1\r\n"),
        ] {
            let mut stream = TcpStream::connect(host).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            write!(stream, "{method} /api/mailboxes/refresh?account={account} HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\n{marker}Content-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            responses.push(response);
        }
        shutdown.store(true, Ordering::SeqCst);
        worker.join().unwrap().unwrap();
        for (response, status) in responses.iter().zip([405, 403, 200]) {
            assert!(
                response.starts_with(&format!("HTTP/1.1 {status}")),
                "{response}"
            );
        }
        assert_eq!(*hooks.0.lock().unwrap(), vec![account]);
    }

    #[test]
    fn desktop_service_reuses_its_url_after_shutdown_and_restart() {
        let directory =
            std::env::temp_dir().join(format!("jianxin-web-restart-{}", uuid::Uuid::new_v4()));
        let mut addresses = Vec::new();
        for _ in 0..2 {
            let mut config = jianxin_config::AppConfig::default();
            config.general.data_dir = Some(directory.clone());
            let app = jianxin_core::App::with_database(
                config,
                jianxin_store::Database::open_in_memory().unwrap(),
            )
            .unwrap();
            let (ready, address) = mpsc::channel();
            let control = ServiceControl {
                ready: Some(ready),
                ..Default::default()
            };
            let shutdown = control.shutdown.clone();
            let worker = std::thread::spawn(move || {
                serve_desktop(app, Path::new("/unused-test-config"), control)
            });
            let url = address.recv_timeout(Duration::from_secs(5)).unwrap();
            let host = url.strip_prefix("http://").unwrap().trim_end_matches('/');
            let mut stream = TcpStream::connect(host).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            write!(
                stream,
                "GET /api/mailboxes HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            drop(stream);
            shutdown.store(true, Ordering::SeqCst);
            worker.join().unwrap().unwrap();
            assert!(response.starts_with("HTTP/1.1 200"));
            let parsed = ammonia::url::Url::parse(&url).unwrap();
            assert_eq!(parsed.host_str(), Some("127.0.0.1"));
            assert!((65250..=65265).contains(&parsed.port().unwrap()));
            addresses.push(url);
        }
        assert_eq!(addresses[0], addresses[1]);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn embedded_service_reports_address_serves_reader_and_stops_without_a_child_process() {
        let config = jianxin_config::AppConfig::default();
        let app = jianxin_core::App::with_database(
            config,
            jianxin_store::Database::open_in_memory().unwrap(),
        )
        .unwrap();
        let (ready, address) = mpsc::channel();
        let control = ServiceControl {
            ready: Some(ready),
            ..Default::default()
        };
        let shutdown = control.shutdown.clone();
        let worker = std::thread::spawn(move || {
            serve(
                app,
                Path::new("/unused-test-config"),
                None,
                0,
                None,
                false,
                0,
                control,
            )
        });
        let url = address.recv_timeout(Duration::from_secs(5)).unwrap();
        let host = url.strip_prefix("http://").unwrap().trim_end_matches('/');
        let mut stream = TcpStream::connect(host).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        write!(
            stream,
            "GET /api/agent HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        shutdown.store(true, Ordering::SeqCst);
        worker.join().unwrap().unwrap();
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.contains("\"mcp\":null"));
        assert!(!response.contains("\"args\":[\"agent\"]"));
    }
}
