use serde::{Deserialize, Serialize};

pub type ThreadId = String;

/// Local conversation grouping.
///
/// Graph exposes `conversationId` and Gmail a `threadId`, but IMAP has neither.
/// Threads are therefore a *local* concern assembled from `Message-ID` /
/// `In-Reply-To` / `References` so all providers end up with the same model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thread {
    pub id: ThreadId,
    pub account_id: crate::AccountId,
    pub subject: String,
    pub message_ids: Vec<crate::MessageId>,
    pub last_message_at: i64,
    pub unread_count: u32,
}

impl Thread {
    pub fn new(account_id: &str, subject: impl Into<String>) -> Self {
        Self {
            id: crate::new_id(),
            account_id: account_id.to_string(),
            subject: subject.into(),
            message_ids: Vec::new(),
            last_message_at: 0,
            unread_count: 0,
        }
    }
}

/// Strip the decorative parts of a subject so replies collapse into one thread:
/// `Re:`, `Fwd:`, leading whitespace and the `[list-tag]` prefix.
pub fn normalise_subject(subject: &str) -> String {
    let mut s = subject.trim().to_string();
    loop {
        let lower = s.to_ascii_lowercase();
        let mut changed = false;
        for prefix in ["re:", "aw:", "fw:", "fwd:", "答复:", "转发:"] {
            if lower.starts_with(prefix) {
                s = s[prefix.len()..].trim_start().to_string();
                changed = true;
                break;
            }
        }
        if !changed {
            break;
        }
    }
    s.trim().to_string()
}

/// Build the `References` header value for a reply: existing references plus the
/// parent `Message-ID`, capped so the header does not grow without bound.
pub fn build_references(parent: &crate::Message) -> Vec<String> {
    let mut refs = parent.references.clone();
    if let Some(id) = &parent.message_id_header {
        if !refs.iter().any(|r| r == id) {
            refs.push(id.clone());
        }
    }
    const MAX: usize = 20;
    if refs.len() > MAX {
        refs = refs.split_off(refs.len() - MAX);
    }
    refs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_reply_prefixes() {
        assert_eq!(normalise_subject("Re: Re: Hello"), "Hello");
        assert_eq!(normalise_subject("Fwd:  Hello "), "Hello");
        assert_eq!(normalise_subject("Hello"), "Hello");
    }
}
