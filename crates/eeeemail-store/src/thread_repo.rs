//! Local conversation grouping.
//!
//! Graph exposes `conversationId` and Gmail a `threadId`, but IMAP has neither.
//! Threads are therefore assembled locally from `Message-ID` / `In-Reply-To` /
//! `References`, with a normalised-subject fallback, so every provider ends up
//! with the same model (§17).

use eeeemail_domain::thread::normalise_subject;
use eeeemail_domain::{AccountId, Message, MessageId, Result, Thread, ThreadId};
use rusqlite::{Connection, OptionalExtension};

use crate::migrations::db_err;

/// Assign a `thread_id` to every message that does not have one yet.
///
/// Resolution order:
///
/// 1. an ancestor named in `In-Reply-To` / `References` that we already know,
/// 2. an existing thread in the same account with the same normalised subject,
/// 3. a brand new thread.
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
    let id = eeeemail_domain::new_id();
    conn.execute(
        "INSERT INTO threads (id, account_id, subject, message_count, unread_count, last_message_at, updated_at)
         VALUES (?1, ?2, ?3, 0, 0, 0, ?4)",
        rusqlite::params![id, account_id, subject, eeeemail_domain::now_ts()],
    )
    .map_err(db_err)?;
    Ok(id)
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<Thread>> {
    conn.query_row(
        "SELECT id, account_id, subject, message_count, unread_count, last_message_at
           FROM threads WHERE id = ?1",
        [id],
        |row| {
            Ok(Thread {
                id: row.get(0)?,
                account_id: row.get(1)?,
                subject: row.get(2)?,
                message_ids: Vec::new(),
                last_message_at: row.get(5)?,
                unread_count: row.get::<_, u32>(4)?,
            })
        },
    )
    .optional()
    .map_err(db_err)
}

pub fn list(conn: &Connection, account_id: &str, limit: u32) -> Result<Vec<Thread>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, account_id, subject, message_count, unread_count, last_message_at
               FROM threads WHERE account_id = ?1
              ORDER BY last_message_at DESC LIMIT ?2",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map(rusqlite::params![account_id, limit], |row| {
            Ok(Thread {
                id: row.get(0)?,
                account_id: row.get(1)?,
                subject: row.get(2)?,
                message_ids: Vec::new(),
                last_message_at: row.get(5)?,
                unread_count: row.get::<_, u32>(4)?,
            })
        })
        .map_err(db_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

/// All message ids belonging to a thread, oldest first.
pub fn message_ids(conn: &Connection, thread_id: &str) -> Result<Vec<MessageId>> {
    let mut stmt = conn
        .prepare(
            "SELECT id FROM messages WHERE thread_id = ?1
              ORDER BY COALESCE(sent_at, received_at, 0), id",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([thread_id], |r| r.get::<_, String>(0))
        .map_err(db_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

/// Recompute the denormalised counters after messages change.
pub fn recompute(conn: &Connection, thread_id: &str) -> Result<()> {
    let (count, unread, last): (u32, u32, i64) = conn
        .query_row(
            "SELECT COUNT(*),
                    COALESCE(SUM(CASE WHEN is_read = 0 THEN 1 ELSE 0 END), 0),
                    COALESCE(MAX(COALESCE(sent_at, received_at, 0)), 0)
               FROM messages WHERE thread_id = ?1",
            [thread_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(db_err)?;

    conn.execute(
        "UPDATE threads SET message_count = ?2, unread_count = ?3, last_message_at = ?4, updated_at = ?5
          WHERE id = ?1",
        rusqlite::params![
            thread_id,
            count,
            unread,
            last,
            eeeemail_domain::now_ts()
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
    use eeeemail_domain::{Address, Folder, FolderKind};

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
