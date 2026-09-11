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

use eeeemail_config::{AccountConfig, AuthConfig};
use eeeemail_domain::{AuthType, MailError, MailErrorCode, Result};
use serde::Deserialize;
use std::fmt::Write as _;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Mutex;
use std::thread::sleep;
use std::time::{Duration, Instant};

const TOKEN_REFRESH_MARGIN: Duration = Duration::from_secs(60);
const TOKEN_HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const OUTLOOK_IMAP_SMTP_SCOPES: &str = "offline_access https://outlook.office.com/IMAP.AccessAsUser.All https://outlook.office.com/SMTP.Send";

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
    refresh_token_source: Option<(PathBuf, String)>,
    oauth_tokens: Mutex<OAuthTokens>,
}

impl ConfigAuth {
    pub fn from_account(account: &AccountConfig) -> Self {
        Self {
            username: account.username(),
            config: account.auth.clone(),
            refresh_token_source: account
                .source_path
                .clone()
                .map(|path| (path, account.email.clone())),
            oauth_tokens: Mutex::new(OAuthTokens::default()),
        }
    }
}

#[derive(Default)]
struct OAuthTokens {
    access_token: Option<CachedToken>,
    refresh_token: Option<String>,
}

struct CachedToken {
    value: String,
    expires_at: Instant,
}

impl CachedToken {
    fn is_fresh(&self) -> bool {
        self.expires_at
            .checked_duration_since(Instant::now())
            .is_some_and(|remaining| remaining > TOKEN_REFRESH_MARGIN)
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    user_code: String,
    device_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

/// What the user must enter in their browser.
#[derive(Debug)]
pub struct DeviceCodePrompt {
    pub user_code: String,
    pub verification_uri: String,
}

/// The stored half of a completed device-code login.
#[derive(Debug)]
pub struct OAuthLogin {
    pub refresh_token: String,
}

pub fn login_device_code(
    account: &AccountConfig,
    prompt: impl FnOnce(&DeviceCodePrompt),
) -> Result<OAuthLogin> {
    let AuthConfig::OAuth2 {
        client_id,
        client_secret,
        token_endpoint,
        scopes,
        ..
    } = &account.auth
    else {
        return Err(MailError::new(
            MailErrorCode::InvalidConfig,
            "account is not configured for oauth2",
        ));
    };
    let (Some(client_id), Some(token_endpoint)) = (client_id.as_deref(), token_endpoint.as_deref())
    else {
        return Err(MailError::new(
            MailErrorCode::InvalidConfig,
            "oauth2 login needs `client_id` and `token_endpoint`",
        ));
    };
    let Some(device_endpoint) = device_endpoint(token_endpoint) else {
        return Err(MailError::new(
            MailErrorCode::InvalidConfig,
            "oauth2 `token_endpoint` must end with /token",
        ));
    };
    let scopes = scopes
        .as_ref()
        .map(|values| values.join(" "))
        .unwrap_or_else(|| default_scopes(account));

    let mut start_form = vec![
        ("client_id", client_id.to_string()),
        ("scope", scopes.clone()),
    ];
    if let Some(secret) = client_secret {
        start_form.push(("client_secret", secret.resolve()?));
    }
    let started: DeviceCodeResponse =
        serde_json::from_str(&post_form(&device_endpoint, &start_form)?).map_err(|e| {
            MailError::new(
                MailErrorCode::AuthenticationFailed,
                format!("invalid oauth2 device-code response: {e}"),
            )
        })?;
    prompt(&DeviceCodePrompt {
        user_code: started.user_code,
        verification_uri: started.verification_uri,
    });

    let mut interval = Duration::from_secs(started.interval.max(1));
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(started.expires_in))
        .unwrap_or_else(Instant::now);
    loop {
        sleep(interval);
        let mut form = vec![
            (
                "grant_type",
                "urn:ietf:params:oauth:grant-type:device_code".to_string(),
            ),
            ("client_id", client_id.to_string()),
            ("device_code", started.device_code.clone()),
        ];
        if let Some(secret) = client_secret {
            form.push(("client_secret", secret.resolve()?));
        }

        match post_form_status(token_endpoint, &form) {
            Ok(body) => {
                let response: TokenResponse = serde_json::from_str(&body).map_err(|e| {
                    MailError::new(
                        MailErrorCode::AuthenticationFailed,
                        format!("invalid oauth2 token response: {e}"),
                    )
                })?;
                return Ok(OAuthLogin {
                    refresh_token: response.refresh_token.ok_or_else(|| {
                        MailError::new(
                            MailErrorCode::AuthenticationFailed,
                            "oauth2 server did not return a refresh_token",
                        )
                    })?,
                });
            }
            Err((error, _description)) if error == "authorization_pending" => {}
            Err((error, _description)) if error == "slow_down" => {
                interval += Duration::from_secs(5);
            }
            Err((error, description)) => {
                return Err(MailError::new(
                    MailErrorCode::AuthenticationFailed,
                    format!("oauth2 device login failed: {error}: {description}"),
                ));
            }
        }
        if Instant::now() >= deadline {
            return Err(MailError::new(
                MailErrorCode::AuthenticationFailed,
                "oauth2 device code expired",
            ));
        }
    }
}

fn default_scopes(account: &AccountConfig) -> String {
    if crate::microsoft::matches(&account.email) {
        OUTLOOK_IMAP_SMTP_SCOPES.to_string()
    } else {
        "offline_access".to_string()
    }
}

fn device_endpoint(token_endpoint: &str) -> Option<String> {
    let base = token_endpoint.strip_suffix("/token")?;
    Some(format!("{base}/devicecode"))
}

fn post_form(endpoint: &str, form: &[(&str, String)]) -> Result<String> {
    post_form_status(endpoint, form).map_err(|(error, description)| {
        MailError::new(
            MailErrorCode::AuthenticationFailed,
            format!("oauth2 request failed: {error}: {description}"),
        )
    })
}

fn post_form_status(
    endpoint: &str,
    form: &[(&str, String)],
) -> std::result::Result<String, (String, String)> {
    let form = form
        .iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect::<Vec<_>>();
    let result = ureq::AgentBuilder::new()
        .timeout_connect(TOKEN_HTTP_TIMEOUT)
        .timeout_read(TOKEN_HTTP_TIMEOUT)
        .timeout_write(TOKEN_HTTP_TIMEOUT)
        .build()
        .post(endpoint)
        .send_form(&form);

    let response = match result {
        Ok(response) => response,
        Err(ureq::Error::Status(_, response)) => response,
        Err(e) => return Err(("transport".to_string(), e.to_string())),
    };
    let mut body = String::new();
    response
        .into_reader()
        .take(64 * 1024)
        .read_to_string(&mut body)
        .map_err(|e| ("transport".to_string(), e.to_string()))?;
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&body) {
        if let Some(error) = value.get("error").and_then(|v| v.as_str()) {
            let description = value
                .get("error_description")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            return Err((error.to_string(), description.to_string()));
        }
    }
    Ok(body)
}

