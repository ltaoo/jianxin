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

use jianxin_domain::{Account, AccountId, AuthType, MailError, MailErrorCode, ProviderId, Result};
use serde::{Deserialize, Serialize};
use std::io::Write;
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
    pub tray_show_unread_count: bool,
    pub sync: SyncConfig,
    pub watch: WatchConfig,
    pub compose: ComposeConfig,
    pub api: ApiConfig,
    pub agent: AgentConfig,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            data_dir: None,
            tray_show_unread_count: true,
            sync: SyncConfig::default(),
            watch: WatchConfig::default(),
            compose: ComposeConfig::default(),
            api: ApiConfig::default(),
            agent: AgentConfig::default(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentConfig {
    pub base_url: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<SecretRef>,
    pub timeout_secs: u64,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self { base_url: "https://api.openai.com/v1".into(), model: String::new(), api_key: None, timeout_secs: 60 }
    }
}

impl std::fmt::Debug for AgentConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("AgentConfig").field("model", &self.model).field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]")).finish_non_exhaustive()
    }
}

/// Credentials for fixed-path external APIs; absent/empty token disables access.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ApiConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<SecretRef>,
}

impl std::fmt::Debug for ApiConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("ApiConfig").field("token", &self.token.as_ref().map(|_| "[REDACTED]")).finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WatchConfig {
    /// Poll interval in seconds.
    pub interval_secs: u64,
    /// Optional HTTP POST endpoint receiving new-mail JSON events.
    pub webhook: Option<String>,
    /// Send a desktop notification for each new mail event.
    pub notify: bool,
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            interval_secs: 300,
            webhook: None,
            notify: false,
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
    /// Mailbox branding for the Web sidebar; independent of the transport provider.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    /// Force a provider: `generic-imap` | `google` | `microsoft`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderId>,
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
    /// Runtime-only origin of this account; used to persist rotated OAuth tokens.
    #[serde(skip)]
    pub source_path: Option<PathBuf>,
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
    pub fn account_id(&self, provider: ProviderId) -> AccountId {
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

    /// Provider key, see `jianxin_provider::registry::detect_provider`.
    pub fn provider_key(&self) -> ProviderId {
        if let Some(p) = &self.provider {
            return *p;
        }
        // An explicitly configured IMAP endpoint means "just do IMAP", even for
        // gmail.com / outlook.com addresses.
        if self.imap.is_some() || self.smtp.is_some() {
            return ProviderId::GenericImap;
        }
        if let Some(preset) = presets::detect(&self.email) {
            // Only route to a native API provider when OAuth2 is configured —
            // otherwise the password/app-password path must use IMAP.
            if preset.provider != ProviderId::GenericImap && self.auth_type() == AuthType::OAuth2 {
                return preset.provider;
            }
        }
        ProviderId::GenericImap
    }

    /// Materialise the local `Account` row (no secrets attached).
    pub fn to_account(&self) -> Account {
        let provider = self.provider_key();
        let mut account = Account::new(provider, &self.email, self.auth_type());
        account.id = self.account_id(provider);
        account.display_name = self.display_name.clone();
        account
    }

    /// Keep an OAuth mailbox before its first login, like the mobile client.
    /// Credential resolution belongs to the connection/login step.
    pub fn needs_authorization(&self) -> bool {
        let AuthConfig::OAuth2 { access_token, token_command, refresh_token, client_id, token_endpoint, .. } = &self.auth else {
            return false;
        };
        !access_token.as_ref().is_some_and(SecretRef::has_value)
            && !token_command.as_ref().is_some_and(|command| !command.trim().is_empty())
            && !(refresh_token.as_ref().is_some_and(SecretRef::has_value)
                && client_id.as_ref().is_some_and(|id| !id.trim().is_empty())
                && token_endpoint.as_ref().is_some_and(|endpoint| !endpoint.trim().is_empty()))
    }

    pub fn apply_oauth_defaults(&mut self) {
        let microsoft = self.provider == Some(ProviderId::Microsoft)
            || presets::detect(&self.email).is_some_and(|preset| preset.provider == ProviderId::Microsoft);
        let AuthConfig::OAuth2 { token_endpoint, scopes, .. } = &mut self.auth else { return; };
        let microsoft = microsoft || token_endpoint.as_deref().is_some_and(|endpoint| endpoint.starts_with("https://login.microsoftonline.com/"));
        if microsoft {
            if token_endpoint.as_ref().is_none_or(|endpoint| endpoint.trim().is_empty()) {
                *token_endpoint = Some("https://login.microsoftonline.com/consumers/oauth2/v2.0/token".into());
            }
            if scopes.is_none() {
                *scopes = Some(vec!["offline_access".into(), "https://outlook.office.com/IMAP.AccessAsUser.All".into(), "https://outlook.office.com/SMTP.Send".into()]);
            }
        }
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
    /// Supports a pre-provisioned access token, a command that prints one, or
    /// refresh credentials. The provider layer refreshes and caches tokens.
    #[serde(rename = "oauth2", alias = "o_auth2")]
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
        /// Space-separated OAuth scopes requested by `jianxin login`.
        #[serde(skip_serializing_if = "Option::is_none")]
        scopes: Option<Vec<String>>,
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
    /// `jianxin_provider::auth` wraps into an XOAUTH2 SASL payload.
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
                if let Some(cmd) = token_command.as_ref().filter(|cmd| !cmd.trim().is_empty()) {
                    return secret::run_command(cmd);
                }
                match access_token {
                    Some(token) if token.has_value() => token.resolve(),
                    _ => Err(MailError::new(
                        MailErrorCode::AuthenticationRequired,
                        "oauth2 account needs `access_token`, `token_command`, or `jianxin login`",
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
            AuthConfig::OAuth2 { .. } => {},
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

/// `$JIANXIN_CONFIG`, else `~/.config/jianxin/config.json`.
pub fn default_config_path() -> PathBuf {
    if let Ok(p) = std::env::var("JIANXIN_CONFIG") {
        return PathBuf::from(p);
    }
    let base = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config");
    base.join("jianxin").join("config.json")
}

pub fn default_data_dir() -> PathBuf {
    dirs::data_dir()
        .map(|base| base.join("jianxin"))
        .unwrap_or_else(|| PathBuf::from(".jianxin"))
}

impl AppConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path).map_err(|e| {
            MailError::new(
                MailErrorCode::InvalidConfig,
                format!("cannot read config {}: {e}", path.display()),
            )
        })?;
        let mut config: AppConfig = serde_json::from_str(&raw).map_err(|e| {
            MailError::new(
                MailErrorCode::InvalidConfig,
                format!("invalid config {}: {e}", path.display()),
            )
        })?;
        for account in &mut config.accounts {
            account.apply_oauth_defaults();
            account.source_path = Some(path.to_path_buf());
        }
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
        #[cfg(unix)]
        if path.exists() {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(
                |e| {
                    MailError::new(
                        MailErrorCode::InvalidConfig,
                        format!("cannot secure {}: {e}", path.display()),
                    )
                },
            )?;
        }
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path).map_err(|e| {
            MailError::new(
                MailErrorCode::InvalidConfig,
                format!("cannot write {}: {e}", path.display()),
            )
        })?;
        file.write_all(json.as_bytes()).map_err(|e| {
            MailError::new(
                MailErrorCode::InvalidConfig,
                format!("cannot write {}: {e}", path.display()),
            )
        })?;
        Ok(())
    }

    /// Replace one account's refresh token and save the whole JSON config.
    pub fn store_refresh_token(path: &Path, email: &str, token: &str) -> Result<()> {
        let mut config = AppConfig::load(path)?;
        let account = config
            .accounts
            .iter_mut()
            .find(|account| account.email == email)
            .ok_or_else(|| {
                MailError::new(
                    MailErrorCode::InvalidConfig,
                    format!("account `{email}` disappeared from {}", path.display()),
                )
            })?;
        let AuthConfig::OAuth2 { refresh_token, .. } = &mut account.auth else {
            return Err(MailError::new(
                MailErrorCode::InvalidConfig,
                format!("account `{email}` is not configured for oauth2"),
            ));
        };
        *refresh_token = Some(SecretRef::Plain(token.to_string()));
        config.save(path)
    }

    pub fn find(&self, selector: &str) -> Option<&AccountConfig> {
        self.accounts
            .iter()
            .find(|a| a.alias() == selector || a.email == selector)
    }

    /// Data directory for the SQLite database and attachment cache.
    pub fn data_dir(&self) -> PathBuf {
        self.general
            .data_dir
            .clone()
            .unwrap_or_else(default_data_dir)
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

/// A ready-to-write example config, used by `jianxin init`.
pub fn example_config() -> AppConfig {
    let json = serde_json::json!({
        "accounts": [
            {
                "id": "gmail",
                "email": "you@gmail.com",
                "platform": "gmail",
                "auth": { "type": "app_password", "password": { "keychain": "jianxin/gmail" } }
            },
            {
                "id": "qq",
                "email": "you@qq.com",
                "platform": "qq",
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
    fn tray_unread_count_defaults_to_visible_and_round_trips() {
        let old: AppConfig = serde_json::from_str("{}").unwrap();
        assert!(old.general.tray_show_unread_count);
        let config: AppConfig = serde_json::from_str(r#"{"general":{"tray_show_unread_count":false}}"#).unwrap();
        let restored: AppConfig = serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        assert!(!restored.general.tray_show_unread_count);
    }

    #[test]
    fn agent_configuration_is_optional_and_secrets_are_redacted() {
        let old:AppConfig=serde_json::from_str("{}").unwrap();assert!(old.general.agent.model.is_empty());
        let config:AppConfig=serde_json::from_str(r#"{"general":{"agent":{"model":"fixture-model","base_url":"https://private.invalid/v1","api_key":"private-model-key"}}}"#).unwrap();
        let debug=format!("{:?}",config.general.agent);assert!(!debug.contains("private-model-key"));assert!(!debug.contains("private.invalid"));
        let restored:AppConfig=serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();assert_eq!(restored.general.agent.model,"fixture-model");assert_eq!(restored.general.agent.api_key.unwrap().resolve().unwrap(),"private-model-key");
    }

    #[test]
    fn api_token_is_optional_round_trips_and_is_redacted_in_debug() {
        let old: AppConfig = serde_json::from_str(r#"{"accounts":[]}"#).unwrap();
        assert!(old.general.api.token.is_none());
        let config: AppConfig = serde_json::from_str(r#"{"general":{"api":{"token":"private-hook-token"}}}"#).unwrap();
        assert_eq!(config.general.api.token.as_ref().unwrap().resolve().unwrap(), "private-hook-token");
        assert!(!format!("{config:?}").contains("private-hook-token"));
        let restored: AppConfig = serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        assert_eq!(restored.general.api.token.unwrap().resolve().unwrap(), "private-hook-token");
    }

    #[test]
    fn example_config_is_valid() {
        let cfg = example_config();
        assert_eq!(cfg.accounts.len(), 3);
        assert_eq!(cfg.accounts[0].provider_key(), ProviderId::GenericImap);
        assert_eq!(cfg.accounts[0].platform.as_deref(), Some("gmail"));
        assert_eq!(cfg.accounts[1].platform.as_deref(), Some("qq"));
    }

    #[test]
    fn platform_is_optional_and_does_not_change_transport_or_account_identity() {
        let mut account = example_config().accounts.remove(0);
        let id = account.to_account().id;
        account.platform = Some("qq".into());
        assert_eq!(account.provider_key(), ProviderId::GenericImap);
        assert_eq!(account.to_account().id, id);
        let restored: AccountConfig = serde_json::from_str(&serde_json::to_string(&account).unwrap()).unwrap();
        assert_eq!(restored.platform.as_deref(), Some("qq"));
        let old: AccountConfig = serde_json::from_value(serde_json::json!({
            "email": "old@example.com", "auth": { "type": "password", "password": "fixture" }
        })).unwrap();
        assert!(old.platform.is_none());
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
    fn outlook_consumer_uses_official_smtp_host() {
        let cfg: AppConfig = serde_json::from_str(
            r#"{"accounts":[{"email":"a@outlook.com","auth":{"type":"oauth2","client_id":"c","refresh_token":"r","token_endpoint":"https://login.microsoftonline.com/consumers/oauth2/v2.0/token"}}]}"#,
        )
        .unwrap();
        let acc = &cfg.accounts[0];
        assert_eq!(acc.imap_server().host, "outlook.office365.com");
        assert_eq!(acc.smtp_server().host, "smtp-mail.outlook.com");
        assert_eq!(acc.smtp_server().port, 587);
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
        assert_eq!(acc.provider_key(), ProviderId::GenericImap);
    }

    #[test]
    fn refresh_credentials_are_sufficient_for_oauth2() {
        let cfg: AppConfig = serde_json::from_str(
            r#"{"accounts":[{
                "email":"funzm@outlook.com",
                "auth":{
                    "type":"oauth2",
                    "client_id":"client",
                    "refresh_token":"refresh",
                    "token_endpoint":"https://example.com/token",
                    "scopes":["offline_access"]
                }
            }]}"#,
        )
        .unwrap();

        cfg.accounts[0]
            .validate()
            .expect("refresh-only oauth2 is valid");
    }

    #[test]
    fn oauth2_login_credentials_are_sufficient_before_first_login() {
        let cfg: AppConfig = serde_json::from_str(
            r#"{"accounts":[{
                "email":"funzm@outlook.com",
                "auth":{
                    "type":"oauth2",
                    "client_id":"client",
                    "token_endpoint":"https://example.com/token"
                }
            }]}"#,
        )
        .unwrap();

        cfg.accounts[0]
            .validate()
            .expect("login-only oauth2 is valid");
    }

    #[test]
    fn unconfigured_oauth_mailboxes_load_before_login_and_outlook_gets_mobile_defaults() {
        let path = std::env::temp_dir().join(format!("jianxin-pending-oauth-{}.json", jianxin_domain::new_id()));
        std::fs::write(&path, r#"{"accounts":[
            {"email":"pending@outlook.com","auth":{"type":"oauth2"}},
            {"email":"other@example.com","auth":{"type":"o_auth2","access_token":""}},
            {"email":"user@company.example","auth":{"type":"oauth2","token_endpoint":"https://login.microsoftonline.com/common/oauth2/v2.0/token"}}
        ]}"#).unwrap();
        let config = AppConfig::load(&path).unwrap();
        assert_eq!(config.accounts.len(), 3);
        assert!(config.accounts.iter().all(AccountConfig::needs_authorization));
        let AuthConfig::OAuth2 { token_endpoint, scopes, client_id, .. } = &config.accounts[0].auth else { panic!("expected oauth2"); };
        assert_eq!(token_endpoint.as_deref(), Some("https://login.microsoftonline.com/consumers/oauth2/v2.0/token"));
        assert!(scopes.as_ref().unwrap().iter().any(|scope| scope == "offline_access"));
        assert!(client_id.is_none());
        let AuthConfig::OAuth2 { scopes, .. } = &config.accounts[2].auth else { panic!("expected oauth2"); };
        assert!(scopes.as_ref().unwrap().iter().any(|scope| scope == "https://outlook.office.com/IMAP.AccessAsUser.All"));
        assert_eq!(config.accounts[1].auth.access_token().unwrap_err().code, MailErrorCode::AuthenticationRequired);
        std::fs::write(&path, r#"{"accounts":[{"email":"missing@example.com","auth":{"type":"password","password":""}}]}"#).unwrap();
        assert!(AppConfig::load(&path).is_err());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn refresh_token_is_stored_in_the_selected_json_account() {
        let path = std::env::temp_dir().join(format!(
            "jianxin-config-{}.json",
            jianxin_domain::new_id()
        ));
        let config: AppConfig = serde_json::from_str(
            r#"{"accounts":[
                {"email":"a@example.com","auth":{"type":"oauth2","client_id":"a","token_endpoint":"https://a/token"}},
                {"email":"b@example.com","auth":{"type":"oauth2","client_id":"b","token_endpoint":"https://b/token"}}
            ]}"#,
        )
        .unwrap();
        config.save(&path).unwrap();

        AppConfig::store_refresh_token(&path, "b@example.com", "rotated").unwrap();

        let loaded = AppConfig::load(&path).unwrap();
        assert!(matches!(
            &loaded.accounts[1].auth,
            AuthConfig::OAuth2 { refresh_token: Some(SecretRef::Plain(token)), .. } if token == "rotated"
        ));
        assert!(matches!(
            &loaded.accounts[0].auth,
            AuthConfig::OAuth2 {
                refresh_token: None,
                ..
            }
        ));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = std::fs::remove_file(&path);
    }
}
