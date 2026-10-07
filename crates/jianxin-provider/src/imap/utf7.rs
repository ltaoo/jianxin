//! Modified UTF-7 (RFC 3501 §5.1.3) decoding for IMAP mailbox names.
//!
//! Mailbox names carry non-ASCII inside `&<modified-base64>-` shifts, where
//! the payload is big-endian UTF-16 and `/` is replaced by `,`. Gmail and the
//! Chinese providers use it for every localized folder (`&XfJT0ZAB-` is
//! `已发送`), so without decoding the folder pane shows the wire format
//! instead of readable text.

/// Decode a mailbox name. Anything that is not valid modified UTF-7 passes
/// through unchanged, so plain ASCII names come back as-is and a malformed
/// shift never corrupts the rest of the string.
pub fn decode(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut rest = name;

    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp + 1..];

        // A second `&` before the closing `-` means the shift was never
        // opened; keep this `&` as a literal and rescan from the next one.
        let Some(end) = tail.find(['&', '-']) else {
            out.push('&');
            out.push_str(tail);
            return out;
        };
        if tail.as_bytes()[end] == b'&' {
            out.push('&');
            rest = tail;
            continue;
        }

        let shifted = &tail[..end];
        if shifted.is_empty() {
            out.push('&'); // `&-` encodes a literal ampersand
        } else {
            match decode_shift(shifted) {
                Some(text) => out.push_str(&text),
                None => {
                    out.push('&');
                    out.push_str(shifted);
                    out.push('-');
                }
            }
        }
        rest = &tail[end + 1..];
    }

    out.push_str(rest);
    out
}

/// One `&…-` shift: modified base64 of big-endian UTF-16 code units.
fn decode_shift(shifted: &str) -> Option<String> {
    let mut units = Vec::with_capacity(shifted.len() / 3 + 1);
    let mut buffer: u32 = 0;
    let mut bits = 0;

    for &byte in shifted.as_bytes() {
        let value = base64_value(byte)? as u32;
        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 16 {
            bits -= 16;
            units.push((buffer >> bits) as u16);
            buffer &= (1 << bits) - 1;
        }
    }

    // Whole UTF-16 units leave 0, 2 or 4 unused bits; more means the payload
    // is not a whole number of code units and therefore not modified UTF-7.
    if bits >= 6 {
        return None;
    }
    String::from_utf16(&units).ok()
}

fn base64_value(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        // RFC 3501 swaps `/` for `,` so mailbox names stay delimiter-safe.
        b',' => Some(63),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_localized_folder_names() {
        // Real Gmail `LIST` output for a Chinese-language account.
        assert_eq!(decode("&XfJT0ZAB-"), "已发送");
        assert_eq!(decode("&YkBnCZCuTvY-"), "所有邮件");
        assert_eq!(decode("&g0l6Pw-"), "草稿");
        assert_eq!(decode("&V4NXPpCuTvY-"), "垃圾邮件");
        assert_eq!(decode("&XfJSoGYfaAc-"), "已加星标");
        assert_eq!(decode("&kc2JgQ-"), "重要");
        assert_eq!(decode("[Gmail]/&XfJT0ZAB-"), "[Gmail]/已发送");
    }

    #[test]
    fn passes_plain_names_through() {
        assert_eq!(decode("INBOX"), "INBOX");
        assert_eq!(decode("[Gmail]/Sent Mail"), "[Gmail]/Sent Mail");
        assert_eq!(decode("已发送"), "已发送");
    }

    #[test]
    fn handles_ampersand_escapes() {
        assert_eq!(decode("&-"), "&");
        assert_eq!(decode("R&D"), "R&D");
        assert_eq!(decode("AT&&-T"), "AT&&T");
    }

    #[test]
    fn leaves_malformed_shifts_untouched() {
        // Valid base64 alphabet but not whole UTF-16 units.
        assert_eq!(decode("Tom&Jerry-"), "Tom&Jerry-");
        // Unterminated shift.
        assert_eq!(decode("a&XfT"), "a&XfT");
        // Non-base64 payload.
        assert_eq!(decode("a&!=b-"), "a&!=b-");
    }

    #[test]
    fn decodes_text_around_multiple_shifts() {
        assert_eq!(decode("x&g0l6Pw-&kc2JgQ-y"), "x草稿重要y");
    }
}
