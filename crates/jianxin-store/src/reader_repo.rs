//! Bounded projections for the Web UI: list/header requests never load bodies.
use crate::migrations::db_err;
use jianxin_domain::Result;
use rusqlite::{params, Connection};
use serde::Serialize;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct MailCounts {
    pub total: u64,
    pub unread: u64,
    pub starred: u64,
}

/// Count real local messages across configured accounts, independently of list pages.
/// Use the same account-scoped immutable/header/row identity as the TUI merged view.
pub fn mail_counts(conn: &Connection, allowed_accounts: &[String]) -> Result<MailCounts> {
    conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(unread), 0), COALESCE(SUM(starred), 0) FROM (
            SELECT MAX(CASE WHEN m.is_read = 0 THEN 1 ELSE 0 END) AS unread,
                   MAX(m.is_starred) AS starred
            FROM messages m
            WHERE m.is_deleted = 0 AND m.account_id IN (SELECT value FROM json_each(?1))
            GROUP BY m.account_id, COALESCE(NULLIF(m.immutable_id, ''), m.message_id_header, m.id)
        )",
        [serde_json::to_string(allowed_accounts).unwrap()],
        |row| {
            Ok(MailCounts {
                total: row.get(0)?,
                unread: row.get(1)?,
                starred: row.get(2)?,
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
    pub starred: bool,
    pub query: &'a str,
    pub limit: u32,
    pub offset: u32,
}

/// Local correspondents from configured mailboxes, bounded and case-insensitively deduplicated.
pub fn contacts(
    conn: &Connection,
    allowed_accounts: &[String],
    query: &str,
) -> Result<Vec<jianxin_domain::Address>> {
    let pattern = format!(
        "%{}%",
        query
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    let mut stmt = conn.prepare(
        "SELECT MAX(NULLIF(a.name,'')),a.email FROM addresses a JOIN messages m ON m.id=a.message_id
         WHERE m.is_deleted=0 AND m.account_id IN (SELECT value FROM json_each(?1))
           AND a.email<>'' AND (a.email LIKE ?2 ESCAPE '\\' OR a.name LIKE ?2 ESCAPE '\\')
         GROUP BY lower(a.email) ORDER BY MAX(COALESCE(m.sent_at,m.received_at,0)) DESC,lower(a.email) LIMIT 20"
    ).map_err(db_err)?;
    let rows = stmt
        .query_map(
            params![serde_json::to_string(allowed_accounts).unwrap(), pattern],
            |row| {
                Ok(jianxin_domain::Address {
                    name: row.get(0)?,
                    email: row.get(1)?,
                })
            },
        )
        .map_err(db_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
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
    let sql = format!(
        "WITH selected AS (
           SELECT m.id,m.thread_id,m.snippet,m.sent_at,m.received_at,m.is_read,m.is_starred,
             {} AS dedup_rank FROM messages m LEFT JOIN folders f ON f.id=m.folder_id
           WHERE m.is_deleted=0 AND m.account_id IN (SELECT value FROM json_each(?8))
             AND (?1='' OR m.account_id=?1) AND (?2='' OR m.folder_id=?2)
         ), visible AS (SELECT * FROM selected WHERE dedup_rank=1), summary AS (
           SELECT thread_id,COUNT(*) AS message_count,
             SUM(CASE WHEN is_read=0 THEN 1 ELSE 0 END) AS unread_count,
             MAX(COALESCE(sent_at,received_at,0)) AS last_message_at
           FROM visible GROUP BY thread_id
         )
         SELECT t.id,t.account_id,t.subject,v.message_count,v.unread_count,v.last_message_at,
           COALESCE((SELECT COALESCE(NULLIF(a.name,''),a.email) FROM visible m
             JOIN addresses a ON a.message_id=m.id AND a.role='from'
             WHERE m.thread_id=t.id ORDER BY COALESCE(m.sent_at,m.received_at,0) DESC,m.id,a.position LIMIT 1),''),
           COALESCE((SELECT substr(m.snippet,1,240) FROM visible m WHERE m.thread_id=t.id
             ORDER BY COALESCE(m.sent_at,m.received_at,0) DESC,m.id LIMIT 1),'')
         FROM threads t JOIN summary v ON v.thread_id=t.id
         WHERE (?3=0 OR v.unread_count>0)
           AND (?9=0 OR EXISTS(SELECT 1 FROM selected m WHERE m.thread_id=t.id AND m.is_starred=1))
           AND (?4='' OR t.subject LIKE ?5 ESCAPE '\\' OR EXISTS (
             SELECT 1 FROM visible m WHERE m.thread_id=t.id AND (
               m.snippet LIKE ?5 ESCAPE '\\' OR EXISTS (SELECT 1 FROM addresses a
               WHERE a.message_id=m.id AND a.role='from' AND
                 (a.name LIKE ?5 ESCAPE '\\' OR a.email LIKE ?5 ESCAPE '\\')))))
         ORDER BY v.last_message_at DESC,t.id LIMIT ?6 OFFSET ?7",
        crate::message_repo::DEDUP_RANK
    );
    let mut stmt = conn.prepare(&sql).map_err(db_err)?;
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
                serde_json::to_string(filter.allowed_accounts).unwrap(),
                filter.starred
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
    pub folder_id: String,
    pub thread_id: Option<String>,
    pub subject: String,
    pub sender: String,
    pub sender_email: String,
    pub recipient: String,
    pub timestamp: i64,
    pub is_read: bool,
    pub is_starred: bool,
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
    headers_in_folder(conn, thread_id, message_id, "", limit, offset)
}

pub fn headers_in_folder(
    conn: &Connection,
    thread_id: &str,
    message_id: &str,
    folder_id: &str,
    limit: u32,
    offset: u32,
) -> Result<Vec<ReaderMessage>> {
    let sql = format!(
        "WITH selected AS (
           SELECT m.*, {} AS dedup_rank FROM messages m
           LEFT JOIN folders f ON f.id=m.folder_id
           WHERE m.is_deleted=0 AND ((?1<>'' AND m.thread_id=?1) OR (?2<>'' AND m.id=?2))
             AND (?5='' OR m.folder_id=?5)
         ) SELECT m.id,m.account_id,m.thread_id,m.subject,
           COALESCE((SELECT group_concat(COALESCE(NULLIF(a.name,''),a.email), ', ') FROM addresses a WHERE a.message_id=m.id AND a.role='from'),''),
           COALESCE((SELECT group_concat(a.email, ', ') FROM addresses a WHERE a.message_id=m.id AND a.role='to'),''),
           COALESCE(m.sent_at,m.received_at,0),m.is_read,COALESCE(substr(m.snippet,1,240),''),
           (SELECT count(*) FROM attachments a WHERE a.message_id=m.id),m.folder_id,
           EXISTS(SELECT 1 FROM messages copy WHERE copy.account_id=m.account_id AND copy.is_deleted=0
             AND copy.is_starred=1 AND COALESCE(NULLIF(copy.immutable_id,''),copy.message_id_header,copy.id)
               =COALESCE(NULLIF(m.immutable_id,''),m.message_id_header,m.id)),
           COALESCE((SELECT group_concat(a.email, ', ') FROM addresses a WHERE a.message_id=m.id AND a.role='from'),'')
         FROM selected m WHERE dedup_rank=1
         ORDER BY COALESCE(m.sent_at,m.received_at,0),m.id LIMIT ?3 OFFSET ?4",
        crate::message_repo::DEDUP_RANK
    );
    let mut stmt = conn.prepare(&sql).map_err(db_err)?;
    let rows = stmt
        .query_map(
            params![thread_id, message_id, limit.min(101), offset, folder_id],
            |r| {
                Ok(ReaderMessage {
                    id: r.get(0)?,
                    account_id: r.get(1)?,
                    folder_id: r.get(10)?,
                    thread_id: r.get(2)?,
                    subject: r.get(3)?,
                    sender: r.get(4)?,
                    sender_email: r.get(12)?,
                    recipient: r.get(5)?,
                    timestamp: r.get(6)?,
                    is_read: r.get(7)?,
                    is_starred: r.get(11)?,
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

/// Folder views count only visible mail, while an unscoped thread retains its full history.
pub fn scope_thread(conn: &Connection, thread: &mut jianxin_domain::Thread, folder_id: &str) -> Result<()> {
    if folder_id.is_empty() { return Ok(()); }
    let sql = format!(
        "SELECT COUNT(*),COALESCE(SUM(CASE WHEN is_read=0 THEN 1 ELSE 0 END),0),
           COALESCE(MAX(COALESCE(sent_at,received_at,0)),0) FROM (
           SELECT m.is_read,m.sent_at,m.received_at,{} AS dedup_rank
           FROM messages m LEFT JOIN folders f ON f.id=m.folder_id
           WHERE m.thread_id=?1 AND m.folder_id=?2 AND m.is_deleted=0
         ) WHERE dedup_rank=1",
        crate::message_repo::DEDUP_RANK
    );
    let (count, unread, last) = conn.query_row(&sql, params![thread.id, folder_id], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    }).map_err(db_err)?;
    thread.message_count = count; thread.unread_count = unread; thread.last_message_at = last;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{message_repo, test_util, thread_repo, Database};
    use jianxin_domain::{FolderKind, Message};

    #[test]
    fn folder_views_project_only_their_messages_before_sort_search_counts_and_pagination() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.lock();
        test_util::seed_account(&conn, "a").unwrap();
        for (id, kind) in [("inbox", FolderKind::Inbox), ("sent", FolderKind::Sent), ("all", FolderKind::Archive)] {
            test_util::seed_folder(&conn, id, "a", id, kind).unwrap();
        }
        let thread = thread_repo::create(&conn, &"a".into(), "Meeting").unwrap();
        let other = thread_repo::create(&conn, &"a".into(), "Other").unwrap();
        let sent_only = thread_repo::create(&conn, &"a".into(), "Only sent").unwrap();
        let mut root = Message::new();
        root.account_id = "a".into(); root.folder_id = "inbox".into(); root.remote_id = "1".into(); root.thread_id = Some(thread.clone());
        root.from = vec![jianxin_domain::Address::with_name("Sender", "sender@example.com")];
        root.sent_at = Some(100); root.snippet = Some("received content".into());
        let mut reply = root.clone();
        reply.id = "sent-reply".into(); reply.folder_id = "sent".into(); reply.remote_id = "2".into(); reply.sent_at = Some(300);
        reply.from = vec![jianxin_domain::Address::with_name("Myself", "owner@example.com")];
        reply.message_id_header = Some("<reply@example.com>".into()); reply.snippet = Some("outgoing-only keyword".into()); reply.is_read = true;
        let mut copy = reply.clone(); copy.id = "all-copy".into(); copy.folder_id = "all".into();
        let mut newer_received = root.clone(); newer_received.id = "newer".into(); newer_received.remote_id = "3".into(); newer_received.thread_id = Some(other.clone()); newer_received.sent_at = Some(200);
        let mut only = reply.clone(); only.id = "only-sent".into(); only.remote_id = "4".into(); only.thread_id = Some(sent_only.clone()); only.message_id_header = None;
        let mut deleted = root.clone(); deleted.id = "deleted-inbox".into(); deleted.remote_id = "5".into(); deleted.thread_id = Some(sent_only); deleted.is_deleted = true;
        message_repo::upsert_many(&conn, &[root.clone(), reply.clone(), copy, newer_received, only, deleted]).unwrap();
        let allowed = vec!["a".into()];
        let mut filter = Filter { account_id: "a", allowed_accounts: &allowed, folder_id: "inbox", unread: false, starred: false, query: "", limit: 1, offset: 0 };
        assert_eq!(conversations(&conn, &filter).unwrap()[0].id, other);
        filter.offset = 1;
        let rows = conversations(&conn, &filter).unwrap();
        assert_eq!(rows.len(), 1); assert_eq!(rows[0].id, thread); assert_eq!(rows[0].sender, "Sender");
        assert_eq!(rows[0].snippet, "received content"); assert_eq!(rows[0].last_message_at, 100);
        assert_eq!(rows[0].message_count, 1); assert_eq!(rows[0].unread_count, 1);
        filter.offset = 2; assert!(conversations(&conn, &filter).unwrap().is_empty());
        filter.offset = 0; filter.query = "outgoing-only"; assert!(conversations(&conn, &filter).unwrap().is_empty());
        filter.folder_id = "sent"; assert!(!conversations(&conn, &filter).unwrap().is_empty());
        assert_eq!(headers_in_folder(&conn, &thread, "", "inbox", 50, 0).unwrap()[0].id, root.id);
        assert_eq!(headers_in_folder(&conn, &thread, "", "sent", 50, 0).unwrap()[0].id, reply.id);
        assert_eq!(headers(&conn, &thread, "", 50, 0).unwrap().len(), 2);
        let mut detail = thread_repo::get(&conn, &thread).unwrap().unwrap();
        scope_thread(&conn, &mut detail, "inbox").unwrap();
        assert_eq!(detail.message_count, 1); assert_eq!(detail.unread_count, 1); assert_eq!(detail.last_message_at, 100);
    }

    #[test]
    fn contact_search_is_local_deduplicated_literal_and_account_scoped() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.lock();
        for account in ["visible", "removed"] {
            test_util::seed_account(&conn, account).unwrap();
            test_util::seed_folder(&conn, account, account, "INBOX", FolderKind::Inbox).unwrap();
            let mut message = Message::new();
            message.account_id = account.into();
            message.folder_id = account.into();
            message.remote_id = "1".into();
            message.from = vec![jianxin_domain::Address::with_name(
                "Alice 100%",
                format!("{account}@example.com"),
            )];
            message.to = message.from.clone();
            message_repo::upsert_many(&conn, &[message]).unwrap();
        }
        let result = contacts(&conn, &["visible".into()], "alice").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].email, "visible@example.com");
        assert_eq!(contacts(&conn, &["visible".into()], "%").unwrap().len(), 1);
        assert!(contacts(&conn, &[], "Alice").unwrap().is_empty());
        assert!(contacts(&conn, &["visible".into()], "removed")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn conversation_copies_collapse_before_pagination_and_repair_existing_counters() {
        let db = Database::open_in_memory().unwrap();
        let mut conn = db.lock();
        test_util::seed_account(&conn, "a").unwrap();
        test_util::seed_folder(&conn, "inbox", "a", "INBOX", FolderKind::Inbox).unwrap();
        test_util::seed_folder(&conn, "all", "a", "All Mail", FolderKind::Archive).unwrap();
        let thread = thread_repo::create(&conn, &"a".into(), "Meeting").unwrap();
        let mut original = Message::new();
        original.account_id = "a".into();
        original.folder_id = "inbox".into();
        original.remote_id = "1".into();
        original.thread_id = Some(thread.clone());
        original.subject = "Meeting".into();
        original.text_body = Some("Same content".into());
        original.message_id_header = Some("<original@example.com>".into());
        original.sent_at = Some(100);
        let mut copy = original.clone();
        copy.id = jianxin_domain::new_id();
        copy.folder_id = "all".into();
        let mut distinct = original.clone();
        distinct.id = jianxin_domain::new_id();
        distinct.remote_id = "2".into();
        distinct.message_id_header = Some("<distinct@example.com>".into());
        distinct.sent_at = Some(200);
        message_repo::upsert_many(&conn, &[original.clone(), copy.clone(), distinct.clone()])
            .unwrap();
        assert_eq!(
            headers(&conn, &thread, "", 1, 0).unwrap()[0].id,
            original.id
        );
        assert_eq!(
            headers(&conn, &thread, "", 1, 1).unwrap()[0].id,
            distinct.id
        );
        assert!(headers(&conn, &thread, "", 1, 2).unwrap().is_empty());
        assert_eq!(headers(&conn, "", &copy.id, 1, 0).unwrap()[0].id, copy.id);
        assert_eq!(
            thread_repo::message_ids(&conn, &thread).unwrap(),
            [original.id, distinct.id]
        );
        conn.execute("UPDATE threads SET message_count=3,unread_count=3", [])
            .unwrap();
        conn.pragma_update(None, "user_version", 5).unwrap();
        crate::migrations::migrate(&mut conn).unwrap();
        let repaired = thread_repo::get(&conn, &thread).unwrap().unwrap();
        assert_eq!(repaired.message_count, 2);
        assert_eq!(repaired.unread_count, 2);
    }

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
        copy.is_starred = true;
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
        deleted.is_starred = true;
        crate::message_repo::upsert_many(
            &conn,
            &[original.clone(), copy.clone(), other_account, deleted],
        )
        .unwrap();
        assert_eq!(
            mail_counts(&conn, &[]).unwrap(),
            MailCounts {
                total: 0,
                unread: 0,
                starred: 0
            }
        );
        assert_eq!(
            mail_counts(&conn, &["a".into()]).unwrap(),
            MailCounts {
                total: 1,
                unread: 1,
                starred: 1
            }
        );
        assert_eq!(
            mail_counts(&conn, &["a".into(), "b".into()]).unwrap(),
            MailCounts {
                total: 2,
                unread: 2,
                starred: 1
            }
        );
        crate::message_repo::set_read(&conn, &copy.id, true).unwrap();
        assert!(headers(&conn, "", &original.id, 1, 0).unwrap()[0].is_starred);
        assert_eq!(
            mail_counts(&conn, &["a".into()]).unwrap(),
            MailCounts {
                total: 1,
                unread: 0,
                starred: 1
            }
        );
        crate::message_repo::set_starred(&conn, &original.id, false).unwrap();
        assert!(!crate::message_repo::get(&conn, &copy.id).unwrap().unwrap().is_starred);
        assert_eq!(mail_counts(&conn, &["a".into()]).unwrap().starred, 0);
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
            message.from = vec![jianxin_domain::Address::with_name("Sender", "sender@example.com")];
            message.text_body = Some("private body".repeat(1000));
            message.html_body = Some("<p>private HTML</p>".repeat(1000));
            message.is_read = index % 2 == 0;
            message.is_starred = index % 3 == 0;
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
            starred: false,
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
        filter.starred = true;
        assert_eq!(conversations(&conn, &filter).unwrap().len(), 20);
        filter.limit = 10;
        filter.offset = 10;
        assert_eq!(conversations(&conn, &filter).unwrap().len(), 10);
        filter.offset = 20;
        assert!(conversations(&conn, &filter).unwrap().is_empty());
        filter.starred = false;
        filter.offset = 0;
        filter.limit = 51;
        filter.query = "%_";
        assert_eq!(conversations(&conn, &filter).unwrap().len(), 51);
        filter.query = "%missing";
        assert!(conversations(&conn, &filter).unwrap().is_empty());
        filter.query = "";
        filter.folder_id = "fb";
        assert!(conversations(&conn, &filter).unwrap().is_empty());
        let header = headers(&conn, "", &first_id, 1, 0).unwrap();
        assert_eq!(header.len(), 1);
        assert_eq!(header[0].folder_id, "fa");
        assert_eq!(header[0].sender, "Sender");
        assert_eq!(header[0].sender_email, "sender@example.com");
        assert!(header[0].is_starred);
        let json = serde_json::to_string(&header).unwrap();
        assert!(!json.contains("private"));
        assert!(!json.contains("text_body"));
        assert!(!json.contains("html_body"));
    }
}
