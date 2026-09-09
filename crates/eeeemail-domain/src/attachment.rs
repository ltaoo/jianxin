use serde::{Deserialize, Serialize};

/// Attachment metadata.
///
/// Attachment *payloads* are lazy: the row only describes the attachment, and
/// the body is fetched on demand (Graph `$value`, IMAP `BODY.PEEK[...]`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    pub message_id: crate::MessageId,
    pub filename: Option<String>,
    pub mime_type: String,
    pub size: u64,
    /// `Content-ID` for inline images.
    pub content_id: Option<String>,
    /// IMAP section path / Graph attachment id — provider specific locator.
    pub remote_id: Option<String>,
    /// Set once the payload has been downloaded into the local cache dir.
    pub local_path: Option<String>,
}

impl Attachment {
    pub fn new(message_id: &str, filename: Option<String>, mime_type: &str, size: u64) -> Self {
        Self {
            id: crate::new_id(),
            message_id: message_id.to_string(),
            filename,
            mime_type: mime_type.to_string(),
            size,
            content_id: None,
            remote_id: None,
            local_path: None,
        }
    }

    pub fn display_name(&self) -> &str {
        self.filename.as_deref().unwrap_or("attachment")
    }
}
