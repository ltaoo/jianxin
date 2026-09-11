//! # eeeemail-provider
//!
//! Provider infrastructure. The dependency direction is `Provider -> Domain`;
//! nothing here may be referenced from the domain crate.
//!
//! ```text
//! MailProvider
//! ├── ApiMailProvider        (Microsoft Graph, Gmail API — Phase 2)
//! └── ProtocolMailProvider   (Generic IMAP + SMTP — today)
//!
//! AuthProvider
//! ├── Password / AppPassword
//! └── OAuth2 (XOAUTH2)
//! ```
//!
//! Providers are registered in a registry and selected by a multi-level
//! detection chain, never by an `if provider == "microsoft"` in the UI.

pub mod auth;
pub mod branded;
pub mod google;
pub mod imap;
pub mod microsoft;
pub mod smtp;
pub mod traits;

pub use auth::{AuthProvider, AuthSecret, ConfigAuth};
pub use branded::BrandedProvider;
pub use imap::{GenericImapProvider, ImapClient, ImapEndpoint, SyncSettings};
pub use smtp::SmtpEndpoint;
pub use traits::{ListMessagesOptions, MailProvider, SearchQuery, SyncDelta};

use eeeemail_config::{presets, AccountConfig, SyncConfig};
pub use eeeemail_domain::FlagUpdate;
pub use eeeemail_domain::ProviderId;
use eeeemail_domain::Result;

pub const GENERIC_IMAP: ProviderId = ProviderId::GenericImap;
pub const GOOGLE: ProviderId = google::ID;
pub const MICROSOFT: ProviderId = microsoft::ID;

/// Registry of known providers.
///
/// Registration is by *domain matcher*, not by hard-coded branching, so adding
/// Yahoo or Zoho later means adding one entry here.
#[derive(Debug, Default)]
pub struct ProviderRegistry {
    entries: Vec<RegistryEntry>,
}

#[derive(Debug)]
struct RegistryEntry {
    id: ProviderId,
    domains: Vec<&'static str>,
}

impl ProviderRegistry {
    /// The registry used by the CLI and TUI.
    pub fn builtin() -> Self {
        Self {
            entries: vec![
                RegistryEntry {
                    id: GOOGLE,
                    domains: vec!["gmail.com", "googlemail.com"],
                },
                RegistryEntry {
                    id: MICROSOFT,
                    domains: vec![
                        "outlook.com",
                        "hotmail.com",
                        "live.com",
                        "office365.com",
                        "msn.com",
                    ],
                },
            ],
        }
    }

    /// Level 1 of detection: is the email domain already known?
    pub fn detect_provider(&self, email: &str) -> Option<ProviderId> {
        let domain = email.rsplit('@').next()?.trim().to_ascii_lowercase();
        self.entries
            .iter()
            .find(|entry| entry.domains.iter().any(|d| *d == domain))
            .map(|entry| entry.id)
    }

    /// Build the provider for one configured account.
    ///
    /// Resolution order (see `AccountConfig::provider_key`):
    /// explicit `provider` > explicit IMAP/SMTP hosts > domain match + OAuth2 >
    /// generic IMAP.
    pub fn build(
        &self,
        config: &AccountConfig,
        sync: &SyncConfig,
    ) -> Result<Box<dyn MailProvider>> {
        match config.provider_key() {
            GOOGLE => Ok(Box::new(google::build(config, sync)?)),
            MICROSOFT => Ok(Box::new(microsoft::build(config, sync)?)),
            GENERIC_IMAP => Ok(Box::new(GenericImapProvider::from_config(config, sync)?)),
        }
    }
}

/// Convenience wrapper around `ProviderRegistry::detect_provider` for cases
/// that only need "is this a known vendor?".
pub fn detect_provider(email: &str) -> ProviderId {
    ProviderRegistry::builtin()
        .detect_provider(email)
        .unwrap_or(GENERIC_IMAP)
}

/// Whether the email domain has an IMAP/SMTP preset, i.e. whether auto-config
/// will work without the user typing hostnames.
pub fn has_preset(email: &str) -> bool {
    presets::detect(email).is_some()
}
