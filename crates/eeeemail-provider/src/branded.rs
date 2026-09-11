//! Native-API providers whose API transport is not implemented yet.
//!
//! The architecture document is explicit that Outlook should not be IMAP-first
//! and Gmail should not be forced onto generic IMAP. Their real transports
//! (Microsoft Graph, Gmail API) are Phase 2 and need an OAuth authorization-code
//! + PKCE flow, which is why they are not wired up here yet.
//!
//! Until then these providers still work — but honestly: they run the IMAP
//! transport with XOAUTH2, and they report the capabilities they *actually*
//! have rather than the ones their vendor API would eventually unlock. When the
//! Graph/Gmail transports land, only this file changes.

use eeeemail_config::{AccountConfig, SyncConfig};
use eeeemail_domain::{
    ComposeMessage, Folder, MailError, MailErrorCode, Message, MessagePage, ProviderCapabilities,
    ProviderId, Result, SendResult, SyncCursor,
};
use std::time::Duration;

use crate::imap::GenericImapProvider;
use crate::traits::{ListMessagesOptions, MailProvider, SearchQuery, SyncDelta};

/// A provider identity (id + capabilities) wrapped around an IMAP transport.
pub struct BrandedProvider {
    id: ProviderId,
    /// Human readable note surfaced by `eeeemail accounts`.
    pub note: &'static str,
    capabilities: ProviderCapabilities,
    inner: GenericImapProvider,
    /// Whether the vendor API transport is already wired up.
    pub api_transport: bool,
}

impl BrandedProvider {
    pub fn new(
        id: ProviderId,
        note: &'static str,
        capabilities: ProviderCapabilities,
        inner: GenericImapProvider,
    ) -> Self {
        Self {
            id,
            note,
            capabilities,
            inner,
            api_transport: false,
        }
    }

    pub fn from_config(
        id: ProviderId,
        note: &'static str,
        capabilities: ProviderCapabilities,
        config: &AccountConfig,
        sync: &SyncConfig,
    ) -> Result<Self> {
        Ok(Self::new(
            id,
            note,
            capabilities,
            GenericImapProvider::from_config(config, sync)?,
        ))
    }
}

impl MailProvider for BrandedProvider {
    fn id(&self) -> ProviderId {
        self.id
    }

    fn account_id(&self) -> &str {
        self.inner.account_id()
    }

    fn account_email(&self) -> &str {
        self.inner.account_email()
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.capabilities.clone()
    }

    fn list_folders(&self) -> Result<Vec<Folder>> {
        self.inner.list_folders()
    }

    fn list_messages(&self, folder: &Folder, opts: ListMessagesOptions) -> Result<MessagePage> {
        self.inner.list_messages(folder, opts)
    }

    fn get_message(&self, folder: &Folder, remote_id: &str) -> Result<Message> {
        self.inner.get_message(folder, remote_id)
    }

    fn send_message(&self, message: &ComposeMessage) -> Result<SendResult> {
        self.inner.send_message(message)
    }

    fn create_draft(&self, folder: &Folder, message: &ComposeMessage) -> Result<Message> {
        self.inner.create_draft(folder, message)
    }

    fn mark_read(&self, folder: &Folder, remote_id: &str, read: bool) -> Result<()> {
        self.inner.mark_read(folder, remote_id, read)
    }

    fn set_flag(&self, folder: &Folder, remote_id: &str, starred: bool) -> Result<()> {
        self.inner.set_flag(folder, remote_id, starred)
    }

    fn move_message(&self, folder: &Folder, remote_id: &str, target: &Folder) -> Result<()> {
        self.inner.move_message(folder, remote_id, target)
    }

    fn delete_message(&self, folder: &Folder, remote_id: &str) -> Result<()> {
        self.inner.delete_message(folder, remote_id)
    }

    fn search(&self, query: &SearchQuery) -> Result<MessagePage> {
        self.inner.search(query)
    }

    fn sync(&self, folder: &Folder, cursor: Option<&SyncCursor>) -> Result<SyncDelta> {
        self.inner.sync(folder, cursor)
    }

    fn test_connection(&self) -> Result<String> {
        self.inner.test_connection()
    }
}

/// Placeholder used by transports that are planned but not implemented.
pub fn not_implemented(provider: ProviderId) -> MailError {
    MailError::new(
        MailErrorCode::UnsupportedFeature,
        format!(
            "{provider} native API transport is not implemented yet; it currently runs over IMAP"
        ),
    )
}

/// Shared socket timeout helper for branded providers.
pub fn timeout_of(sync: &SyncConfig) -> Duration {
    Duration::from_secs(sync.timeout_secs.max(1))
}
