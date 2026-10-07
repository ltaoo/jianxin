use serde::{Deserialize, Serialize};
use std::fmt;

/// A single mailbox address: `"Name" <user@example.com>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Address {
    pub name: Option<String>,
    pub email: String,
}

impl Address {
    pub fn new(email: impl Into<String>) -> Self {
        Self {
            name: None,
            email: email.into(),
        }
    }

    pub fn with_name(name: impl Into<String>, email: impl Into<String>) -> Self {
        Self {
            name: Some(name.into()),
            email: email.into(),
        }
    }

    /// Best-effort RFC 5322 rendering, used when composing outgoing mail.
    pub fn to_header(&self) -> String {
        match &self.name {
            Some(name) if !name.is_empty() => {
                format!("{} <{}>", quote_if_needed(name), self.email)
            }
            _ => self.email.clone(),
        }
    }

    /// Short form for list display: display name when present, else the address.
    pub fn display(&self) -> &str {
        match &self.name {
            Some(n) if !n.is_empty() => n.as_str(),
            _ => self.email.as_str(),
        }
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_header())
    }
}

fn quote_if_needed(s: &str) -> String {
    if s.chars()
        .any(|c| matches!(c, '"' | ',' | '<' | '>' | '@' | '\\' | '.' | '[' | ']'))
        || s != s.trim()
    {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

/// Parse a header value such as
/// `"Alice" <a@example.com>, Bob <b@example.com>` into addresses.
///
/// Deliberately forgiving: a malformed header yields whatever could be salvaged
/// rather than an error, because in practice broken `From:` headers are common
/// and must not block rendering an inbox.
pub fn parse_address_list(input: &str) -> Vec<Address> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    // Fast path: no comma at all — a single address.
    if !contains_top_level_comma(trimmed) {
        return parse_one(trimmed).into_iter().collect();
    }

    let mut out = Vec::new();
    let mut buf = String::new();
    let mut depth_angle = 0usize;
    let mut in_quotes = false;
    let mut escaped = false;

    for ch in trimmed.chars() {
        if escaped {
            buf.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_quotes => {
                buf.push(ch);
                escaped = true;
            }
            '"' => {
                in_quotes = !in_quotes;
                buf.push(ch);
            }
            '<' if !in_quotes => {
                depth_angle += 1;
                buf.push(ch);
            }
            '>' if !in_quotes => {
                depth_angle = depth_angle.saturating_sub(1);
                buf.push(ch);
            }
            ',' if !in_quotes && depth_angle == 0 => {
                out.extend(parse_one(buf.trim()));
                buf.clear();
            }
            _ => buf.push(ch),
        }
    }
    out.extend(parse_one(buf.trim()));
    out
}

fn contains_top_level_comma(s: &str) -> bool {
    let mut depth = 0usize;
    let mut in_quotes = false;
    for ch in s.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            '<' if !in_quotes => depth += 1,
            '>' if !in_quotes => depth = depth.saturating_sub(1),
            ',' if !in_quotes && depth == 0 => return true,
            _ => {}
        }
    }
    false
}

fn parse_one(chunk: &str) -> Option<Address> {
    let chunk = chunk.trim().trim_end_matches(';').trim();
    if chunk.is_empty() {
        return None;
    }

    if let Some(start) = chunk.rfind('<') {
        if let Some(end) = chunk[start..].find('>') {
            let email = chunk[start + 1..start + end].trim().to_string();
            let name = chunk[..start].trim().trim_matches('"').trim().to_string();
            if !email.is_empty() {
                let name = if name.is_empty() { None } else { Some(name) };
                return Some(Address { name, email });
            }
        }
    }

    // No angle brackets — the chunk is a bare address.
    let email = chunk.trim_matches('"').trim().to_string();
    if email.contains('@') {
        Some(Address::new(email))
    } else {
        None
    }
}

/// Join addresses for a header line.
pub fn join_addresses(addresses: &[Address]) -> String {
    addresses
        .iter()
        .map(|a| a.to_header())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_grouped_list() {
        let list = parse_address_list(r#""Alice A" <a@example.com>, Bob <b@example.com>, c@d.com"#);
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].name.as_deref(), Some("Alice A"));
        assert_eq!(list[0].email, "a@example.com");
        assert_eq!(list[1].name.as_deref(), Some("Bob"));
        assert_eq!(list[2].email, "c@d.com");
    }

    #[test]
    fn ignores_comma_inside_display_name() {
        let list = parse_address_list(r#""Doe, John" <john@example.com>, x@y.com"#);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name.as_deref(), Some("Doe, John"));
    }

    #[test]
    fn empty_input_yields_nothing() {
        assert!(parse_address_list("   ").is_empty());
    }
}
