use crate::{AccountId, Address, Attachment, FolderId};
use serde::{Deserialize, Serialize};

/// Local message id (UUID). The provider id lives in `remote_id`.
pub type MessageId = String;

/// The single unified message model.
///
/// Neither a Graph message, nor a Gmail message, nor a raw IMAP envelope is
/// allowed to reach the UI — everything is normalised into this shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    // --- identity ---------------------------------------------------------
    pub id: MessageId,
    pub account_id: AccountId,
    pub folder_id: FolderId,
    /// Provider id. For IMAP this is the UID (as text); for Graph the message id.
    pub remote_id: String,
    /// Stable provider id when the provider offers one (Graph immutable id).
    pub immutable_id: Option<String>,

    // --- threading --------------------------------------------------------
    pub thread_id: Option<String>,
    /// RFC 5322 `Message-ID`.
    pub message_id_header: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,

    // --- addressing -------------------------------------------------------
    pub from: Vec<Address>,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
    pub reply_to: Vec<Address>,

    // --- content ----------------------------------------------------------
    pub subject: String,
    pub snippet: Option<String>,
    pub text_body: Option<String>,
    pub html_body: Option<String>,
    pub attachments: Vec<Attachment>,

    // --- timestamps (unix seconds) ----------------------------------------
    pub received_at: Option<i64>,
    pub sent_at: Option<i64>,

    // --- state ------------------------------------------------------------
    pub is_read: bool,
    pub is_starred: bool,
    pub is_draft: bool,
    pub is_deleted: bool,
    pub labels: Vec<String>,
    pub size: u64,

    /// Provider-specific escape hatch (Graph categories, Gmail label ids, …).
    /// Prefer adding a real field; use this only for genuinely vendor-specific
    /// data that cannot be abstracted.
    pub provider_data: Option<serde_json::Value>,
}

impl Default for Message {
    fn default() -> Self {
        Self {
            id: crate::new_id(),
            account_id: String::new(),
            folder_id: String::new(),
            remote_id: String::new(),
            immutable_id: None,
            thread_id: None,
            message_id_header: None,
            in_reply_to: None,
            references: Vec::new(),
            from: Vec::new(),
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            reply_to: Vec::new(),
            subject: String::new(),
            snippet: None,
            text_body: None,
            html_body: None,
            attachments: Vec::new(),
            received_at: None,
            sent_at: None,
            is_read: false,
            is_starred: false,
            is_draft: false,
            is_deleted: false,
            labels: Vec::new(),
            size: 0,
            provider_data: None,
        }
    }
}

impl Message {
    pub fn new() -> Self {
        Self::default()
    }

    /// Deterministic local id derived from the account + folder + remote id.
    pub fn deterministic_id(account_id: &str, folder_id: &str, remote_id: &str) -> MessageId {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        account_id.hash(&mut hasher);
        '\u{1f}'.hash(&mut hasher);
        folder_id.hash(&mut hasher);
        '\u{1f}'.hash(&mut hasher);
        remote_id.hash(&mut hasher);
        let h = hasher.finish();
        format!("{:016x}{:016x}", h, h.wrapping_mul(0x9E3779B97F4A7C15))
    }

    /// Best-effort sender for list display.
    pub fn sender(&self) -> &Address {
        self.from.first().unwrap_or_else(|| {
            // No `From:` header at all — synthesise something printable.
            static FALLBACK: std::sync::OnceLock<Address> = std::sync::OnceLock::new();
            FALLBACK.get_or_init(|| Address::new("unknown@invalid"))
        })
    }

    /// Timestamp used for sorting the message list.
    pub fn sort_ts(&self) -> i64 {
        self.sent_at.or(self.received_at).unwrap_or(0)
    }

    /// Body to render in the reader pane.
    pub fn body_preferred(&self) -> &str {
        match (&self.text_body, &self.html_body) {
            (Some(text), _) if !text.trim().is_empty() => text,
            (_, Some(html)) if !html.trim().is_empty() => html,
            (Some(text), _) => text,
            _ => "",
        }
    }
}

/// One page of a listing / search result.
#[derive(Debug, Clone, Default)]
pub struct MessagePage {
    pub messages: Vec<Message>,
    pub has_more: bool,
    /// Provider cursor (Gmail `nextPageToken`, Graph `@odata.nextLink`, …).
    /// Never assume a cursor type — the sync layer stores it opaquely.
    pub next_cursor: Option<String>,
}

impl MessagePage {
    pub fn new(messages: Vec<Message>) -> Self {
        Self {
            messages,
            has_more: false,
            next_cursor: None,
        }
    }
}
