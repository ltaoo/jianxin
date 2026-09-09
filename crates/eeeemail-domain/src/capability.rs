use serde::{Deserialize, Serialize};

/// A single optional provider feature.
///
/// Feature detection in the UI must ask `capabilities.supports(Capability::X)`,
/// never `if provider == "microsoft"` — see "Feature Detection".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    Read,
    Send,
    Drafts,
    Attachments,
    Folders,
    Labels,
    Categories,
    Reply,
    Forward,
    Threading,
    DeltaSync,
    Push,
    ServerSearch,
    SharedMailbox,
    Calendar,
    Contacts,
}

/// The capability set a provider reports after it initialises.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub read: bool,
    pub send: bool,
    pub drafts: bool,
    pub attachments: bool,
    pub folders: bool,
    pub labels: bool,
    pub categories: bool,
    pub reply: bool,
    pub forward: bool,
    pub threading: bool,
    pub delta_sync: bool,
    pub push: bool,
    pub server_search: bool,
    pub shared_mailbox: bool,
    pub calendar: bool,
    pub contacts: bool,
}

impl ProviderCapabilities {
    pub fn none() -> Self {
        Self {
            read: false,
            send: false,
            drafts: false,
            attachments: false,
            folders: false,
            labels: false,
            categories: false,
            reply: false,
            forward: false,
            threading: false,
            delta_sync: false,
            push: false,
            server_search: false,
            shared_mailbox: false,
            calendar: false,
            contacts: false,
        }
    }

    /// What a plain IMAP + SMTP mailbox can do.
    pub fn imap() -> Self {
        Self {
            read: true,
            send: true,
            drafts: true,
            attachments: true,
            folders: true,
            server_search: true,
            delta_sync: true,
            push: false, // requires IMAP IDLE; see Phase 3
            reply: true,
            forward: true,
            threading: false, // local threading only
            ..Self::none()
        }
    }

    /// Microsoft Graph.
    pub fn graph() -> Self {
        Self {
            read: true,
            send: true,
            drafts: true,
            attachments: true,
            folders: true,
            categories: true,
            delta_sync: true,
            push: true,
            server_search: true,
            shared_mailbox: true,
            calendar: true,
            contacts: true,
            reply: true,
            forward: true,
            threading: true,
            ..Self::none()
        }
    }

    /// Gmail API.
    pub fn gmail() -> Self {
        Self {
            read: true,
            send: true,
            drafts: true,
            attachments: true,
            labels: true,
            delta_sync: true,
            push: true,
            server_search: true,
            threading: true,
            reply: true,
            forward: true,
            folders: true,
            ..Self::none()
        }
    }

    pub fn supports(&self, capability: Capability) -> bool {
        match capability {
            Capability::Read => self.read,
            Capability::Send => self.send,
            Capability::Drafts => self.drafts,
            Capability::Attachments => self.attachments,
            Capability::Folders => self.folders,
            Capability::Labels => self.labels,
            Capability::Categories => self.categories,
            Capability::Reply => self.reply,
            Capability::Forward => self.forward,
            Capability::Threading => self.threading,
            Capability::DeltaSync => self.delta_sync,
            Capability::Push => self.push,
            Capability::ServerSearch => self.server_search,
            Capability::SharedMailbox => self.shared_mailbox,
            Capability::Calendar => self.calendar,
            Capability::Contacts => self.contacts,
        }
    }

    /// Enable one capability (builder style).
    pub fn with(mut self, capability: Capability, value: bool) -> Self {
        match capability {
            Capability::Read => self.read = value,
            Capability::Send => self.send = value,
            Capability::Drafts => self.drafts = value,
            Capability::Attachments => self.attachments = value,
            Capability::Folders => self.folders = value,
            Capability::Labels => self.labels = value,
            Capability::Categories => self.categories = value,
            Capability::Reply => self.reply = value,
            Capability::Forward => self.forward = value,
            Capability::Threading => self.threading = value,
            Capability::DeltaSync => self.delta_sync = value,
            Capability::Push => self.push = value,
            Capability::ServerSearch => self.server_search = value,
            Capability::SharedMailbox => self.shared_mailbox = value,
            Capability::Calendar => self.calendar = value,
            Capability::Contacts => self.contacts = value,
        }
        self
    }
}

impl Default for ProviderCapabilities {
    fn default() -> Self {
        Self::none()
    }
}
