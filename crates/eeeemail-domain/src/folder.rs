use serde::{Deserialize, Serialize};

/// Local folder id (UUID). The remote name is kept separately in `remote_id`.
pub type FolderId = String;

/// Normalised folder role.
///
/// Never use the raw remote name as a key: providers disagree on `INBOX`,
/// `Inbox`, `收件箱`, `AAMk...`. Everything is mapped to this enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FolderKind {
    Inbox,
    Sent,
    Drafts,
    Trash,
    Spam,
    Archive,
    Custom,
}

impl FolderKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            FolderKind::Inbox => "inbox",
            FolderKind::Sent => "sent",
            FolderKind::Drafts => "drafts",
            FolderKind::Trash => "trash",
            FolderKind::Spam => "spam",
            FolderKind::Archive => "archive",
            FolderKind::Custom => "custom",
        }
    }

    /// Whether this folder should be synced by default.
    pub fn sync_by_default(&self) -> bool {
        !matches!(self, FolderKind::Spam | FolderKind::Trash)
    }

    /// Map a remote folder onto a role.
    ///
    /// `special_use` (IMAP SPECIAL-USE / RFC 6154 attributes such as `\Sent`)
    /// wins when present, because localised names are unreliable. Otherwise fall
    /// back to a name heuristic covering English and the common Chinese
    /// providers (163/126/QQ/aliyun).
    pub fn detect(name: &str, special_use: Option<&str>) -> Self {
        if let Some(flags) = special_use {
            let lower = flags.to_ascii_lowercase();
            if lower.contains("\\sent") {
                return FolderKind::Sent;
            }
            if lower.contains("\\drafts") {
                return FolderKind::Drafts;
            }
            if lower.contains("\\trash") {
                return FolderKind::Trash;
            }
            if lower.contains("\\junk") || lower.contains("\\spam") {
                return FolderKind::Spam;
            }
            if lower.contains("\\all") || lower.contains("\\archive") {
                return FolderKind::Archive;
            }
            if lower.contains("\\inbox") {
                return FolderKind::Inbox;
            }
        }

        // Take the last path segment so `Foo/Bar/Sent Items` still resolves.
        let last = name.rsplit(|c| c == '/' || c == '.').next().unwrap_or(name);
        let n = last.trim().to_ascii_lowercase();

        if n == "inbox" {
            return FolderKind::Inbox;
        }

        for (needles, kind) in [
            (
                vec![
                    "sent",
                    "sent items",
                    "sent mail",
                    "已发送",
                    "发件箱",
                    "已发送邮件",
                ],
                FolderKind::Sent,
            ),
            (
                vec!["drafts", "draft", "草稿", "草稿箱", "草稿夹"],
                FolderKind::Drafts,
            ),
            (
                vec![
                    "trash",
                    "deleted",
                    "deleted items",
                    "bin",
                    "已删除",
                    "已删除邮件",
                    "垃圾箱",
                    "回收站",
                ],
                FolderKind::Trash,
            ),
            (
                vec![
                    "spam",
                    "junk",
                    "junk mail",
                    "bulk",
                    "垃圾邮件",
                    "广告邮件",
                    "订阅邮件",
                ],
                FolderKind::Spam,
            ),
            (
                vec!["archive", "all mail", "归档", "所有邮件", "存档"],
                FolderKind::Archive,
            ),
        ] {
            if needles.iter().any(|needle| n.contains(needle)) {
                return kind;
            }
        }

        FolderKind::Custom
    }
}

/// A folder belonging to one account.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Folder {
    pub id: FolderId,
    pub account_id: crate::AccountId,
    /// Remote name / id, e.g. `INBOX`, `INBOX.Sent`, `AAMkAGI2...`.
    pub remote_id: String,
    pub parent_id: Option<FolderId>,
    pub name: String,
    pub kind: FolderKind,
    pub unread_count: u32,
    pub total_count: u32,
}

impl Folder {
    pub fn new(account_id: &str, remote_id: &str, name: &str, kind: FolderKind) -> Self {
        Self {
            id: crate::new_id(),
            account_id: account_id.to_string(),
            remote_id: remote_id.to_string(),
            parent_id: None,
            name: name.to_string(),
            kind,
            unread_count: 0,
            total_count: 0,
        }
    }

    /// Deterministic local id so repeated syncs are idempotent.
    pub fn deterministic_id(account_id: &str, remote_id: &str) -> FolderId {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        account_id.hash(&mut hasher);
        '\u{1f}'.hash(&mut hasher);
        remote_id.hash(&mut hasher);
        let h = hasher.finish();
        format!("{:016x}{:016x}", h, h.wrapping_mul(0x9E3779B97F4A7C15))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_english_names() {
        assert_eq!(FolderKind::detect("INBOX", None), FolderKind::Inbox);
        assert_eq!(FolderKind::detect("Sent Items", None), FolderKind::Sent);
        assert_eq!(FolderKind::detect("Drafts", None), FolderKind::Drafts);
        assert_eq!(FolderKind::detect("Trash", None), FolderKind::Trash);
        assert_eq!(FolderKind::detect("Junk", None), FolderKind::Spam);
        assert_eq!(FolderKind::detect("All Mail", None), FolderKind::Archive);
    }

    #[test]
    fn detects_chinese_names() {
        assert_eq!(FolderKind::detect("&UXZO1mWHTvZZOQ-", None), FolderKind::Custom); // modified UTF-7, unknown
        assert_eq!(FolderKind::detect("已发送", None), FolderKind::Sent);
        assert_eq!(FolderKind::detect("草稿箱", None), FolderKind::Drafts);
        assert_eq!(FolderKind::detect("垃圾邮件", None), FolderKind::Spam);
    }

    #[test]
    fn special_use_wins() {
        assert_eq!(
            FolderKind::detect("BOGUS-NAME", Some("\\HasNoChildren \\Sent")),
            FolderKind::Sent
        );
    }

    #[test]
    fn nested_paths_use_last_segment() {
        assert_eq!(FolderKind::detect("Foo/Sent Items", None), FolderKind::Sent);
    }
}
