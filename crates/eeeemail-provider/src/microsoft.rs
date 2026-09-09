//! Microsoft / Outlook provider.
//!
//! ```text
//! MicrosoftProvider
//!      ├── MicrosoftOAuth
//!      ├── GraphTransport      <- Phase 2
//!      └── IMAP + XOAUTH2      <- active today
//! ```
//!
//! Microsoft officially supports OAuth-authenticated IMAP and requires the
//! XOAUTH2 SASL payload, which `crate::auth` builds. Graph (delta sync,
//! categories, focused inbox, shared mailboxes) comes later; the capability set
//! below reflects what this transport can actually do right now.

use eeeemail_config::{AccountConfig, SyncConfig};
use eeeemail_domain::{Capability, ProviderCapabilities, Result};

use crate::branded::BrandedProvider;

pub const ID: &str = "microsoft";

pub const NOTE: &str = "Outlook over IMAP + XOAUTH2 (Microsoft Graph transport: Phase 2)";

pub fn capabilities() -> ProviderCapabilities {
    ProviderCapabilities::imap().with(Capability::Threading, false)
}

/// Well-known Microsoft consumer domains.
pub fn matches(email: &str) -> bool {
    let domain = email.rsplit('@').next().unwrap_or_default().to_lowercase();
    matches!(
        domain.as_str(),
        "outlook.com" | "hotmail.com" | "live.com" | "office365.com" | "msn.com"
    )
}

pub fn build(config: &AccountConfig, sync: &SyncConfig) -> Result<BrandedProvider> {
    BrandedProvider::from_config(ID, NOTE, capabilities(), config, sync)
}

pub const PENDING_FEATURES: &[Capability] = &[
    Capability::Categories,
    Capability::Threading,
    Capability::Push,
    Capability::SharedMailbox,
];
