//! The `MailProvider` contract and its supporting types.
//!
//! Two rules shape this file:
//!
//! 1. **Do not assume every provider supports everything.** Most methods have a
//!    default implementation returning `UnsupportedFeature`, and callers gate on
//!    `capabilities()` instead of on the provider name.
//! 2. **Provider IDs and local IDs stay separate.** Remote ids are opaque
//!    strings handed back to the provider; nothing else may interpret them.

use eeeemail_domain::{
    Capability, ComposeMessage, Folder, MailError, MailErrorCode, Message, MessagePage,
    ProviderCapabilities, ProviderId, Result, SendResult, SyncCursor, SyncResult,
};

/// Re-exported so downstream modules can import flag updates from the trait
/// module without reaching into the domain crate directly.
pub use eeeemail_domain::FlagUpdate;

/// Options for listing a folder.
#[derive(Debug, Clone, Default)]
pub struct ListMessagesOptions {
    pub limit: u32,
    pub offset: u32,
    pub unread_only: bool,
    /// Provider page token (`nextPageToken`, IMAP `UID` range cursor, …).
    pub cursor: Option<String>,
}

impl ListMessagesOptions {
    pub fn new(limit: u32) -> Self {
        Self {
            limit,
            ..Default::default()
        }
    }

    pub fn offset(mut self, offset: u32) -> Self {
        self.offset = offset;
        self
    }

    pub fn unread_only(mut self) -> Self {
        self.unread_only = true;
        self
    }
}

/// A provider-independent search request.
#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    /// Full-text needle (subject + body + addresses).
    pub text: Option<String>,
    pub subject: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub unread_only: bool,
    /// Restrict to one folder; `None` searches the whole account.
    pub folder_id: Option<String>,
    pub limit: u32,
}

impl SearchQuery {
    pub fn text(needle: impl Into<String>) -> Self {
        Self {
            text: Some(needle.into()),
            limit: 50,
            ..Default::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_none()
            && self.subject.is_none()
            && self.from.is_none()
            && self.to.is_none()
            && !self.unread_only
    }
}

/// What changed in one folder since the last cursor.
///
/// The sync pipeline is `Provider -> Remote changes -> Normalizer -> Domain
/// events -> Local DB`, so the delta already carries fully mapped domain
/// `Message` values rather than provider payloads.
#[derive(Debug, Clone, Default)]
pub struct SyncDelta {
    pub folder_id: String,
    pub folder_name: String,
    /// Newly arrived messages, fully parsed.
    pub new_messages: Vec<Message>,
    /// Messages whose flags changed but whose body did not.
    pub flag_updates: Vec<FlagUpdate>,
    /// Remote ids that no longer exist on the server.
    ///
    /// Providers with a real change feed (Graph delta) report deletions
    /// directly. IMAP has no change feed, so it instead fills
    /// `present_remote_ids` and lets the sync service diff against the local
    /// database.
    pub removed_remote_ids: Vec<String>,
    /// Every remote id currently on the server, when the provider can cheaply
    /// enumerate them.
    pub present_remote_ids: Vec<String>,
    pub cursor: Option<SyncCursor>,
    pub has_more: bool,
}

impl SyncDelta {
    pub fn new(folder_id: &str, folder_name: &str) -> Self {
        Self {
            folder_id: folder_id.to_string(),
            folder_name: folder_name.to_string(),
            ..Default::default()
        }
    }

    pub fn summary(&self) -> SyncResult {
        SyncResult {
            folder_id: self.folder_id.clone(),
            folder_name: self.folder_name.clone(),
            upserted: self.new_messages.len(),
            removed: self.removed_remote_ids.len(),
            flagged: self.flag_updates.len(),
            cursor: self.cursor.clone(),
            has_more: self.has_more,
        }
    }
}

/// Unified mail provider.
///
/// Implementations are cheap, stateless factories: each call opens its own
/// connection. That keeps them `Send + Sync` for free, which the TUI needs in
/// order to sync on a background thread while the UI keeps rendering.
pub trait MailProvider: Send + Sync {
    fn id(&self) -> ProviderId;

    fn account_id(&self) -> &str;

    fn account_email(&self) -> &str;

    fn capabilities(&self) -> ProviderCapabilities;

    /// Gate an operation on a capability rather than on the provider name.
    fn require(&self, capability: Capability) -> Result<()> {
        if self.capabilities().supports(capability) {
            Ok(())
        } else {
            Err(MailError::unsupported(&format!("{capability:?}")))
        }
    }

    // --- required ---------------------------------------------------------

    fn list_folders(&self) -> Result<Vec<Folder>>;

    fn list_messages(&self, folder: &Folder, opts: ListMessagesOptions) -> Result<MessagePage>;

    fn get_message(&self, folder: &Folder, remote_id: &str) -> Result<Message>;

    fn send_message(&self, message: &ComposeMessage) -> Result<SendResult>;

    // --- optional ---------------------------------------------------------

    fn create_draft(&self, _folder: &Folder, _message: &ComposeMessage) -> Result<Message> {
        Err(MailError::unsupported("create_draft"))
    }

    fn update_draft(
        &self,
        _folder: &Folder,
        _remote_id: &str,
        _message: &ComposeMessage,
    ) -> Result<Message> {
        Err(MailError::unsupported("update_draft"))
    }

    fn mark_read(&self, _folder: &Folder, _remote_id: &str, _read: bool) -> Result<()> {
        Err(MailError::unsupported("mark_read"))
    }

    fn set_flag(&self, _folder: &Folder, _remote_id: &str, _starred: bool) -> Result<()> {
        Err(MailError::unsupported("set_flag"))
    }

    fn move_message(&self, _folder: &Folder, _remote_id: &str, _target: &Folder) -> Result<()> {
        Err(MailError::unsupported("move_message"))
    }

    fn delete_message(&self, _folder: &Folder, _remote_id: &str) -> Result<()> {
        Err(MailError::unsupported("delete_message"))
    }

    fn search(&self, _query: &SearchQuery) -> Result<MessagePage> {
        Err(MailError::unsupported("search"))
    }

    /// Incremental sync of one folder.
    fn sync(&self, _folder: &Folder, _cursor: Option<&SyncCursor>) -> Result<SyncDelta> {
        Err(MailError::unsupported("sync"))
    }

    /// Test connectivity and credentials. Used by `eeeemail doctor`.
    fn test_connection(&self) -> Result<String> {
        self.list_folders()
            .map(|folders| format!("ok, {} folders", folders.len()))
    }
}

/// Helper used by providers that do not implement a given operation.
pub fn unsupported(feature: &str) -> MailError {
    MailError::new(
        MailErrorCode::UnsupportedFeature,
        format!("this provider does not support `{feature}`"),
    )
}
