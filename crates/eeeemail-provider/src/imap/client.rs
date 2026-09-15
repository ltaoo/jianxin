use eeeemail_config::TlsMode;
use eeeemail_domain::{MailError, MailErrorCode, Result};
use imap::types::{Flag, NameAttribute};
use std::net::TcpStream;
use std::time::Duration;

use crate::auth::{xoauth2_payload, AuthProvider, AuthSecret};
use crate::imap::stream::{starttls, wrap_tls, MaybeTlsStream};

/// Where to reach an IMAP server.
#[derive(Debug, Clone)]
pub struct ImapEndpoint {
    pub host: String,
    pub port: u16,
    pub tls: TlsMode,
    pub timeout: Duration,
}

impl ImapEndpoint {
    pub fn new(host: &str, port: u16, tls: TlsMode, timeout: Duration) -> Self {
        Self {
            host: host.to_string(),
            port,
            tls,
            timeout,
        }
    }
}

/// A folder as reported by `LIST`, before domain mapping.
#[derive(Debug, Clone)]
pub struct RawFolder {
    pub name: String,
    /// Space-joined IMAP attributes, e.g. `\HasNoChildren \Sent`.
    pub attributes: String,
    pub delimiter: Option<String>,
}

/// A fetched message, still in its raw RFC 822 form.
#[derive(Debug, Clone)]
pub struct RawMessage {
    pub uid: u32,
    pub flags: Vec<String>,
    pub size: u64,
    pub raw: Vec<u8>,
}

/// `SELECT` response values that matter for syncing.
#[derive(Debug, Clone, Copy, Default)]
pub struct MailboxStatus {
    pub uid_validity: Option<u32>,
    pub uid_next: Option<u32>,
    pub exists: u32,
    pub unseen: Option<u32>,
}

/// A live IMAP session.
///
/// One instance per operation — connections are cheap to open and this keeps
/// everything stateless (and therefore trivially shareable across threads).
pub struct ImapClient {
    session: imap::Session<MaybeTlsStream>,
}

impl Drop for ImapClient {
    fn drop(&mut self) {
        let _ = self.session.logout();
    }
}

/// XOAUTH2 SASL mechanism for `AUTHENTICATE`.
struct Xoauth2 {
    payload: String,
}

impl imap::Authenticator for Xoauth2 {
    type Response = String;

    fn process(&self, _challenge: &[u8]) -> String {
        self.payload.clone()
    }
}

impl ImapClient {
    pub fn connect(endpoint: &ImapEndpoint, auth: &dyn AuthProvider) -> Result<Self> {
        let addr = (endpoint.host.as_str(), endpoint.port);
        let tcp = TcpStream::connect(&addr).map_err(|e| {
            MailError::new(
                MailErrorCode::NetworkUnavailable,
                format!("cannot connect to {}:{}: {e}", endpoint.host, endpoint.port),
            )
        })?;
        tcp.set_read_timeout(Some(endpoint.timeout)).ok();
        tcp.set_write_timeout(Some(endpoint.timeout)).ok();

        let (stream, greeting_consumed) = match endpoint.tls {
            TlsMode::Tls => (wrap_tls(tcp, &endpoint.host)?, false),
            TlsMode::StartTls => (starttls(tcp, &endpoint.host)?, true),
            TlsMode::None => (MaybeTlsStream::Plain(tcp), false),
        };

        let mut client = imap::Client::new(stream);
        if !greeting_consumed {
            client.read_greeting().map_err(imap_err)?;
        }

        let session = match auth.imap_secret()? {
            AuthSecret::Password(password) => client
                .login(auth.username(), password)
                .map_err(|(e, _)| imap_err(e)),
            AuthSecret::OAuth2(token) => {
                let authenticator = Xoauth2 {
                    payload: xoauth2_payload(auth.username(), &token),
                };
                client
                    .authenticate("XOAUTH2", &authenticator)
                    .map_err(|(e, _)| imap_err(e))
            }
        }?;

        Ok(Self { session })
    }

