//! Bounded projections for the Web UI: list/header requests never load bodies.
use crate::migrations::db_err;
use jianxin_domain::Result;
use rusqlite::{params, Connection};
use serde::Serialize;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct MailCounts {
    pub total: u64,
    pub unread: u64,
}

/// Count real local messages across configured accounts, independently of list pages.
/// Use the same account-scoped immutable/header/row identity as the TUI merged view.
pub fn mail_counts(conn: &Connection, allowed_accounts: &[String]) -> Result<MailCounts> {
    conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(unread), 0) FROM (
            SELECT MAX(CASE WHEN m.is_read = 0 THEN 1 ELSE 0 END) AS unread
            FROM messages m
            WHERE m.is_deleted = 0 AND m.account_id IN (SELECT value FROM json_each(?1))
            GROUP BY m.account_id, COALESCE(NULLIF(m.immutable_id, ''), m.message_id_header, m.id)
        )",
        [serde_json::to_string(allowed_accounts).unwrap()],
        |row| {
            Ok(MailCounts {
                total: row.get(0)?,
                unread: row.get(1)?,
            })
        },
    )
    .map_err(db_err)
}

#[derive(Debug, Serialize)]
pub struct Conversation {
    pub id: String,
    pub account_id: String,
    pub subject: String,
    pub message_count: u32,
    pub unread_count: u32,
    pub last_message_at: i64,
    pub sender: String,
    pub snippet: String,
}

pub struct Filter<'a> {
    pub account_id: &'a str,
    pub allowed_accounts: &'a [String],
    pub folder_id: &'a str,
    pub unread: bool,
    pub query: &'a str,
    pub limit: u32,
    pub offset: u32,
}

