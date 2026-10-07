//! Raw-protocol queries for IMAP threading extensions.
//!
//! `imap` 2.4's response parser only knows the RFC 3501 response set: a
//! `* THREAD …` line (RFC 5256) or a FETCH carrying Gmail's `X-GM-THRID`
//! fails to parse, aborts the read, and poisons the session — the tagged line
//! is never consumed, so the next command trips the crate's tag assertion.
//! These queries therefore run on a dedicated connection whose bytes we read
//! ourselves, the same trick `stream.rs` uses for STARTTLS.
//!
//! The connection is throwaway: one per folder sync, and only when the server
//! advertises a threading extension *and* the sync actually fetched new mail.

use std::collections::HashMap;
use std::io::Write;
use std::net::TcpStream;

use jianxin_config::TlsMode;
use jianxin_domain::{MailError, MailErrorCode, Result};

use crate::auth::{imap_quote, xoauth2_payload, AuthProvider, AuthSecret};
use crate::imap::client::ImapEndpoint;
use crate::imap::stream::{expect_greeting, read_line, starttls, wrap_tls, MaybeTlsStream};

/// Server-assigned thread keys per IMAP UID: `gmail-thrid:<id>` or
/// `imap-thread:<uidvalidity>:<uid>`.
pub type ThreadHints = HashMap<u32, String>;