    // --- folders ----------------------------------------------------------

    pub fn list_folders(&mut self) -> Result<Vec<RawFolder>> {
        let names = self.session.list(None, Some("*")).map_err(imap_err)?;
        let mut folders = Vec::with_capacity(names.len());
        for name in names.iter() {
            let attributes = name
                .attributes()
                .iter()
                .map(attribute_name)
                .collect::<Vec<_>>()
                .join(" ");
            folders.push(RawFolder {
                name: name.name().to_string(),
                attributes,
                delimiter: name.delimiter().map(|d| d.to_string()),
            });
        }
        Ok(folders)
    }

    /// `SELECT` a folder and report its sync-relevant state.
    pub fn select(&mut self, folder: &str) -> Result<MailboxStatus> {
        let mailbox = self.session.select(folder).map_err(imap_err)?;
        Ok(MailboxStatus {
            uid_validity: mailbox.uid_validity,
            uid_next: mailbox.uid_next,
            exists: mailbox.exists,
            unseen: mailbox.unseen,
        })
    }

    // --- reading ----------------------------------------------------------

    /// Whether the server advertises the given capability atom, e.g.
    /// `THREAD=REFERENCES` (RFC 5256) or `X-GM-EXT-1` (Gmail extensions).
    /// Servers see an unknown atom as a missing one, so this is safe to call
    /// speculatively.
    pub fn has_capability(&mut self, name: &str) -> bool {
        self.session
            .capabilities()
            .map(|caps| caps.has_str(name))
            .unwrap_or(false)
    }

    /// `UID SEARCH <criteria>` as a sorted uid list.
    pub fn uid_search(&mut self, criteria: &str) -> Result<Vec<u32>> {
        let uids = self.session.uid_search(criteria).map_err(imap_err)?;
        let mut out: Vec<u32> = uids.into_iter().collect();
        out.sort_unstable();
        Ok(out)
    }

    /// Fetch full bodies for a uid set, e.g. `"1:50"` or `"7,9,12"`.
    ///
    /// `BODY.PEEK[]` is used instead of `RFC822` so that reading a message never
    /// flips `\Seen` as a side effect.
    pub fn fetch_messages(&mut self, uid_set: &str) -> Result<Vec<RawMessage>> {
        if uid_set.is_empty() {
            return Ok(Vec::new());
        }
        let fetch = self
            .session
            .uid_fetch(uid_set, "(FLAGS RFC822.SIZE BODY.PEEK[])")
            .map_err(imap_err)?;

        let mut out = Vec::new();
        for item in fetch.iter() {
            let uid = item.uid.unwrap_or(0);
            out.push(RawMessage {
                uid,
                flags: item.flags().iter().map(|f| f.to_string()).collect(),
                size: item.size.map(u64::from).unwrap_or(0),
                raw: item.body().unwrap_or(&[]).to_vec(),
            });
        }
        out.sort_by_key(|m| m.uid);
        Ok(out)
    }

    /// Flags only — cheap enough to run over an entire folder during sync.
    pub fn fetch_flags(&mut self, uid_set: &str) -> Result<Vec<(u32, Vec<String>)>> {
        if uid_set.is_empty() {
            return Ok(Vec::new());
        }
        let fetch = self.session.uid_fetch(uid_set, "FLAGS").map_err(imap_err)?;
        let mut out = Vec::new();
        for item in fetch.iter() {
            out.push((
                item.uid.unwrap_or(0),
                item.flags().iter().map(|f| f.to_string()).collect(),
            ));
        }
        out.sort_by_key(|(uid, _)| *uid);
        Ok(out)
    }

    // --- mutation ---------------------------------------------------------

    pub fn set_read(&mut self, uid: u32, read: bool) -> Result<()> {
        let flag = if read { "+FLAGS" } else { "-FLAGS" };
        self.session
            .uid_store(uid.to_string(), format!("{flag} (\\Seen)"))
            .map_err(imap_err)?;
        Ok(())
    }

