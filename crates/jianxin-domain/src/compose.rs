use crate::Address;
use serde::{Deserialize, Serialize};

/// An attachment the user is about to send.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeAttachment {
    pub filename: String,
    pub mime_type: String,
    #[serde(skip)]
    pub data: Vec<u8>,
}

impl ComposeAttachment {
    pub fn new(filename: impl Into<String>, mime_type: impl Into<String>, data: Vec<u8>) -> Self {
        Self {
            filename: filename.into(),
            mime_type: mime_type.into(),
            data,
        }
    }

    /// Guess a MIME type from the file extension — deliberately small, since we
    /// only need to cover what people actually attach.
    pub fn guess_mime(filename: &str) -> String {
        let ext = filename
            .rsplit('.')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext.as_str() {
            "txt" | "log" | "md" => "text/plain",
            "csv" => "text/csv",
            "html" | "htm" => "text/html",
            "json" => "application/json",
            "pdf" => "application/pdf",
            "zip" => "application/zip",
            "gz" | "tgz" => "application/gzip",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "svg" => "image/svg+xml",
            "mp3" => "audio/mpeg",
            "mp4" => "video/mp4",
            _ => "application/octet-stream",
        }
        .to_string()
    }
}

/// What the UI hands to `ComposeService`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComposeMessage {
    /// Falls back to the account address when `None`.
    pub from: Option<Address>,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
    pub subject: String,
    pub text_body: Option<String>,
    pub html_body: Option<String>,
    pub attachments: Vec<ComposeAttachment>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
}

impl ComposeMessage {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn to_address(mut self, address: Address) -> Self {
        self.to.push(address);
        self
    }

    pub fn subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = subject.into();
        self
    }

    pub fn text(mut self, body: impl Into<String>) -> Self {
        self.text_body = Some(body.into());
        self
    }

    pub fn has_recipients(&self) -> bool {
        !self.to.is_empty() || !self.cc.is_empty() || !self.bcc.is_empty()
    }
}

/// Provider acknowledgement of a successful send.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SendResult {
    /// Remote id of the created message, when the provider exposes one.
    pub remote_id: Option<String>,
    pub message_id_header: Option<String>,
    pub sent_at: i64,
}