/// Collect thread keys for `uids` from `folder`.
///
/// `use_thread_references` runs `UID THREAD REFERENCES UTF-8 ALL` (RFC 5256)
/// and keys each group by its smallest UID — stable within a UIDVALIDITY
/// epoch because new mail always gets larger UIDs. `use_gmail_thrid` runs
/// Gmail's proprietary FETCH attribute; Gmail's own grouping wins where both
/// apply. The caller has already checked the server capabilities.
pub fn fetch_thread_hints(
    endpoint: &ImapEndpoint,
    auth: &dyn AuthProvider,
    folder: &str,
    uids: &[u32],
    uid_validity: u32,
    use_thread_references: bool,
    use_gmail_thrid: bool,
) -> Result<ThreadHints> {
    let mut raw = RawImap::connect(endpoint, auth)?;
    raw.cmd(&format!("SELECT {}", imap_quote(folder)))?;

    let mut hints: ThreadHints = HashMap::new();

    if use_thread_references {
        let lines = raw.cmd("UID THREAD REFERENCES UTF-8 ALL")?;
        let mut groups: Vec<Vec<u32>> = Vec::new();
        for line in &lines {
            if let Some(mut parsed) = parse_thread_line(line) {
                groups.append(&mut parsed);
            }
        }
        for group in groups {
            if let Some(min) = group.iter().copied().min() {
                let key = format!("imap-thread:{uid_validity}:{min}");
                for uid in group {
                    hints.entry(uid).or_insert_with(|| key.clone());
                }
            }
        }
    }

    if use_gmail_thrid {
        for chunk in uids.chunks(500) {
            let set = chunk
                .iter()
                .map(|uid| uid.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let lines = raw.cmd(&format!("UID FETCH {set} (X-GM-THRID)"))?;
            for line in &lines {
                if let Some((uid, thrid)) = parse_thrid_line(line) {
                    hints.insert(uid, format!("gmail-thrid:{thrid}"));
                }
            }
        }
    }

    Ok(hints)
}

/// A minimal line-oriented IMAP client for commands the `imap` crate cannot
/// parse responses for. Tags are `X<n>` so they can never collide with the
/// crate's `a<n>` tags on the main connection.
struct RawImap {
    stream: MaybeTlsStream,
    tag: u32,
}

impl RawImap {
    fn connect(endpoint: &ImapEndpoint, auth: &dyn AuthProvider) -> Result<Self> {
        let addr = (endpoint.host.as_str(), endpoint.port);
        let tcp = TcpStream::connect(addr).map_err(|e| {
            MailError::new(
                MailErrorCode::NetworkUnavailable,
                format!("cannot connect to {}:{}: {e}", endpoint.host, endpoint.port),
            )
        })?;
        tcp.set_read_timeout(Some(endpoint.timeout)).ok();
        tcp.set_write_timeout(Some(endpoint.timeout)).ok();

        let (stream, greeting_consumed) = match endpoint.tls {
            TlsMode::Tls => (wrap_tls(tcp, &endpoint.host)?, false),
            TlsMode::StartTls => (starttls(tcp, &endpoint.host)?, true),
            TlsMode::None => (MaybeTlsStream::Plain(tcp), false),
        };

        let mut raw = Self { stream, tag: 0 };
        if !greeting_consumed {
            expect_greeting(&mut raw.stream)?;
        }
        raw.authenticate(auth)?;
        Ok(raw)
    }

    fn authenticate(&mut self, auth: &dyn AuthProvider) -> Result<()> {
        match auth.imap_secret()? {
            AuthSecret::Password(password) => self
                .cmd(&format!(
                    "LOGIN {} {}",
                    imap_quote(auth.username()),
                    imap_quote(&password)
                ))
                .map(|_| ()),
            AuthSecret::OAuth2(token) => {
                let tag = self.next_tag();
                self.write_line(&format!("{tag} AUTHENTICATE XOAUTH2"))?;
                let mut sent_payload = false;
                loop {
                    let line = self.read_line()?;
                    if let Some(rest) = line.strip_prefix(&format!("{tag} ")) {
                        return check_status(rest).map(|_| ());
                    }
                    if line.starts_with('+') {
                        if sent_payload {
                            // A second continuation is the base64 error JSON:
                            // acknowledge it and the server replies with NO.
                            self.write_line("")?;
                        } else {
                            self.write_line(&xoauth2_payload(auth.username(), &token))?;
                            sent_payload = true;
                        }
                    }
                }
            }
        }
    }

    /// Run one command and collect its untagged response lines.
    fn cmd(&mut self, command: &str) -> Result<Vec<String>> {
        let tag = self.next_tag();
        self.write_line(&format!("{tag} {command}"))?;
        let mut untagged = Vec::new();
        loop {
            let line = self.read_line()?;
            if let Some(rest) = line.strip_prefix(&format!("{tag} ")) {
                return check_status(rest).map(|_| untagged);
            }
            untagged.push(line);
        }
    }

    fn next_tag(&mut self) -> String {
        self.tag += 1;
        format!("X{}", self.tag)
    }

    fn read_line(&mut self) -> Result<String> {
        read_line(&mut self.stream).map_err(|e| {
            MailError::new(
                MailErrorCode::NetworkUnavailable,
                format!("imap extension read failed: {e}"),
            )
        })
    }

    fn write_line(&mut self, line: &str) -> Result<()> {
        self.stream
            .write_all(line.as_bytes())
            .and_then(|_| self.stream.write_all(b"\r\n"))
            .and_then(|_| self.stream.flush())
            .map_err(|e| {
                MailError::new(
                    MailErrorCode::NetworkUnavailable,
                    format!("imap extension write failed: {e}"),
                )
            })
    }
}

fn check_status(rest: &str) -> Result<()> {
    if rest.to_ascii_uppercase().starts_with("OK") {
        Ok(())
    } else {
        Err(MailError::new(
            MailErrorCode::ProviderUnavailable,
            format!("imap command rejected: {rest}"),
        ))
    }
}

/// Parse `* THREAD (3 5 (4 23) 6)(1)` into its top-level groups, flattening
/// nested children into the enclosing group — the granularity threading needs.
fn parse_thread_line(line: &str) -> Option<Vec<Vec<u32>>> {
    let rest = line.strip_prefix("* THREAD ")?;
    let b = rest.as_bytes();
    let mut i = 0;
    let mut groups = Vec::new();

    while i < b.len() {
        skip_ws(b, &mut i);
        if i >= b.len() {
            break;
        }
        let mut uids = Vec::new();
        if b[i] == b'(' {
            i += 1;
            parse_thread_group(b, &mut i, &mut uids);
        } else {
            parse_thread_number(b, &mut i, &mut uids);
        }
        groups.push(uids);
    }
    Some(groups)
}

fn parse_thread_group(b: &[u8], i: &mut usize, out: &mut Vec<u32>) {
    // The caller has already consumed the opening '('.
    loop {
        skip_ws(b, i);
        if *i >= b.len() {
            return;
        }
        match b[*i] {
            b'(' => {
                *i += 1;
                parse_thread_group(b, i, out);
            }
            b')' => {
                *i += 1;
                return;
            }
            c if c.is_ascii_digit() => parse_thread_number(b, i, out),
            _ => *i += 1,
        }
    }
}

fn parse_thread_number(b: &[u8], i: &mut usize, out: &mut Vec<u32>) {
    let start = *i;
    while *i < b.len() && b[*i].is_ascii_digit() {
        *i += 1;
    }
    if *i > start {
        if let Ok(n) = std::str::from_utf8(&b[start..*i])
            .unwrap_or("")
            .parse::<u32>()
        {
            out.push(n);
        }
    }
}

fn skip_ws(b: &[u8], i: &mut usize) {
    while *i < b.len() && b[*i] == b' ' {
        *i += 1;
    }
}

/// Parse `* 4 FETCH (UID 172339 X-GM-THRID 1723442620624002600)`.
fn parse_thrid_line(line: &str) -> Option<(u32, u64)> {
    if !line.starts_with("* ") {
        return None;
    }
    let upper = line.to_ascii_uppercase();
    if !upper.contains("FETCH") {
        return None;
    }
    let uid = number_after(&upper, "UID ")?;
    let thrid = number_after(&upper, "X-GM-THRID ")?;
    Some((uid as u32, thrid))
}

fn number_after(line: &str, marker: &str) -> Option<u64> {
    let start = line.find(marker)? + marker.len();
    let digits: &str = &line[start..];
    let end = digits
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(digits.len());
    digits[..end].parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flat_thread_response() {
        let groups = parse_thread_line("* THREAD (3 5 4)(1)(2)").unwrap();
        assert_eq!(groups, vec![vec![3, 5, 4], vec![1], vec![2]]);
    }

    #[test]
    fn flattens_nested_thread_groups() {
        let groups = parse_thread_line("* THREAD (3 5 (4 23) 6)(1)").unwrap();
        assert_eq!(groups, vec![vec![3, 5, 4, 23, 6], vec![1]]);
    }

    #[test]
    fn accepts_bare_numbers_and_empty_thread() {
        assert_eq!(
            parse_thread_line("* THREAD 7 8").unwrap(),
            vec![vec![7], vec![8]]
        );
        // RFC 5256: an empty result is `* THREAD` with nothing after it.
        assert_eq!(parse_thread_line("* THREAD"), None);
        assert!(parse_thread_line("* SEARCH 1 2").is_none());
    }

    #[test]
    fn parses_gmail_thrid_lines() {
        assert_eq!(
            parse_thrid_line("* 4 FETCH (UID 172339 X-GM-THRID 1723442620624002600)"),
            Some((172339, 1723442620624002600))
        );
        assert_eq!(
            parse_thrid_line("* 12 FETCH (X-GM-THRID 42 UID 7)"),
            Some((7, 42))
        );
        assert_eq!(parse_thrid_line("* 3 FETCH (UID 9 FLAGS (\\Seen))"), None);
        assert_eq!(parse_thrid_line("* 2 EXISTS"), None);
    }
}
