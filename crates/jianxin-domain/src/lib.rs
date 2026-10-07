//! # jianxin-domain
//!
//! Provider-agnostic domain model.
//!
//! Nothing in this crate may know about Graph / Gmail / IMAP / SMTP. The moment
//! `GraphMessage` or `IMAPMessage` shows up here, the layering is broken — see
//! "Provider 与 Domain 的边界" in the architecture document.
//!
//! Dependency rule: `Provider -> Domain`, never `Domain -> Provider`.

pub mod account;
pub mod address;
pub mod attachment;
pub mod capability;
pub mod compose;
pub mod error;
pub mod folder;
pub mod message;
pub mod provider;
pub mod sync;
pub mod thread;

pub use account::{Account, AccountId, AuthType};
pub use address::{join_addresses, parse_address_list, Address};
pub use attachment::Attachment;
pub use capability::{Capability, ProviderCapabilities};
pub use compose::{ComposeAttachment, ComposeMessage, SendResult};
pub use error::{MailError, MailErrorCode, Result};
pub use folder::{Folder, FolderId, FolderKind};
pub use message::{Message, MessageId, MessagePage};
pub use provider::ProviderId;
pub use sync::{FlagUpdate, SyncCursor, SyncResult};
pub use thread::{Thread, ThreadId};

/// Local identifiers are always UUIDv4. Remote (provider) identifiers are never
/// used as primary keys — the mapping is `local UUID + provider + remoteId`.
pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Unix timestamp (seconds) for "now".
pub fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Collapse whitespace and truncate, used to derive message list snippets.
pub fn make_snippet(text: &str, max_chars: usize) -> String {
    let flat: String = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    if flat.chars().count() <= max_chars {
        flat
    } else {
        let truncated: String = flat.chars().take(max_chars).collect();
        format!("{truncated}…")
    }
}