fn refresh_access_token(config: &AuthConfig, old_refresh_token: &str) -> Result<TokenResponse> {
    let AuthConfig::OAuth2 {
        client_id,
        client_secret,
        token_endpoint,
        ..
    } = config
    else {
        return Err(MailError::new(
            MailErrorCode::InvalidConfig,
            "account is not configured for oauth2",
        ));
    };
    let (client_id, token_endpoint) = (client_id.as_deref(), token_endpoint.as_deref());
    let (Some(client_id), Some(token_endpoint)) = (client_id, token_endpoint) else {
        return Err(MailError::new(
            MailErrorCode::InvalidConfig,
            "oauth2 refresh needs `client_id` and `token_endpoint`",
        ));
    };

    let mut form = vec![
        ("grant_type", "refresh_token".to_string()),
        ("client_id", client_id.to_string()),
        ("refresh_token", old_refresh_token.to_string()),
    ];
    if let Some(secret) = client_secret {
        form.push(("client_secret", secret.resolve()?));
    }
    let body = post_form(token_endpoint, &form)?;
    serde_json::from_str::<TokenResponse>(&body).map_err(|e| {
        MailError::new(
            MailErrorCode::AuthenticationFailed,
            format!("oauth2 token response was invalid: {e}"),
        )
    })
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
            AuthType::OAuth2 => Ok(Some(self.oauth_access_token()?)),
            _ => Ok(None),
        }
    }
}

