use anyhow::Result;
use std::{
    path::Path,
    sync::{atomic::AtomicBool, mpsc::Sender, Arc},
};
mod web;
pub use web::{message_url, resolve_message, serve};

/// Platform actions supplied by the host; the HTTP library never starts a CLI.
pub trait RuntimeHooks: Send + Sync {
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
