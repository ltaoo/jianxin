//! SMTP transport.
//!
//! Sending goes through the Outbox (`Compose -> Draft -> Outbox -> Provider`),
//! so this module only knows how to hand one fully built message to a server —
//! retry policy lives one layer up.

use eeeemail_config::TlsMode;
use eeeemail_domain::{Address, ComposeMessage, MailError, MailErrorCode, Result, SendResult};
use lettre::message::header::ContentType;
use lettre::message::{Attachment, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{Message, SmtpTransport, Transport};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct SmtpEndpoint {
    pub host: String,
    pub port: u16,
    pub tls: TlsMode,
    pub timeout: Duration,
}

impl SmtpEndpoint {
    pub fn new(host: &str, port: u16, tls: TlsMode, timeout: Duration) -> Self {
        Self {
            host: host.to_string(),
            port,
            tls,
            timeout,
        }
    }
}

/// Build the MIME message without sending it.
///
/// Exposed separately so the outbox can persist a rendered draft and so
/// `eeeemail send --dry-run` can show exactly what would go out.
pub fn build_message(message: &ComposeMessage, from: &Address) -> Result<Message> {
    if !message.has_recipients() {
        return Err(MailError::new(
            MailErrorCode::SendFailed,
            "refusing to send a message without recipients",
        ));
    }

    let mut builder = Message::builder()
        .from(to_mailbox(from)?)
        .subject(message.subject.clone())
        .message_id(Some(generate_message_id(from)));

    for address in &message.to {
        builder = builder.to(to_mailbox(address)?);
    }
    for address in &message.cc {
        builder = builder.cc(to_mailbox(address)?);
    }
    for address in &message.bcc {
        builder = builder.bcc(to_mailbox(address)?);
    }

    if let Some(in_reply_to) = &message.in_reply_to {
        builder = builder.in_reply_to(in_reply_to.clone());
        if !message.references.is_empty() {
            builder = builder.references(message.references.join(" "));
        }
    }

    let plain = message.text_body.clone().unwrap_or_default();
    let html = message.html_body.clone();
    let text_part = || -> Result<SinglePart> {
        Ok(SinglePart::builder()
            .header(content_type("text/plain; charset=utf-8")?)
            .body(plain.clone()))
    };

    if message.attachments.is_empty() && html.is_none() {
        return builder
            .singlepart(text_part()?)
            .map_err(|e| MailError::new(MailErrorCode::SendFailed, format!("build message: {e}")));
    }

    if message.attachments.is_empty() {
        let html_part = SinglePart::builder()
            .header(content_type("text/html; charset=utf-8")?)
            .body(html.unwrap_or_default());
        return builder
            .multipart(MultiPart::alternative().singlepart(text_part()?).singlepart(html_part))
            .map_err(|e| MailError::new(MailErrorCode::SendFailed, format!("build message: {e}")));
    }

    let mut mixed = MultiPart::mixed().singlepart(text_part()?);
    if let Some(html) = html {
        mixed = mixed.singlepart(
            SinglePart::builder()
                .header(content_type("text/html; charset=utf-8")?)
                .body(html),
        );
    }
    for attachment in &message.attachments {
        let mime = if attachment.mime_type.is_empty() {
            eeeemail_domain::ComposeAttachment::guess_mime(&attachment.filename)
        } else {
            attachment.mime_type.clone()
        };
        mixed = mixed.singlepart(Attachment::new(attachment.filename.clone()).body(
            attachment.data.clone(),
            content_type(&mime)?,
        ));
    }

    builder
        .multipart(mixed)
        .map_err(|e| MailError::new(MailErrorCode::SendFailed, format!("build message: {e}")))
}

/// `<uuid@domain>` using the sender's domain.
pub fn generate_message_id(from: &Address) -> String {
    let domain = from.email.rsplit('@').next().unwrap_or("localhost");
    let unique = uuid::Uuid::new_v4().simple();
    format!("<{unique}@{domain}>")
}

/// Deliver a message through SMTP.
pub fn send(
    endpoint: &SmtpEndpoint,
    username: &str,
    secret: &str,
    is_oauth2: bool,
    message: &ComposeMessage,
    from: &Address,
) -> Result<SendResult> {
    let message_id = generate_message_id(from);
    let email = build_message(message, from)?;

    let tls = match endpoint.tls {
        // Implicit TLS: port 465.
        TlsMode::Tls => Tls::Wrapper(tls_parameters(&endpoint.host)?),
        // STARTTLS: port 587.
        TlsMode::StartTls => Tls::Required(tls_parameters(&endpoint.host)?),
        // Local relay only.
        TlsMode::None => Tls::None,
    };

    let mut builder = SmtpTransport::builder_dangerous(endpoint.host.clone())
        .port(endpoint.port)
        .tls(tls)
        .timeout(Some(endpoint.timeout))
        .credentials(Credentials::new(username.to_string(), secret.to_string()));

    if is_oauth2 {
        builder = builder.authentication(vec![Mechanism::Xoauth2]);
    } else {
        // Office 365 and several Chinese providers only offer LOGIN.
        builder = builder.authentication(vec![Mechanism::Plain, Mechanism::Login]);
    }

    let mailer = builder.build();
    mailer.send(&email).map_err(|e| {
        let text = e.to_string();
        let lower = text.to_ascii_lowercase();
        let code = if lower.contains("authentication")
            || lower.contains("credentials")
            || lower.contains("5.7.")
            || lower.contains("535")
        {
            MailErrorCode::AuthenticationFailed
        } else if lower.contains("timed out") || lower.contains("timeout") {
            MailErrorCode::Timeout
        } else if lower.contains("4.7.0") || lower.contains("too many") {
            MailErrorCode::RateLimited
        } else if lower.contains("5.3.4") || lower.contains("too big") || lower.contains("size") {
            MailErrorCode::AttachmentTooLarge
        } else {
            MailErrorCode::SendFailed
        };
        MailError::new(code, text)
    })?;

    Ok(SendResult {
        remote_id: None,
        message_id_header: Some(message_id),
        sent_at: eeeemail_domain::now_ts(),
    })
}

fn tls_parameters(host: &str) -> Result<TlsParameters> {
    TlsParameters::new(host.to_string())
        .map_err(|e| MailError::new(MailErrorCode::SendFailed, format!("tls setup: {e}")))
}

fn content_type(value: &str) -> Result<ContentType> {
    ContentType::parse(value)
        .map_err(|e| MailError::new(MailErrorCode::SendFailed, format!("bad content type: {e}")))
}

fn to_mailbox(address: &Address) -> Result<Mailbox> {
    // `Mailbox` implements `FromStr` for `Name <a@b.c>`; fall back to the bare
    // address when the display name contains something lettre rejects.
    if let Ok(mailbox) = address.to_header().parse::<Mailbox>() {
        return Ok(mailbox);
    }
    address
        .email
        .parse::<Mailbox>()
        .map_err(|e| MailError::new(MailErrorCode::SendFailed, format!("bad address: {e}")))
}