impl ConfigAuth {
    fn oauth_access_token(&self) -> Result<String> {
        let mut tokens = self.oauth_tokens.lock().map_err(poisoned_lock)?;
        if let Some(token) = &tokens.access_token {
            if token.is_fresh() {
                return Ok(token.value.clone());
            }
        }

        if let AuthConfig::OAuth2 {
            client_id: Some(_),
            refresh_token: Some(refresh_ref),
            token_endpoint: Some(_),
            ..
        } = &self.config
        {
            let refresh_token = match &tokens.refresh_token {
                Some(token) => token.clone(),
                None => refresh_ref.resolve()?,
            };
            let response = refresh_access_token(&self.config, &refresh_token)?;
            tokens.access_token = Some(CachedToken {
                value: response.access_token,
                expires_at: token_expiry(response.expires_in),
            });
            if let Some(rotated) = response.refresh_token {
                store_rotated_refresh_token(
                    refresh_ref,
                    self.refresh_token_source.as_ref(),
                    &rotated,
                )?;
                tokens.refresh_token = Some(rotated);
            }
            return Ok(tokens
                .access_token
                .as_ref()
                .expect("token was just cached")
                .value
                .clone());
        }

        self.config.access_token()
    }
}

fn token_expiry(expires_in: u64) -> Instant {
    let ttl = Duration::from_secs(expires_in).saturating_sub(Duration::from_secs(1));
    Instant::now().checked_add(ttl).unwrap_or_else(Instant::now)
}

fn store_rotated_refresh_token(
    source: &eeeemail_config::SecretRef,
    json_source: Option<&(PathBuf, String)>,
    token: &str,
) -> Result<()> {
    match source {
        eeeemail_config::SecretRef::File { file } => {
            if let Some(parent) = file.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    MailError::new(
                        MailErrorCode::AuthenticationFailed,
                        format!(
                            "cannot create OAuth token directory {}: {e}",
                            parent.display()
                        ),
                    )
                })?;
            }
            std::fs::write(file, token).map_err(|e| {
                MailError::new(
                    MailErrorCode::AuthenticationFailed,
                    format!(
                        "cannot store rotated OAuth refresh token {}: {e}",
                        file.display()
                    ),
                )
            })?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600)).map_err(
                    |e| {
                        MailError::new(
                            MailErrorCode::AuthenticationFailed,
                            format!("cannot secure OAuth refresh token {}: {e}", file.display()),
                        )
                    },
                )?;
            }
        }
        eeeemail_config::SecretRef::Plain(_) => {
            let Some((path, email)) = json_source else {
                return Ok(());
            };
            // ponytail: read-modify-write without a lock; add file locking if
            // concurrent syncs rotate tokens for different accounts.
            eeeemail_config::AppConfig::store_refresh_token(path, email, token)?;
        }
        _ => {}
    }
    Ok(())
}

fn poisoned_lock<T>(_: std::sync::PoisonError<T>) -> MailError {
    MailError::new(
        MailErrorCode::Unknown,
        "oauth2 token state lock was poisoned",
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_the_device_code_endpoint() {
        assert_eq!(
            device_endpoint("https://login.microsoftonline.com/consumers/oauth2/v2.0/token")
                .as_deref(),
            Some("https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode")
        );
    }

    #[test]
    fn token_uses_a_refresh_margin() {
        let token = CachedToken {
            value: "token".to_string(),
            expires_at: Instant::now(),
        };

        assert!(!token.is_fresh());
    }

    #[test]
    fn writes_rotated_file_back_to_disk() {
        let path =
            std::env::temp_dir().join(format!("eeeemail-refresh-{}.txt", uuid::Uuid::new_v4()));
        let source = eeeemail_config::SecretRef::File { file: path.clone() };

        store_rotated_refresh_token(&source, None, "rotated").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "rotated");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn writes_rotated_json_token_back_to_disk() {
        let path =
            std::env::temp_dir().join(format!("eeeemail-refresh-{}.json", uuid::Uuid::new_v4()));
        let config = r#"{"accounts":[{"email":"a@example.com","auth":{"type":"oauth2","client_id":"c","refresh_token":"old","token_endpoint":"https://example.com/token"}}]}"#;
        std::fs::write(&path, config).unwrap();
        let source = eeeemail_config::SecretRef::Plain("old".to_string());
        let origin = (path.clone(), "a@example.com".to_string());

        store_rotated_refresh_token(&source, Some(&origin), "rotated").unwrap();

        let saved = eeeemail_config::AppConfig::load(&path).unwrap();
        assert!(matches!(
            &saved.accounts[0].auth,
            eeeemail_config::AuthConfig::OAuth2 {
                refresh_token: Some(eeeemail_config::SecretRef::Plain(token)),
                ..
            } if token == "rotated"
        ));
        let _ = std::fs::remove_file(path);
    }
}
