//! Gmail provider.
//!
//! ```text
//! GmailProvider
//!      ├── GmailOAuth
//!      ├── GmailAPITransport   <- Phase 2
//!      └── IMAP + XOAUTH2      <- active today
//! ```
//!
//! Gmail IMAP with XOAUTH2 is fully functional, so this provider is useful as
//! soon as an access token is configured. The Gmail API transport (history-based
//! delta sync, labels, native threads) slots in behind the same
//! `MailProvider` trait later.

use eeeemail_config::{AccountConfig, SyncConfig};
use eeeemail_domain::{Capability, ProviderCapabilities, ProviderId, Result};

use crate::branded::BrandedProvider;

pub const ID: ProviderId = ProviderId::Google;

pub const NOTE: &str = "Gmail over IMAP + XOAUTH2 (Gmail API transport: Phase 2)";

pub fn capabilities() -> ProviderCapabilities {
    // Honest about today's transport: IMAP gives us read/send/folders/search
    // and UID-based delta sync, but not labels or native threads.
    ProviderCapabilities::imap().with(Capability::Threading, false)
}

/// Well-known Gmail domains.
pub fn matches(email: &str) -> bool {
    let domain = email.rsplit('@').next().unwrap_or_default().to_lowercase();
    matches!(domain.as_str(), "gmail.com" | "googlemail.com")
}

pub fn build(config: &AccountConfig, sync: &SyncConfig) -> Result<BrandedProvider> {
    BrandedProvider::from_config(ID, NOTE, capabilities(), config, sync)
}

/// Compile-time reminder of what is still missing, surfaced by `eeeemail doctor`.
pub const PENDING_FEATURES: &[Capability] =
    &[Capability::Labels, Capability::Threading, Capability::Push];
