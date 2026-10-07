//! IMAP → domain normalisation.
//!
//! "Provider -> Normalizer -> Domain events -> Local DB": this is the
//! normalizer. Nothing above this module knows what a `Fetch` or a MIME part is.

use jianxin_config::FolderOverrides;
use jianxin_domain::{make_snippet, parse_address_list};
use jianxin_domain::{Address, Attachment, Folder, FolderKind, Message, Result};
use mailparse::{DispositionType, MailHeaderMap, ParsedMail};

use super::client::RawFolder;
use super::client::RawMessage;
use super::utf7;

/// Map a `LIST` entry onto a domain folder.
///
/// Remote names are never used as identifiers: the local id is derived, and the
/// role is detected from SPECIAL-USE attributes first, then the name, then an
/// explicit user override. The stored `name` is the decoded (readable) form;
/// `remote_id` keeps the wire form because that is what `SELECT` expects.
pub fn folder_from_raw(
    raw: &RawFolder,
    account_id: &str,
    overrides: Option<&FolderOverrides>,
) -> Folder {
    let display = utf7::decode(&raw.name);
    let mut kind = FolderKind::detect(&display, Some(&raw.attributes));

    if let Some(overrides) = overrides {
        for (role, forced) in [
            (FolderKind::Inbox, &overrides.inbox),
            (FolderKind::Sent, &overrides.sent),
            (FolderKind::Drafts, &overrides.drafts),
            (FolderKind::Trash, &overrides.trash),
            (FolderKind::Spam, &overrides.spam),
            (FolderKind::Archive, &overrides.archive),
        ] {
            if forced
                .as_deref()
                .is_some_and(|n| n == raw.name || n == display)
            {
                kind = role;
            }
        }
    }

    let mut folder = Folder::new(account_id, &raw.name, &display, kind);
    folder.id = Folder::deterministic_id(account_id, &raw.name);
    folder
}

/// Human-friendly display name for a remote folder path.
pub fn display_name(remote_id: &str, delimiter: Option<&str>) -> String {
    match delimiter {
        Some(d) if !d.is_empty() => remote_id.rsplit(d).next().unwrap_or(remote_id).to_string(),
        _ => remote_id
            .rsplit('/')
            .next()
            .unwrap_or(remote_id)
            .to_string(),
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
    message.received_at = raw.received_at;

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
    message.thread_index_root = headers
        .get_first_value("Thread-Index")
        .and_then(|s| decode_thread_index_root(&s));

    message.sent_at = headers
        .get_first_value("Date")
        .and_then(|value| mailparse::dateparse(&value).ok());

    walk_parts(message, parsed);
}

/// Outlook's `Thread-Index` (MS-OXCMAIL): base64 of a 16-byte conversation
/// GUID followed by per-message clock data. Only the GUID identifies the
/// conversation, so replies and forwards share the first 16 decoded bytes.
fn decode_thread_index_root(raw: &str) -> Option<String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(raw.trim())
        .ok()?;
    if bytes.len() < 16 {
        return None;
    }
    Some(
        bytes[..16]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
    )
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
        let mut attachment = Attachment::new(&message.id, filename.clone(), &mime, size);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_receipt_time_survives_missing_or_invalid_date_headers() {
        for date in ["", "Date: 32 Oct 2026 12:00:00 +0000\r\n", "Date: Fri, 02 Oct 2026 12:00:00 +0000\r\n"] {
            let raw = RawMessage {
                uid: 1, flags: vec![], size: 100, received_at: Some(1_791_028_800),
                raw: format!("{date}Subject: Security advisory\r\nFrom: alerts@example.com\r\n\r\nAdvisory").into_bytes(),
            };
            let message = message_from_raw(&raw, "acc", "inbox", FolderKind::Inbox).unwrap();
            assert_eq!(message.received_at, raw.received_at);
            assert!(message.sort_ts() > 0);
            assert!(!message.is_read);
            if date.contains("Fri,") { assert!(message.sent_at.is_some()); }
            else { assert!(message.sent_at.is_none()); }
        }
        let raw = RawMessage { uid: 2, flags: vec![], size: 0,
            received_at: Some(1_791_028_800), raw: vec![] };
        assert_eq!(message_from_raw(&raw, "acc", "inbox", FolderKind::Inbox).unwrap().received_at, raw.received_at);
    }

    #[test]
    fn decodes_display_name_but_keeps_wire_remote_id() {
        let raw = RawFolder {
            name: "[Gmail]/&XfJT0ZAB-".to_string(),
            attributes: "\\HasNoChildren \\Sent".to_string(),
            delimiter: Some("/".to_string()),
        };
        let folder = folder_from_raw(&raw, "acc", None);
        assert_eq!(folder.remote_id, "[Gmail]/&XfJT0ZAB-");
        assert_eq!(folder.name, "[Gmail]/已发送");
        assert_eq!(folder.kind, FolderKind::Sent);
    }

    #[test]
    fn detects_role_from_decoded_name_without_special_use() {
        let raw = RawFolder {
            name: "&XfJT0ZAB-".to_string(), // 已发送, no SPECIAL-USE attribute
            attributes: "\\HasNoChildren".to_string(),
            delimiter: None,
        };
        let folder = folder_from_raw(&raw, "acc", None);
        assert_eq!(folder.name, "已发送");
        assert_eq!(folder.kind, FolderKind::Sent);
    }

    #[test]
    fn decodes_thread_index_root() {
        // 22 bytes: a 16-byte GUID, then per-message clock data.
        let raw: Vec<u8> = (0u8..22).collect();
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&raw);
        assert_eq!(
            decode_thread_index_root(&encoded).as_deref(),
            Some("000102030405060708090a0b0c0d0e0f")
        );
    }

    #[test]
    fn rejects_broken_thread_index() {
        assert_eq!(decode_thread_index_root("not base64!!!"), None);
        assert_eq!(
            decode_thread_index_root(&{
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.encode([0u8; 8])
            }),
            None
        );
    }
}
