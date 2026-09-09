//! JSON account configuration.
//!
//! Design constraints from the architecture document:
//!
//! * Authentication is *not* owned by the mail provider, so auth lives next to
//!   the account, not inside an IMAP/Graph transport.
//! * Provider selection is multi-level: explicit config > email-domain match >
//!   generic IMAP fallback.
//! * Secrets never sit in the config file in plaintext by default — they are
//!   references into the environment, the OS keychain or an external command.

pub mod presets;
pub mod secret;

pub use presets::ProviderPreset;
pub use secret::SecretRef;

use eeeemail_domain::{Account, AccountId, AuthType, MailError, MailErrorCode, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Top level
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub general: GeneralConfig,
    pub accounts: Vec<AccountConfig>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            accounts: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    /// Overrides the platform data directory (SQLite db, attachment cache).
    pub data_dir: Option<PathBuf>,
    pub sync: SyncConfig,
    pub compose: ComposeConfig,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            data_dir: None,
            sync: SyncConfig::default(),
            compose: ComposeConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncConfig {
    /// How many messages to fetch per IMAP round trip.
    pub page_size: u32,
    /// Cap on how much history to pull during the initial sync of one folder.
    pub max_messages_per_folder: u32,
    /// Socket timeout in seconds.
    pub timeout_secs: u64,
    /// Folder roles to skip, e.g. `["spam", "trash"]`.
    pub skip_folders: Vec<String>,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            page_size: 50,
            max_messages_per_folder: 500,
            timeout_secs: 30,
            skip_folders: vec!["spam".into(), "trash".into()],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ComposeConfig {
    /// External editor used by the TUI. Defaults to `$VISUAL` / `$EDITOR`.
    pub editor: Option<String>,
    /// Signature appended to composed messages.
    pub signature: Option<String>,
}

impl Default for ComposeConfig {
    fn default() -> Self {
        Self {
            editor: None,
            signature: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Account
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountConfig {
    /// Short alias used on the CLI (`--account work`). Defaults to the email.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Force a provider: `generic-imap` | `google` | `microsoft`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    pub auth: AuthConfig,
    /// Explicit IMAP endpoint; merged over the provider preset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub imap: Option<ServerConfig>,
    /// Explicit SMTP endpoint; merged over the provider preset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub smtp: Option<ServerConfig>,
    /// Force folder roles when SPECIAL-USE and the name heuristic both fail.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folders: Option<FolderOverrides>,
}

impl AccountConfig {
    /// Alias used by `--account`.
    pub fn alias(&self) -> &str {
        self.id.as_deref().unwrap_or(&self.email)
    }

    pub fn auth_type(&self) -> AuthType {
        self.auth.auth_type()
    }

    /// Local account id, stable across restarts.
    pub fn account_id(&self, provider: &str) -> AccountId {
        Account::deterministic_id(provider, &self.email)
    }

    /// IMAP endpoint: explicit config wins, otherwise the domain preset,
    /// otherwise `imap.<domain>`.
    pub fn imap_server(&self) -> ServerConfig {
        let preset = presets::detect(&self.email);
        let mut base = self
            .imap
            .clone()
            .or_else(|| preset.map(|p| p.imap))
            .unwrap_or_else(|| ServerConfig::guess("imap", &self.email, 993, TlsMode::Tls));
        if base.username.is_none() {
            base.username = Some(self.username());
        }
        base
    }

    /// SMTP endpoint, resolved the same way as IMAP.
    pub fn smtp_server(&self) -> ServerConfig {
        let preset = presets::detect(&self.email);
        let mut base = self
            .smtp
            .clone()
            .or_else(|| preset.map(|p| p.smtp))
            .unwrap_or_else(|| ServerConfig::guess("smtp", &self.email, 465, TlsMode::Tls));
        if base.username.is_none() {
            base.username = Some(self.username());
        }
        base
    }

    /// Login name; defaults to the full email address.
    pub fn username(&self) -> String {
        self.auth
            .username()
            .map(|s| s.to_string())
            .unwrap_or_else(|| self.email.clone())
    }

    /// Provider key, see `eeeemail_provider::registry::detect_provider`.
    pub fn provider_key(&self) -> String {
        if let Some(p) = &self.provider {
            return p.clone();
        }
        // An explicitly configured IMAP endpoint means "just do IMAP", even for
        // gmail.com / outlook.com addresses.
        if self.imap.is_some() || self.smtp.is_some() {
            return "generic-imap".to_string();
        }
        if let Some(preset) = presets::detect(&self.email) {
            // Only route to a native API provider when OAuth2 is configured —
            // otherwise the password/app-password path must use IMAP.
            if preset.provider != "generic-imap" && self.auth_type() == AuthType::OAuth2 {
                return preset.provider.to_string();
            }
        }
        "generic-imap".to_string()
    }

    /// Materialise the local `Account` row (no secrets attached).
    pub fn to_account(&self) -> Account {
        let provider = self.provider_key();
        let mut account = Account::new(&provider, &self.email, self.auth_type());
        account.id = self.account_id(&provider);
        account.display_name = self.display_name.clone();
        account
    }

    pub fn validate(&self) -> Result<()> {
        if !self.email.contains('@') {
            return Err(MailError::new(
                MailErrorCode::InvalidConfig,
                format!("account `{}` has an invalid email address", self.alias()),
            ));
        }
        self.auth.validate(self.alias())?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TlsMode {
    /// Implicit TLS — connect with TLS immediately (IMAP 993, SMTP 465).
    Tls,
    /// Plain connection upgraded with STARTTLS (IMAP 143, SMTP 587).
    // `starttls` is the spelling most people actually type, and the protocol's
    // own name; accept both rather than rejecting a valid config file.
    #[serde(alias = "starttls")]
    StartTls,
    /// No encryption. Only for local relays.
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    #[serde(default = "default_tls")]
    pub tls: TlsMode,
    /// Login override. Defaults to the account email.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

fn default_tls() -> TlsMode {
    TlsMode::Tls
}

impl ServerConfig {
    pub fn new(host: &str, port: u16, tls: TlsMode) -> Self {
        Self {
            host: host.to_string(),
            port,
            tls,
            username: None,
        }
    }

    /// Last-resort endpoint guess used when no preset matches.
    pub fn guess(prefix: &str, email: &str, port: u16, tls: TlsMode) -> Self {
        let domain = email.rsplit('@').next().unwrap_or(email);
        Self::new(&format!("{prefix}.{domain}"), port, tls)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FolderOverrides {
    pub inbox: Option<String>,
    pub sent: Option<String>,
    pub drafts: Option<String>,
    pub trash: Option<String>,
    pub spam: Option<String>,
    pub archive: Option<String>,
}

// ---------------------------------------------------------------------------
// Authentication
// ---------------------------------------------------------------------------

/// Authentication is a dimension orthogonal to transport, so it is modelled on
/// its own: `Password | AppPassword | OAuth2` x `IMAP | SMTP | Graph | Gmail`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthConfig {
    Password {
        #[serde(skip_serializing_if = "Option::is_none")]
        username: Option<String>,
        password: SecretRef,
    },
    AppPassword {
        #[serde(skip_serializing_if = "Option::is_none")]
        username: Option<String>,
        password: SecretRef,
    },
    /// OAuth 2.0.
    ///
    /// Phase 1 ships the two shapes that work without a full
    /// authorization-code + PKCE daemon: a pre-provisioned access token, or a
    /// command that prints one. The redirect/refresh flow plugs into the same
    /// `AuthProvider` trait later without touching the mail layer.
    OAuth2 {
        #[serde(skip_serializing_if = "Option::is_none")]
        username: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        access_token: Option<SecretRef>,
        /// Command whose stdout is an access token (e.g. an `oauth2l` wrapper).
        #[serde(skip_serializing_if = "Option::is_none")]
        token_command: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        client_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        client_secret: Option<SecretRef>,
        #[serde(skip_serializing_if = "Option::is_none")]
        refresh_token: Option<SecretRef>,
        #[serde(skip_serializing_if = "Option::is_none")]
        token_endpoint: Option<String>,
    },
}

impl AuthConfig {
    pub fn auth_type(&self) -> AuthType {
        match self {
            AuthConfig::Password { .. } => AuthType::Password,
            AuthConfig::AppPassword { .. } => AuthType::AppPassword,
            AuthConfig::OAuth2 { .. } => AuthType::OAuth2,
        }
    }

    pub fn username(&self) -> Option<&str> {
        match self {
            AuthConfig::Password { username, .. }
            | AuthConfig::AppPassword { username, .. }
            | AuthConfig::OAuth2 { username, .. } => username.as_deref(),
        }
    }

    /// Resolve the IMAP/SMTP password. For OAuth2 this is the bearer token that
    /// `eeeemail_provider::auth` wraps into an XOAUTH2 SASL payload.
    pub fn password(&self) -> Result<String> {
        match self {
            AuthConfig::Password { password, .. } | AuthConfig::AppPassword { password, .. } => {
                password.resolve()
            }
            AuthConfig::OAuth2 { .. } => self.access_token(),
        }
    }

    pub fn access_token(&self) -> Result<String> {
        match self {
            AuthConfig::OAuth2 {
                access_token,
                token_command,
                ..
            } => {
                if let Some(cmd) = token_command {
                    return secret::run_command(cmd);
                }
                match access_token {
                    Some(token) => token.resolve(),
                    None => Err(MailError::new(
                        MailErrorCode::AuthenticationRequired,
                        "oauth2 account needs `access_token` or `token_command`",
                    )),
                }
            }
            _ => Err(MailError::new(
                MailErrorCode::InvalidConfig,
                "account is not configured for oauth2",
            )),
        }
    }

    fn validate(&self, alias: &str) -> Result<()> {
        match self {
            AuthConfig::Password { password, .. } | AuthConfig::AppPassword { password, .. } => {
                if !password.has_value() {
                    return Err(MailError::new(
                        MailErrorCode::InvalidConfig,
                        format!("account `{alias}` is missing `auth.password`"),
                    ));
                }
            }
            AuthConfig::OAuth2 {
                access_token,
                token_command,
                ..
            } => {
                let has_token = access_token.as_ref().is_some_and(|s| s.has_value());
                if token_command.is_none() && !has_token {
                    return Err(MailError::new(
                        MailErrorCode::InvalidConfig,
                        format!(
                            "account `{alias}` is oauth2 but has neither `access_token` nor `token_command`"
                        ),
                    ));
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

/// `$EEEEMAIL_CONFIG`, else `<config_dir>/eeeemail/config.json`.
pub fn default_config_path() -> PathBuf {
    if let Ok(p) = std::env::var("EEEEMAIL_CONFIG") {
        return PathBuf::from(p);
    }
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("~/.config"));
    base.join("eeeemail").join("config.json")
}

pub fn default_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from(".eeeemail"))
        .join("eeeemail")
}

impl AppConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path).map_err(|e| {
            MailError::new(
                MailErrorCode::InvalidConfig,
                format!("cannot read config {}: {e}", path.display()),
            )
        })?;
        let config: AppConfig = serde_json::from_str(&raw).map_err(|e| {
            MailError::new(
                MailErrorCode::InvalidConfig,
                format!("invalid config {}: {e}", path.display()),
            )
        })?;
        for account in &config.accounts {
            account.validate()?;
        }
        Ok(config)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                MailError::new(
                    MailErrorCode::InvalidConfig,
                    format!("cannot create {}: {e}", parent.display()),
                )
            })?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| MailError::new(MailErrorCode::InvalidConfig, e.to_string()))?;
        std::fs::write(path, json).map_err(|e| {
            MailError::new(
                MailErrorCode::InvalidConfig,
                format!("cannot write {}: {e}", path.display()),
            )
        })
    }

    pub fn find(&self, selector: &str) -> Option<&AccountConfig> {
        self.accounts
            .iter()
            .find(|a| a.alias() == selector || a.email == selector)
    }

    /// Data directory for the SQLite database and attachment cache.
    pub fn data_dir(&self) -> PathBuf {
        self.general.data_dir.clone().unwrap_or_else(default_data_dir)
    }

    /// External editor: explicit config, then `$VISUAL`, then `$EDITOR`.
    pub fn editor(&self) -> String {
        if let Some(e) = &self.general.compose.editor {
            return e.clone();
        }
        std::env::var("VISUAL")
            .or_else(|_| std::env::var("EDITOR"))
            .unwrap_or_else(|_| "vim".to_string())
    }
}

/// A ready-to-write example config, used by `eeeemail init`.
pub fn example_config() -> AppConfig {
    let json = serde_json::json!({
        "accounts": [
            {
                "id": "gmail",
                "email": "you@gmail.com",
                "auth": { "type": "app_password", "password": { "keychain": "eeeemail/gmail" } }
            },
            {
                "id": "qq",
                "email": "you@qq.com",
                "auth": { "type": "app_password", "password": { "env": "QQ_MAIL_PASSWORD" } }
            },
            {
                "id": "custom",
                "email": "you@example.com",
                "provider": "generic-imap",
                "auth": { "type": "password", "password": { "command": "security find-internet-password -s imap.example.com -w" } },
                "imap": { "host": "imap.example.com", "port": 993, "tls": "tls" },
                "smtp": { "host": "smtp.example.com", "port": 465, "tls": "tls" }
            }
        ]
    });
    serde_json::from_value(json).expect("example config must be valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_config_is_valid() {
        let cfg = example_config();
        assert_eq!(cfg.accounts.len(), 3);
        assert_eq!(cfg.accounts[0].provider_key(), "generic-imap");
    }

    #[test]
    fn presets_are_applied() {
        let cfg: AppConfig = serde_json::from_str(
            r#"{"accounts":[{"email":"a@gmail.com","auth":{"type":"app_password","password":"x"}}]}"#,
        )
        .unwrap();
        let acc = &cfg.accounts[0];
        assert_eq!(acc.imap_server().host, "imap.gmail.com");
        assert_eq!(acc.smtp_server().port, 465);
    }

    #[test]
    fn explicit_hosts_beat_presets() {
        let cfg: AppConfig = serde_json::from_str(
            r#"{"accounts":[{
                "email":"a@gmail.com",
                "auth":{"type":"app_password","password":"x"},
                "imap":{"host":"mail.internal","port":143,"tls":"starttls"}
            }]}"#,
        )
        .unwrap();
        let acc = &cfg.accounts[0];
        assert_eq!(acc.imap_server().host, "mail.internal");
        assert_eq!(acc.provider_key(), "generic-imap");
    }
}
