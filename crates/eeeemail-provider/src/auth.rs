//! Authentication providers.
//!
//! Authentication is deliberately **not** owned by the mail provider. The
//! combinations are a cross product:
//!
//! ```text
//! Password | AppPassword | OAuth2      x     IMAP | SMTP | Graph | Gmail API
//! ```
//!
//! so `AuthProvider` is injected into transports instead of being a method on
//! them. Adding Yahoo OAuth or enterprise SSO later must not touch the mail
//! domain at all.

use eeeemail_config::AuthConfig;
use eeeemail_domain::{AuthType, MailError, MailErrorCode, Result};
use std::fmt::Write as _;

pub trait AuthProvider: Send + Sync {
    fn kind(&self) -> AuthType;

    fn username(&self) -> &str;

    /// Password for `LOGIN` / `PLAIN`.
    fn password(&self) -> Result<String>;

    /// Bearer token when this is an OAuth2 session.
    fn access_token(&self) -> Result<Option<String>>;

    /// What to hand to the IMAP client.
    ///
    /// OAuth2 uses `AUTHENTICATE XOAUTH2`, so the transport asks for the raw
    /// token here and wraps it itself; everything else is a plain password.
    fn imap_secret(&self) -> Result<AuthSecret> {
        match self.access_token()? {
            Some(token) => Ok(AuthSecret::OAuth2(token)),
            None => Ok(AuthSecret::Password(self.password()?)),
        }
    }
}

/// Either a plain password or an OAuth2 bearer token.
#[derive(Debug, Clone)]
pub enum AuthSecret {
    Password(String),
    OAuth2(String),
}

/// Build the SASL XOAUTH2 initial client response.
///
/// ```text
/// user=<email>^Aauth=Bearer <token>^A^A   -> base64
/// ```
pub fn xoauth2_payload(user: &str, token: &str) -> String {
    use base64::Engine;
    let raw = format!("user={user}\u{1}auth=Bearer {token}\u{1}\u{1}");
    base64::engine::general_purpose::STANDARD.encode(raw)
}

/// Adapter turning the JSON `auth` block into an `AuthProvider`.
///
/// Resolution is lazy so a broken secret in one account does not stop the other
/// accounts from loading, and so `token_command` is only executed right before a
/// connection is opened.
pub struct ConfigAuth {
    username: String,
    config: AuthConfig,
}

impl ConfigAuth {
    pub fn new(username: impl Into<String>, config: AuthConfig) -> Self {
        Self {
            username: username.into(),
            config,
        }
    }
}

impl AuthProvider for ConfigAuth {
    fn kind(&self) -> AuthType {
        self.config.auth_type()
    }

    fn username(&self) -> &str {
        &self.username
    }

    fn password(&self) -> Result<String> {
        self.config.password()
    }

    fn access_token(&self) -> Result<Option<String>> {
        match self.config.auth_type() {
            AuthType::OAuth2 => Ok(Some(self.config.access_token()?)),
            _ => Ok(None),
        }
    }
}

/// Escape a string for use inside an IMAP quoted string.
pub fn imap_quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        if ch == '"' || ch == '\\' {
            out.push('\\');
        }
        let _ = out.write_char(ch);
    }
    out.push('"');
    out
}

/// Map an `AuthType` onto the SMTP mechanisms to try, in order.
pub fn smtp_mechanism(kind: AuthType) -> &'static [&'static str] {
    match kind {
        AuthType::OAuth2 => &["XOAUTH2"],
        // Office 365 in particular rejects PLAIN-only clients sometimes; LOGIN
        // is obsolete but harmless to offer second.
        _ => &["PLAIN", "LOGIN"],
    }
}

pub fn missing_credential(what: &str) -> MailError {
    MailError::new(
        MailErrorCode::AuthenticationRequired,
        format!("missing credential: {what}"),
    )
}
