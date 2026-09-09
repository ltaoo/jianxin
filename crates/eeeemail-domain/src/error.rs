use serde::{Deserialize, Serialize};

/// Unified error taxonomy.
///
/// HTTP / protocol level failures must never leak to the UI. A Graph 401 becomes
/// `AuthenticationRequired`; an IMAP `NO` on login becomes `AuthenticationFailed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MailErrorCode {
    AuthenticationRequired,
    AuthenticationFailed,
    PermissionDenied,
    NetworkUnavailable,
    Timeout,
    RateLimited,
    MessageNotFound,
    FolderNotFound,
    SendFailed,
    AttachmentTooLarge,
    ProviderUnavailable,
    UnsupportedFeature,
    InvalidConfig,
    Storage,
    Unknown,
}

impl MailErrorCode {
    /// Whether an automatic retry makes sense.
    ///
    /// 429 / 5xx / timeouts / network drops: retry with backoff.
    /// 403 permission errors: do not retry forever.
    pub fn is_retriable(&self) -> bool {
        matches!(
            self,
            MailErrorCode::NetworkUnavailable
                | MailErrorCode::Timeout
                | MailErrorCode::RateLimited
                | MailErrorCode::ProviderUnavailable
                | MailErrorCode::Unknown
        )
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            MailErrorCode::AuthenticationRequired => "AUTHENTICATION_REQUIRED",
            MailErrorCode::AuthenticationFailed => "AUTHENTICATION_FAILED",
            MailErrorCode::PermissionDenied => "PERMISSION_DENIED",
            MailErrorCode::NetworkUnavailable => "NETWORK_UNAVAILABLE",
            MailErrorCode::Timeout => "TIMEOUT",
            MailErrorCode::RateLimited => "RATE_LIMITED",
            MailErrorCode::MessageNotFound => "MESSAGE_NOT_FOUND",
            MailErrorCode::FolderNotFound => "FOLDER_NOT_FOUND",
            MailErrorCode::SendFailed => "SEND_FAILED",
            MailErrorCode::AttachmentTooLarge => "ATTACHMENT_TOO_LARGE",
            MailErrorCode::ProviderUnavailable => "PROVIDER_UNAVAILABLE",
            MailErrorCode::UnsupportedFeature => "UNSUPPORTED_FEATURE",
            MailErrorCode::InvalidConfig => "INVALID_CONFIG",
            MailErrorCode::Storage => "STORAGE",
            MailErrorCode::Unknown => "UNKNOWN",
        }
    }
}

impl std::fmt::Display for MailErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug)]
pub struct MailError {
    pub code: MailErrorCode,
    pub message: String,
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl MailError {
    pub fn new(code: MailErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            source: None,
        }
    }

    pub fn with_source(mut self, err: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(err));
        self
    }

    pub fn unsupported(feature: &str) -> Self {
        Self::new(
            MailErrorCode::UnsupportedFeature,
            format!("provider does not support `{feature}`"),
        )
    }

    pub fn auth_failed(detail: impl Into<String>) -> Self {
        Self::new(MailErrorCode::AuthenticationFailed, detail)
    }

    pub fn auth_required(detail: impl Into<String>) -> Self {
        Self::new(MailErrorCode::AuthenticationRequired, detail)
    }

    pub fn is_retriable(&self) -> bool {
        self.code.is_retriable()
    }
}

impl std::fmt::Display for MailError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)?;
        if let Some(src) = &self.source {
            write!(f, " ({src})")?;
        }
        Ok(())
    }
}

impl std::error::Error for MailError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_ref().map(|e| e.as_ref() as _)
    }
}

pub type Result<T, E = MailError> = std::result::Result<T, E>;
