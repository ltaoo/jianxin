use serde::{Deserialize, Serialize};

use crate::ProviderId;

/// Local account id (UUID). Never reuse a provider id for this.
pub type AccountId = String;

/// How the account authenticates. Authentication is a separate axis from mail
/// transport — see "Authentication 不应该和 Mail Provider 强绑定".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthType {
    /// Plain account password.
    Password,
    /// Provider-issued application password (Gmail, QQ, 163, …).
    AppPassword,
    /// OAuth 2.0 bearer token (Graph, Gmail API, IMAP XOAUTH2).
    OAuth2,
}

impl AuthType {
    pub fn as_str(&self) -> &'static str {
        match self {
            AuthType::Password => "password",
            AuthType::AppPassword => "app_password",
            AuthType::OAuth2 => "oauth2",
        }
    }
}

impl std::fmt::Display for AuthType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A configured mailbox.
///
/// Credentials are deliberately *not* part of this struct: the account row only
/// carries a reference, secrets live in the OS keychain / env / secret command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: AccountId,
    /// Provider key, e.g. `generic-imap`, `google`, `microsoft`.
    pub provider: ProviderId,
    pub email: String,
    pub display_name: Option<String>,
    pub auth_type: AuthType,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Account {
    pub fn new(provider: ProviderId, email: impl Into<String>, auth: AuthType) -> Self {
        let now = crate::now_ts();
        Self {
            id: crate::new_id(),
            provider: provider.into(),
            email: email.into(),
            display_name: None,
            auth_type: auth,
            created_at: now,
            updated_at: now,
        }
    }

    /// Stable-ish local id derived from provider + email, so re-importing the
    /// same JSON config does not duplicate local rows or lose the sync cursor.
    pub fn deterministic_id(provider: ProviderId, email: &str) -> AccountId {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        provider.as_str().hash(&mut hasher);
        '\u{1f}'.hash(&mut hasher);
        email.to_lowercase().hash(&mut hasher);
        let h = hasher.finish();
        format!("{:016x}{:016x}", h, h.wrapping_mul(0x9E3779B97F4A7C15))
    }
}