pub fn conversations(conn: &Connection, filter: &Filter<'_>) -> Result<Vec<Conversation>> {
    // LIKE metacharacters are literal user text, not wildcards.
    let query = format!(
        "%{}%",
        filter
            .query
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    let mut stmt = conn.prepare(
        "SELECT t.id, t.account_id, t.subject, t.message_count, t.unread_count, t.last_message_at,
           COALESCE((SELECT COALESCE(NULLIF(a.name, ''), a.email) FROM messages m
             JOIN addresses a ON a.message_id=m.id AND a.role='from'
             WHERE m.thread_id=t.id ORDER BY COALESCE(m.sent_at,m.received_at,0) DESC, m.id, a.position LIMIT 1), ''),
           COALESCE((SELECT substr(m.snippet,1,240) FROM messages m WHERE m.thread_id=t.id
             ORDER BY COALESCE(m.sent_at,m.received_at,0) DESC, m.id LIMIT 1), '')
         FROM threads t WHERE t.account_id IN (SELECT value FROM json_each(?8)) AND (?1='' OR t.account_id=?1)
           AND (?2='' OR EXISTS(SELECT 1 FROM messages m WHERE m.thread_id=t.id AND m.folder_id=?2))
           AND (?3=0 OR t.unread_count>0)
           AND (?4='' OR t.subject LIKE ?5 ESCAPE '\\' OR EXISTS (
             SELECT 1 FROM messages m WHERE m.thread_id=t.id AND (
               m.snippet LIKE ?5 ESCAPE '\\' OR EXISTS (SELECT 1 FROM addresses a
               WHERE a.message_id=m.id AND a.role='from' AND
                 (a.name LIKE ?5 ESCAPE '\\' OR a.email LIKE ?5 ESCAPE '\\')))))
         ORDER BY t.last_message_at DESC, t.id LIMIT ?6 OFFSET ?7"
    ).map_err(db_err)?;
    let rows = stmt
        .query_map(
            params![
                filter.account_id,
                filter.folder_id,
                filter.unread,
                filter.query,
                query,
                filter.limit.min(101),
                filter.offset,
                serde_json::to_string(filter.allowed_accounts).unwrap()
            ],
            |r| {
                Ok(Conversation {
                    id: r.get(0)?,
                    account_id: r.get(1)?,
                    subject: r.get(2)?,
                    message_count: r.get(3)?,
                    unread_count: r.get(4)?,
                    last_message_at: r.get(5)?,
                    sender: r.get(6)?,
                    snippet: r.get(7)?,
                })
            },
        )
        .map_err(db_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

#[derive(Debug, Serialize)]
pub struct ReaderMessage {
    pub id: String,
    pub account_id: String,
    pub thread_id: Option<String>,
    pub subject: String,
    pub sender: String,
    pub recipient: String,
    pub timestamp: i64,
    pub is_read: bool,
    pub snippet: String,
    pub attachment_count: u32,
}

pub fn headers(
    conn: &Connection,
    thread_id: &str,
    message_id: &str,
    limit: u32,
    offset: u32,
) -> Result<Vec<ReaderMessage>> {
    let mut stmt = conn.prepare(
        "SELECT m.id,m.account_id,m.thread_id,m.subject,
           COALESCE((SELECT group_concat(COALESCE(NULLIF(a.name,''),a.email), ', ') FROM addresses a WHERE a.message_id=m.id AND a.role='from'),''),
           COALESCE((SELECT group_concat(a.email, ', ') FROM addresses a WHERE a.message_id=m.id AND a.role='to'),''),
           COALESCE(m.sent_at,m.received_at,0),m.is_read,COALESCE(substr(m.snippet,1,240),''),
           (SELECT count(*) FROM attachments a WHERE a.message_id=m.id)
         FROM messages m WHERE (?1<>'' AND m.thread_id=?1) OR (?2<>'' AND m.id=?2)
         ORDER BY COALESCE(m.sent_at,m.received_at,0),m.id LIMIT ?3 OFFSET ?4"
    ).map_err(db_err)?;
    let rows = stmt
        .query_map(
            params![thread_id, message_id, limit.min(101), offset],
            |r| {
                Ok(ReaderMessage {
                    id: r.get(0)?,
                    account_id: r.get(1)?,
                    thread_id: r.get(2)?,
                    subject: r.get(3)?,
                    sender: r.get(4)?,
                    recipient: r.get(5)?,
                    timestamp: r.get(6)?,
                    is_read: r.get(7)?,
                    snippet: r.get(8)?,
                    attachment_count: r.get(9)?,
                })
            },
        )
        .map_err(db_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{message_repo, test_util, thread_repo, Database};
    use jianxin_domain::{FolderKind, Message};

    #[test]
    fn global_counts_collapse_folder_copies_exclude_deleted_and_scope_accounts() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.lock();
        for account in ["a", "b"] {
            test_util::seed_account(&conn, account).unwrap();
            test_util::seed_folder(
                &conn,
                &format!("{account}-inbox"),
                account,
                "INBOX",
                FolderKind::Inbox,
            )
            .unwrap();
        }
        test_util::seed_folder(&conn, "a-archive", "a", "Archive", FolderKind::Archive).unwrap();
        let mut original = Message::new();
        original.account_id = "a".into();
        original.folder_id = "a-inbox".into();
        original.remote_id = "1".into();
        original.message_id_header = Some("<same@example.com>".into());
        original.is_read = true;
        let mut copy = original.clone();
        copy.id = jianxin_domain::new_id();
        copy.folder_id = "a-archive".into();
        copy.is_read = false;
        let mut other_account = original.clone();
        other_account.id = jianxin_domain::new_id();
        other_account.account_id = "b".into();
        other_account.folder_id = "b-inbox".into();
        other_account.is_read = false;
        let mut deleted = original.clone();
        deleted.id = jianxin_domain::new_id();
        deleted.remote_id = "2".into();
        deleted.message_id_header = None;
        deleted.is_deleted = true;
        crate::message_repo::upsert_many(
            &conn,
            &[original.clone(), copy.clone(), other_account, deleted],
        )
        .unwrap();
        assert_eq!(
            mail_counts(&conn, &[]).unwrap(),
            MailCounts {
                total: 0,
                unread: 0
            }
        );
        assert_eq!(
            mail_counts(&conn, &["a".into()]).unwrap(),
            MailCounts {
                total: 1,
                unread: 1
            }
        );
        assert_eq!(
            mail_counts(&conn, &["a".into(), "b".into()]).unwrap(),
            MailCounts {
                total: 2,
                unread: 2
            }
        );
        crate::message_repo::set_read(&conn, &copy.id, true).unwrap();
        assert_eq!(
            mail_counts(&conn, &["a".into()]).unwrap(),
            MailCounts {
                total: 1,
                unread: 0
            }
        );
    }

    #[test]
    fn projections_are_paginated_account_scoped_and_treat_search_as_literal() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.lock();
        test_util::seed_account(&conn, "a").unwrap();
        test_util::seed_account(&conn, "b").unwrap();
        test_util::seed_folder(&conn, "fa", "a", "INBOX", FolderKind::Inbox).unwrap();
        test_util::seed_folder(&conn, "fb", "b", "INBOX", FolderKind::Inbox).unwrap();
        let mut first_id = String::new();
        for index in 0..61 {
            let account = if index == 60 { "b" } else { "a" };
            let id = thread_repo::create(&conn, &account.to_string(), "100%_literal").unwrap();
            let mut message = Message::new();
            message.account_id = account.into();
            message.folder_id = if account == "a" { "fa" } else { "fb" }.into();
            message.remote_id = index.to_string();
            message.thread_id = Some(id.clone());
            message.subject = "100%_literal".into();
            message.text_body = Some("private body".repeat(1000));
            message.html_body = Some("<p>private HTML</p>".repeat(1000));
            message.is_read = index % 2 == 0;
            if index == 0 {
                first_id = message.id.clone();
            }
            message_repo::upsert_many(&conn, std::slice::from_ref(&message)).unwrap();
            thread_repo::recompute(&conn, &id).unwrap();
        }
        let allowed = vec!["a".to_string()];
        let mut filter = Filter {
            account_id: "",
            allowed_accounts: &allowed,
            folder_id: "",
            unread: false,
            query: "",
            limit: 51,
            offset: 0,
        };
        assert_eq!(conversations(&conn, &filter).unwrap().len(), 51);
        filter.offset = 50;
        assert_eq!(conversations(&conn, &filter).unwrap().len(), 10);
        filter.offset = 0;
        filter.unread = true;
        assert_eq!(conversations(&conn, &filter).unwrap().len(), 30);
        filter.unread = false;
        filter.query = "%_";
        assert_eq!(conversations(&conn, &filter).unwrap().len(), 51);
        filter.query = "%missing";
        assert!(conversations(&conn, &filter).unwrap().is_empty());
        filter.query = "";
        filter.folder_id = "fb";
        assert!(conversations(&conn, &filter).unwrap().is_empty());
        let header = headers(&conn, "", &first_id, 1, 0).unwrap();
        assert_eq!(header.len(), 1);
        let json = serde_json::to_string(&header).unwrap();
        assert!(!json.contains("private"));
        assert!(!json.contains("text_body"));
        assert!(!json.contains("html_body"));
    }
}
