//! IMAP → domain normalisation.
//!
//! "Provider -> Normalizer -> Domain events -> Local DB": this is the
//! normalizer. Nothing above this module knows what a `Fetch` or a MIME part is.

use eeeemail_config::FolderOverrides;
use eeeemail_domain::{make_snippet, parse_address_list};
use eeeemail_domain::{Address, Attachment, Folder, FolderKind, Message, Result};
use mailparse::{DispositionType, MailHeaderMap, ParsedMail};

use super::client::RawFolder;
use super::client::RawMessage;

/// Map a `LIST` entry onto a domain folder.
///
/// Remote names are never used as identifiers: the local id is derived, and the
/// role is detected from SPECIAL-USE attributes first, then the name, then an
/// explicit user override.
pub fn folder_from_raw(
    raw: &RawFolder,
    account_id: &str,
    overrides: Option<&FolderOverrides>,
) -> Folder {
    let mut kind = FolderKind::detect(&raw.name, Some(&raw.attributes));

    if let Some(overrides) = overrides {
        for (role, forced) in [
            (FolderKind::Inbox, &overrides.inbox),
            (FolderKind::Sent, &overrides.sent),
            (FolderKind::Drafts, &overrides.drafts),
            (FolderKind::Trash, &overrides.trash),
            (FolderKind::Spam, &overrides.spam),
            (FolderKind::Archive, &overrides.archive),
        ] {
            if forced.as_deref() == Some(raw.name.as_str()) {
                kind = role;
            }
        }
    }

    let mut folder = Folder::new(account_id, &raw.name, &raw.name, kind);
    folder.id = Folder::deterministic_id(account_id, &raw.name);
    folder
}

/// Human-friendly display name for a remote folder path.
pub fn display_name(remote_id: &str, delimiter: Option<&str>) -> String {
    match delimiter {
        Some(d) if !d.is_empty() => remote_id
            .rsplit(d)
            .next()
            .unwrap_or(remote_id)
            .to_string(),
        _ => remote_id.rsplit('/').next().unwrap_or(remote_id).to_string(),
    }
}

/// Map a fetched message onto the unified domain model.
pub fn message_from_raw(
    raw: &RawMessage,
    account_id: &str,
    folder_id: &str,
    folder_kind: FolderKind,
) -> Result<Message> {
    let mut message = Message::new();
    message.account_id = account_id.to_string();
    message.folder_id = folder_id.to_string();
    message.remote_id = raw.uid.to_string();
    message.id = Message::deterministic_id(account_id, folder_id, &raw.uid.to_string());
    message.size = raw.size;

    for flag in &raw.flags {
        match flag.to_ascii_lowercase().as_str() {
            "\\seen" => message.is_read = true,
            "\\flagged" => message.is_starred = true,
            "\\draft" => message.is_draft = true,
            "\\deleted" => message.is_deleted = true,
            _ => {}
        }
    }
    message.is_draft |= matches!(folder_kind, FolderKind::Drafts);

    if raw.raw.is_empty() {
        return Ok(message);
    }

    match mailparse::parse_mail(&raw.raw) {
        Ok(parsed) => {
            fill_from_parsed(&mut message, &parsed);
        }
        Err(err) => {
            // A malformed message must not break an inbox listing — keep the
            // envelope we already have and record a placeholder body.
            tracing::warn!(uid = raw.uid, "cannot parse message: {err}");
            message.subject = "(unparseable message)".to_string();
        }
    }

    if message.snippet.is_none() {
        message.snippet = message
            .text_body
            .as_deref()
            .map(|t| make_snippet(t, 200))
            .filter(|s| !s.is_empty());
    }

    Ok(message)
}

fn fill_from_parsed(message: &mut Message, parsed: &ParsedMail) {
    let headers = &parsed.headers;

    message.subject = headers
        .get_first_value("Subject")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "(no subject)".to_string());

    message.from = addresses(headers.get_first_value("From"));
    message.to = addresses(headers.get_first_value("To"));
    message.cc = addresses(headers.get_first_value("Cc"));
    message.bcc = addresses(headers.get_first_value("Bcc"));
    message.reply_to = addresses(headers.get_first_value("Reply-To"));

    message.message_id_header = headers
        .get_first_value("Message-ID")
        .map(|s| s.trim().to_string());
    message.in_reply_to = headers
        .get_first_value("In-Reply-To")
        .map(|s| s.trim().to_string());
    message.references = headers
        .get_first_value("References")
        .map(|s| {
            s.split_whitespace()
                .filter(|part| part.starts_with('<'))
                .map(|part| part.to_string())
                .collect()
        })
        .unwrap_or_default();

    message.sent_at = headers
        .get_first_value("Date")
        .and_then(|value| mailparse::dateparse(&value).ok());

    walk_parts(message, parsed);
}

fn addresses(header: Option<String>) -> Vec<Address> {
    parse_address_list(&header.unwrap_or_default())
}

fn walk_parts(message: &mut Message, part: &ParsedMail) {
    let mime = part.ctype.mimetype.to_ascii_lowercase();
    let disposition = part.get_content_disposition();
    let filename = disposition
        .params
        .get("filename")
        .cloned()
        .or_else(|| part.ctype.params.get("name").cloned());

    let is_attachment = matches!(disposition.disposition, DispositionType::Attachment)
        || (filename.is_some() && !mime.starts_with("text/") && !mime.starts_with("multipart/"));

    if is_attachment {
        let size = part.get_body_raw().map(|b| b.len() as u64).unwrap_or(0);
        let mut attachment = Attachment::new(
            &message.id,
            filename.clone(),
            &mime,
            size,
        );
        attachment.content_id = part.headers.get_first_value("Content-ID");
        message.attachments.push(attachment);
        return;
    }

    match mime.as_str() {
        "text/plain" if message.text_body.is_none() => {
            message.text_body = decoded_body(part);
        }
        "text/html" if message.html_body.is_none() => {
            message.html_body = decoded_body(part);
        }
        _ => {}
    }

    for sub in &part.subparts {
        walk_parts(message, sub);
    }
}

fn decoded_body(part: &ParsedMail) -> Option<String> {
    if let Ok(body) = part.get_body() {
        return Some(body);
    }
    // Fall back to whatever bytes are there rather than dropping the content.
    part.get_body_raw()
        .ok()
        .map(|raw| String::from_utf8_lossy(&raw).to_string())
}
