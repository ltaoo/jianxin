//! Local conversation grouping.
//!
//! Graph exposes `conversationId` and Gmail a `threadId`, but IMAP has neither.
//! Threads are therefore assembled locally from `Message-ID` / `In-Reply-To` /
//! `References`, with provider thread keys, the Outlook `Thread-Index` root and
//! a normalised-subject fallback, so every provider ends up with the same
//! model (§17).

use jianxin_domain::thread::normalise_subject;
use jianxin_domain::{AccountId, Message, MessageId, Result, Thread, ThreadId};
use rusqlite::{Connection, OptionalExtension};

use crate::migrations::db_err;

/// Assign a `thread_id` to every message that does not have one yet.
///
/// Resolution order:
///
/// 1. an ancestor named in `In-Reply-To` / `References` that we already know,
/// 2. another message carrying the same server thread key (Gmail
///    `X-GM-THRID`, an IMAP `THREAD=REFERENCES` group),
/// 3. another message with the same Outlook `Thread-Index` root,
/// 4. an existing thread in the same account with the same normalised subject,
/// 5. a brand new thread.
pub fn assign(conn: &Connection, account_id: &AccountId, messages: &mut [Message]) -> Result<()> {
    if messages.is_empty() {
        return Ok(());
    }

    // Roots before replies. A reply can only find its parent if the parent has
    // a thread already, and an IMAP fetch hands us a folder in arrival order —
    // often the reply before the message it answers.
    let mut order: Vec<usize> = (0..messages.len()).collect();
    order.sort_by_key(|&i| {
        let m = &messages[i];
        m.references.len() + usize::from(m.in_reply_to.is_some())
    });

    for i in order {
        if messages[i].thread_id.is_some() {
            continue;
        }

        let mut thread_id = ancestor_thread(conn, account_id, &messages[i])?;

        if thread_id.is_none() {
            if let Some(key) = &messages[i].server_thread_key {
                thread_id = keyed_thread(conn, account_id, "server_thread_key", key)?;
            }
        }

        if thread_id.is_none() {
            if let Some(root) = &messages[i].thread_index_root {
                thread_id = keyed_thread(conn, account_id, "thread_index_root", root)?;
            }
        }

        if thread_id.is_none() {
            let subject = normalise_subject(&messages[i].subject);
            if !subject.is_empty() {
                thread_id = conn
                    .query_row(
                        "SELECT thread_id FROM messages
                          WHERE account_id = ?1 AND subject_normalized = ?2
                            AND thread_id IS NOT NULL
                          LIMIT 1",
                        rusqlite::params![account_id, subject],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(db_err)?;
            }

            if thread_id.is_none() {
                let id = create(
                    conn,
                    account_id,
                    &subject_or_placeholder(&messages[i].subject),
                )?;
                thread_id = Some(id);
            }
        }

        messages[i].thread_id = thread_id.clone();
        if let Some(t) = thread_id {
            conn.execute(
                "UPDATE messages SET thread_id = ?2 WHERE id = ?1",
                rusqlite::params![messages[i].id, t],
            )
            .map_err(db_err)?;
        }
    }
    Ok(())
}

/// Look up the thread of any message that already carries `column = key`.
///
/// `column` is a fixed identifier from this module, never user input.
fn keyed_thread(
    conn: &Connection,
    account_id: &AccountId,
    column: &str,
    key: &str,
) -> Result<Option<ThreadId>> {
    let sql = format!(
        "SELECT thread_id FROM messages
          WHERE account_id = ?1 AND {column} = ?2 AND thread_id IS NOT NULL
          LIMIT 1"
    );
    conn.query_row(&sql, rusqlite::params![account_id, key], |r| {
        r.get::<_, String>(0)
    })
    .optional()
    .map_err(db_err)
}

/// Look up the thread of any ancestor referenced by this message.
fn ancestor_thread(
    conn: &Connection,
    account_id: &AccountId,
    m: &Message,
) -> Result<Option<ThreadId>> {
    // Walk newest reference first: `References` grows upwards and the last
    // entry is the immediate parent.
    let mut candidates: Vec<&str> = Vec::new();
    if let Some(r) = &m.in_reply_to {
        candidates.push(r.as_str());
    }
    for r in m.references.iter().rev().take(10) {
        candidates.push(r.as_str());
    }

    for candidate in candidates {
        let found: Option<ThreadId> = conn
            .query_row(
                "SELECT thread_id FROM messages
                  WHERE account_id = ?1 AND message_id_header = ?2 AND thread_id IS NOT NULL
                  LIMIT 1",
                rusqlite::params![account_id, candidate],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(db_err)?;
        if found.is_some() {
            return Ok(found);
        }
    }
    Ok(None)
}

pub fn create(conn: &Connection, account_id: &AccountId, subject: &str) -> Result<ThreadId> {
    let id = jianxin_domain::new_id();
    conn.execute(
        "INSERT INTO threads (id, account_id, subject, message_count, unread_count, last_message_at, updated_at)
         VALUES (?1, ?2, ?3, 0, 0, 0, ?4)",
        rusqlite::params![id, account_id, subject, jianxin_domain::now_ts()],
    )
    .map_err(db_err)?;
    Ok(id)
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<Thread>> {
    conn.query_row(
        "SELECT id, account_id, subject, message_count, unread_count, last_message_at
           FROM threads WHERE id = ?1",
        [id],
        row_to_thread,
    )
    .optional()
    .map_err(db_err)
}

pub fn list(
    conn: &Connection,
    account_id: &str,
    limit: u32,
    offset: u32,
    unread_only: bool,
) -> Result<Vec<Thread>> {
    let unread_clause = if unread_only {
        "AND unread_count > 0"
    } else {
        ""
    };
    let sql = format!(
        "SELECT id, account_id, subject, message_count, unread_count, last_message_at
           FROM threads WHERE account_id = ?1 {unread_clause}
          ORDER BY last_message_at DESC LIMIT ?2 OFFSET ?3"
    );
    let mut stmt = conn.prepare(&sql).map_err(db_err)?;
    let rows = stmt
        .query_map(rusqlite::params![account_id, limit, offset], row_to_thread)
        .map_err(db_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

/// Threads across every account, newest first — the merged conversation view.
pub fn list_all(
    conn: &Connection,
    limit: u32,
    offset: u32,
    unread_only: bool,
) -> Result<Vec<Thread>> {
    let unread_clause = if unread_only {
        "WHERE unread_count > 0"
    } else {
        ""
    };
    let sql = format!(
        "SELECT id, account_id, subject, message_count, unread_count, last_message_at
           FROM threads {unread_clause}
          ORDER BY last_message_at DESC LIMIT ?1 OFFSET ?2"
    );
    let mut stmt = conn.prepare(&sql).map_err(db_err)?;
    let rows = stmt
        .query_map(rusqlite::params![limit, offset], row_to_thread)
        .map_err(db_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

/// Conversations represented in a folder, retaining the full conversation counters.
pub fn list_folder(conn: &Connection, folder_id: &str, limit: u32) -> Result<Vec<Thread>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, account_id, subject, message_count, unread_count, last_message_at
         FROM threads t WHERE EXISTS (
             SELECT 1 FROM messages m WHERE m.thread_id = t.id AND m.folder_id = ?1
         ) ORDER BY last_message_at DESC LIMIT ?2",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map(rusqlite::params![folder_id, limit], row_to_thread)
        .map_err(db_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

fn row_to_thread(row: &rusqlite::Row<'_>) -> rusqlite::Result<Thread> {
    Ok(Thread {
        id: row.get(0)?,
        account_id: row.get(1)?,
        subject: row.get(2)?,
        message_count: row.get::<_, u32>(3)?,
        message_ids: Vec::new(),
        last_message_at: row.get(5)?,
        unread_count: row.get::<_, u32>(4)?,
    })
}

/// All message ids belonging to a thread, oldest first.
pub fn message_ids(conn: &Connection, thread_id: &str) -> Result<Vec<MessageId>> {
    let sql = format!(
        "SELECT id FROM (
           SELECT m.*, {} AS dedup_rank FROM messages m LEFT JOIN folders f ON f.id=m.folder_id
           WHERE m.thread_id=?1 AND m.is_deleted=0
         ) WHERE dedup_rank=1 ORDER BY COALESCE(sent_at, received_at, 0), id",
        crate::message_repo::DEDUP_RANK
    );
    let mut stmt = conn.prepare(&sql).map_err(db_err)?;
    let rows = stmt
        .query_map([thread_id], |r| r.get::<_, String>(0))
        .map_err(db_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

/// Recompute the denormalised counters after messages change.
pub fn recompute(conn: &Connection, thread_id: &str) -> Result<()> {
    let sql = format!(
        "SELECT COUNT(*),
           COALESCE(SUM(CASE WHEN is_read=0 THEN 1 ELSE 0 END),0),
           COALESCE(MAX(COALESCE(sent_at,received_at,0)),0)
         FROM (
           SELECT m.*, {} AS dedup_rank FROM messages m LEFT JOIN folders f ON f.id=m.folder_id
           WHERE m.thread_id=?1 AND m.is_deleted=0
         ) WHERE dedup_rank=1",
        crate::message_repo::DEDUP_RANK
    );
    let (count, unread, last): (u32, u32, i64) = conn
        .query_row(&sql, [thread_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(db_err)?;

    conn.execute(
        "UPDATE threads SET message_count = ?2, unread_count = ?3, last_message_at = ?4, updated_at = ?5
          WHERE id = ?1",
        rusqlite::params![
            thread_id,
            count,
            unread,
            last,
            jianxin_domain::now_ts()
        ],
    )
    .map_err(db_err)?;
    Ok(())
}

/// Recompute every thread touched by the given messages.
pub fn recompute_for(conn: &Connection, messages: &[Message]) -> Result<()> {
    let mut seen: Vec<&str> = Vec::new();
    for m in messages {
        if let Some(t) = &m.thread_id {
            if !seen.contains(&t.as_str()) {
                seen.push(t.as_str());
            }
        }
    }
    for t in seen {
        recompute(conn, t)?;
    }
    Ok(())
}

/// Wipe and rebuild every thread of one account from the stored messages,
/// returning the resulting thread count. Grouping rules only apply to mail
/// synced after they landed; this re-runs grouping over what is already
/// stored. Oldest-first ordering approximates delivery order, so a parent is
/// usually assigned before the replies that reference it.
pub fn rebuild(conn: &Connection, account_id: &AccountId) -> Result<usize> {
    conn.execute(
        "UPDATE messages SET thread_id = NULL WHERE account_id = ?1",
        [account_id as &str],
    )
    .map_err(db_err)?;
    conn.execute(
        "DELETE FROM threads WHERE account_id = ?1",
        [account_id as &str],
    )
    .map_err(db_err)?;

    let mut messages = crate::message_repo::list_account(conn, account_id)?;
    assign(conn, account_id, &mut messages)?;
    recompute_for(conn, &messages)?;

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM threads WHERE account_id = ?1",
            [account_id as &str],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    Ok(count.max(0) as usize)
}

fn subject_or_placeholder(subject: &str) -> String {
    let s = normalise_subject(subject);
    if s.is_empty() {
        "(no subject)".to_string()
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;
    use jianxin_domain::{Address, Folder, FolderKind};

    struct Fixture {
        db: Database,
        folder_id: String,
    }

    fn fixture() -> Fixture {
        let db = Database::open_in_memory().unwrap();
        let conn = db.lock();
        crate::test_util::seed_account(&conn, "acc").unwrap();
        let f = Folder::new("acc", "INBOX", "Inbox", FolderKind::Inbox);
        let folder_id = f.id.clone();
        crate::folder_repo::upsert_many(&conn, &[f]).unwrap();
        drop(conn);
        Fixture { db, folder_id }
    }

    fn msg(fix: &Fixture, remote: &str, subject: &str) -> Message {
        let mut m = Message::new();
        m.id = Message::deterministic_id("acc", &fix.folder_id, remote);
        m.account_id = "acc".to_string();
        m.folder_id = fix.folder_id.clone();
        m.remote_id = remote.to_string();
        m.subject = subject.to_string();
        m.from = vec![Address::new("bob@example.com")];
        m.sent_at = Some(100);
        m
    }

    fn save(fix: &Fixture, messages: &mut [Message]) {
        let conn = fix.db.lock();
        crate::message_repo::upsert_many(&conn, messages).unwrap();
        crate::thread_repo::assign(&conn, &"acc".to_string(), messages).unwrap();
        crate::thread_repo::recompute_for(&conn, messages).unwrap();
    }

    #[test]
    fn folder_conversations_include_replies_without_other_threads() {
        let fix = fixture();
        let sent = Folder::new("acc", "Sent", "Sent", FolderKind::Sent);
        crate::folder_repo::upsert_many(&fix.db.lock(), &[sent.clone()]).unwrap();
        let mut root = msg(&fix, "1", "Budget");
        let mut reply = msg(&fix, "2", "Re: Budget");
        reply.folder_id = sent.id.clone();
        let mut unrelated = msg(&fix, "3", "Other conversation");
        unrelated.folder_id = sent.id.clone();
        save(&fix, std::slice::from_mut(&mut root));
        save(&fix, std::slice::from_mut(&mut reply));
        save(&fix, std::slice::from_mut(&mut unrelated));
        let conn = fix.db.lock();
        let inbox = list_folder(&conn, &fix.folder_id, 50).unwrap();
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].message_count, 2);
        assert_eq!(message_ids(&conn, &inbox[0].id).unwrap().len(), 2);
        assert_eq!(list_folder(&conn, &sent.id, 50).unwrap().len(), 2);
        assert!(list_folder(&conn, "missing-folder", 50).unwrap().is_empty());
    }

    #[test]
    fn new_reply_moves_conversation_ahead_of_newer_root() {
        let fix = fixture();
        let mut root = msg(&fix, "1", "Budget");
        let mut other = msg(&fix, "2", "Other");
        other.sent_at = Some(200);
        save(&fix, std::slice::from_mut(&mut root));
        save(&fix, std::slice::from_mut(&mut other));
        let conn = fix.db.lock();
        let mut reply = msg(&fix, "3", "Re: Budget");
        reply.thread_id = root.thread_id.clone();
        reply.sent_at = None;
        reply.received_at = Some(300);
        // Cache writes can already carry a thread and do not call assign/recompute.
        crate::message_repo::upsert_many(&conn, &[reply.clone()]).unwrap();
        let threads = list_all(&conn, 50, 0, false).unwrap();
        assert_eq!(threads[0].id, root.thread_id.clone().unwrap());
        assert_eq!(threads[0].last_message_at, 300);
        assert_eq!(threads[0].message_count, 2);
        assert_eq!(
            list_folder(&conn, &fix.folder_id, 1).unwrap()[0].id,
            threads[0].id
        );
        let allowed = vec!["acc".into()];
        let rows = crate::reader_repo::conversations(
            &conn,
            &crate::reader_repo::Filter {
                account_id: "",
                allowed_accounts: &allowed,
                folder_id: "",
                unread: false,
                starred: false,
                query: "",
                limit: 1,
                offset: 0,
            },
        )
        .unwrap();
        assert_eq!(rows[0].id, threads[0].id);
        assert_eq!(rows[0].last_message_at, 300);
        // A re-sync without a thread id must refresh the stored conversation too.
        reply.thread_id = None;
        reply.received_at = Some(400);
        crate::message_repo::upsert_many(&conn, &[reply]).unwrap();
        assert_eq!(
            get(&conn, &threads[0].id).unwrap().unwrap().last_message_at,
            400
        );
        conn.execute("UPDATE threads SET last_message_at = 100", [])
            .unwrap();
        conn.pragma_update(None, "user_version", 4).unwrap();
        drop(conn);
        let mut conn = fix.db.lock();
        crate::migrations::migrate(&mut conn).unwrap();
        assert_eq!(
            list_all(&conn, 1, 0, false).unwrap()[0].last_message_at,
            400
        );
        crate::migrations::migrate(&mut conn).unwrap();
        assert_eq!(
            list_all(&conn, 1, 0, false).unwrap()[0].last_message_at,
            400
        );
    }

    #[test]
    fn groups_by_references() {
        let fix = fixture();
        let mut root = msg(&fix, "1", "Budget");
        root.message_id_header = Some("<root@example.com>".to_string());
        let mut reply = msg(&fix, "2", "Re: Budget");
        reply.message_id_header = Some("<reply@example.com>".to_string());
        reply.in_reply_to = Some("<root@example.com>".to_string());
        reply.references = vec!["<root@example.com>".to_string()];

        save(&fix, std::slice::from_mut(&mut root));
        save(&fix, std::slice::from_mut(&mut reply));

        assert_eq!(root.thread_id, reply.thread_id);
    }

    #[test]
    fn falls_back_to_subject() {
        let fix = fixture();
        let mut a = msg(&fix, "1", "Weekly report");
        let mut b = msg(&fix, "2", "Re: Re: Weekly report");
        save(&fix, std::slice::from_mut(&mut a));
        save(&fix, std::slice::from_mut(&mut b));
        assert_eq!(a.thread_id, b.thread_id);
    }

    #[test]
    fn groups_by_server_thread_key() {
        let fix = fixture();
        let mut a = msg(&fix, "1", "A");
        a.server_thread_key = Some("gmail-thrid:42".to_string());
        // No headers tying these together and a different subject — only the
        // server key can join them.
        let mut b = msg(&fix, "2", "B");
        b.server_thread_key = Some("gmail-thrid:42".to_string());
        save(&fix, std::slice::from_mut(&mut a));
        save(&fix, std::slice::from_mut(&mut b));
        assert_eq!(a.thread_id, b.thread_id);
    }

    #[test]
    fn groups_by_thread_index_root() {
        let fix = fixture();
        let mut a = msg(&fix, "1", "A");
        a.thread_index_root = Some("0102030405060708090a0b0c0d0e0f10".to_string());
        let mut b = msg(&fix, "2", "B");
        b.thread_index_root = Some("0102030405060708090a0b0c0d0e0f10".to_string());
        save(&fix, std::slice::from_mut(&mut a));
        save(&fix, std::slice::from_mut(&mut b));
        assert_eq!(a.thread_id, b.thread_id);
    }

    #[test]
    fn rebuild_regroups_everything() {
        let fix = fixture();
        let mut a = msg(&fix, "1", "Weekly");
        let mut b = msg(&fix, "2", "Re: Weekly");
        save(&fix, std::slice::from_mut(&mut a));
        let orphan_thread = {
            let conn = fix.db.lock();
            let id = create(&conn, &"acc".to_string(), "Re: Weekly").unwrap();
            b.thread_id = Some(id.clone());
            crate::message_repo::upsert_many(&conn, std::slice::from_mut(&mut b)).unwrap();
            id
        };
        assert_ne!(a.thread_id.as_deref(), b.thread_id.as_deref());

        let count = {
            let conn = fix.db.lock();
            rebuild(&conn, &"acc".to_string()).unwrap()
        };
        assert_eq!(count, 1);
        assert!(get(&fix.db.lock(), &orphan_thread).unwrap().is_none());

        let conn = fix.db.lock();
        let regrouped = crate::message_repo::get(&conn, &a.id).unwrap().unwrap();
        let ids = message_ids(&conn, regrouped.thread_id.as_deref().unwrap()).unwrap();
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn distinct_subjects_get_distinct_threads() {
        let fix = fixture();
        let mut a = msg(&fix, "1", "Alpha");
        let mut b = msg(&fix, "2", "Beta");
        save(&fix, std::slice::from_mut(&mut a));
        save(&fix, std::slice::from_mut(&mut b));
        assert_ne!(a.thread_id, b.thread_id);
    }

    #[test]
    fn recompute_updates_counters() {
        let fix = fixture();
        let mut a = msg(&fix, "1", "Thread");
        a.is_read = false;
        let mut b = msg(&fix, "2", "Re: Thread");
        b.is_read = true;
        save(&fix, std::slice::from_mut(&mut a));
        save(&fix, std::slice::from_mut(&mut b));

        let conn = fix.db.lock();
        let t = get(&conn, a.thread_id.as_ref().unwrap()).unwrap().unwrap();
        assert_eq!(t.subject, "Thread");
        let ids = message_ids(&conn, &t.id).unwrap();
        assert_eq!(ids.len(), 2);
    }
}