    pub fn set_flagged(&mut self, uid: u32, flagged: bool) -> Result<()> {
        let flag = if flagged { "+FLAGS" } else { "-FLAGS" };
        self.session
            .uid_store(uid.to_string(), format!("{flag} (\\Flagged)"))
            .map_err(imap_err)?;
        Ok(())
    }

    /// Copy to another folder. Used by `move` (see `delete`) and by Drafts.
    pub fn copy(&mut self, uid: u32, destination: &str) -> Result<()> {
        self.session
            .uid_copy(uid.to_string(), destination)
            .map_err(imap_err)?;
        Ok(())
    }

    /// Mark `\Deleted` and expunge.
    pub fn delete(&mut self, uid: u32) -> Result<()> {
        self.session
            .uid_store(uid.to_string(), "+FLAGS (\\Deleted)")
            .map_err(imap_err)?;
        self.session.expunge().map_err(imap_err)?;
        Ok(())
    }

    /// Append a raw message — the IMAP route for creating drafts and for
    /// storing a sent copy when the server has no "append to Sent" via SMTP.
    pub fn append(&mut self, folder: &str, raw: &[u8], seen: bool) -> Result<()> {
        let mut flags: Vec<Flag> = vec![Flag::Draft];
        if seen {
            flags.push(Flag::Seen);
        }
        self.session
            .append_with_flags(folder, raw, &flags)
            .map_err(imap_err)?;
        Ok(())
    }

    pub fn close(&mut self) -> Result<()> {
        self.session.close().map_err(imap_err)
    }
}

fn attribute_name(attribute: &NameAttribute) -> String {
    match attribute {
        NameAttribute::NoInferiors => "\\Noinferiors".to_string(),
        NameAttribute::NoSelect => "\\Noselect".to_string(),
        NameAttribute::Marked => "\\Marked".to_string(),
        NameAttribute::Unmarked => "\\Unmarked".to_string(),
        NameAttribute::Custom(value) => value.to_string(),
    }
}

/// Translate an `imap` crate error into the unified error model.
///
/// The UI must never see `Error::No("…")`; a login rejection has to become
/// `AuthenticationFailed` and an unreachable host `NetworkUnavailable`.
pub fn imap_err(err: imap::Error) -> MailError {
    match &err {
        imap::Error::No(text) | imap::Error::Bad(text) => {
            let lower = text.to_ascii_lowercase();
            let code = if lower.contains("authentication")
                || lower.contains("authenticate")
                || lower.contains("invalid credentials")
                || lower.contains("login")
                || lower.contains("log in")
                || lower.contains("password")
                || lower.contains("too many")
            {
                MailErrorCode::AuthenticationFailed
            } else if lower.contains("permission") || lower.contains("denied") {
                MailErrorCode::PermissionDenied
            } else if lower.contains("not found") || lower.contains("doesn't exist") {
                MailErrorCode::FolderNotFound
            } else {
                MailErrorCode::ProviderUnavailable
            };
            MailError::new(code, text.clone())
        }
        imap::Error::Io(io) => MailError::new(
            if io.kind() == std::io::ErrorKind::TimedOut {
                MailErrorCode::Timeout
            } else {
                MailErrorCode::NetworkUnavailable
            },
            format!("imap i/o error: {io}"),
        ),
        imap::Error::ConnectionLost => MailError::new(
            MailErrorCode::NetworkUnavailable,
            "imap connection lost".to_string(),
        ),
        imap::Error::TlsHandshake(_) | imap::Error::Tls(_) => MailError::new(
            MailErrorCode::NetworkUnavailable,
            format!("imap tls error: {err}"),
        ),
        imap::Error::Parse(_) => MailError::new(
            MailErrorCode::ProviderUnavailable,
            format!("cannot parse imap response: {err}"),
        ),
        _ => MailError::new(MailErrorCode::Unknown, format!("imap error: {err}")),
    }
}
