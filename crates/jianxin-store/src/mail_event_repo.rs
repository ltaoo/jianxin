//! Persistent new-mail events.

use jianxin_domain::{MailError, MailErrorCode, Message, Result};
use rusqlite::{Connection, OptionalExtension};

use crate::migrations::db_err;

#[derive(Debug, Clone)]
pub struct MailEvent {
    pub id: String,
    pub account_id: String,
    pub message_id: String,
    pub detected_at: i64,
}

/// Record inbox arrivals once per account and Message-ID (local id fallback).
pub fn record_messages(
    conn: &Connection,
    account_id: &str,
    messages: &[Message],
) -> Result<Vec<MailEvent>> {
    let tx = conn.unchecked_transaction().map_err(db_err)?;
    let mut out = Vec::new();
    for message in messages {
        if message.account_id != account_id {
            return Err(MailError::new(
                MailErrorCode::Storage,
                format!(
                    "message {} does not belong to account {account_id}",
                    message.id
                ),
            ));
        }
        let inbox: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM folders WHERE id=?1 AND account_id=?2 AND kind='inbox')",
            rusqlite::params![message.folder_id, account_id], |row| row.get(0),
        ).map_err(db_err)?;
        if !inbox || message.is_draft || message.is_deleted {
            continue;
        }
        let recorded: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM mail_events WHERE message_id=?1)",
                [&message.id],
                |row| row.get(0),
            )
            .map_err(db_err)?;
        if recorded {
            continue;
        }
        let event = MailEvent {
            id: jianxin_domain::new_id(),
            account_id: account_id.to_string(),
            message_id: message.id.clone(),
            detected_at: jianxin_domain::now_ts(),
        };
        let header = message
            .message_id_header
            .as_deref()
            .unwrap_or("")
            .trim_matches([' ', '\t', '\n', '\r']);
        let dedup_key = if header.is_empty() {
            format!("local:{}", message.id)
        } else {
            format!("header:{header}")
        };
        let claimed = tx.execute(
            "INSERT OR IGNORE INTO mail_event_keys (account_id, dedup_key, event_id) VALUES (?1,?2,?3)",
            rusqlite::params![account_id, dedup_key, event.id],
        ).map_err(db_err)?;
        if claimed == 0 {
            continue;
        }
        let changed = tx
            .execute(
                "INSERT INTO mail_events
                     (id, account_id, message_id, detected_at, delivered_at)
                 VALUES (?1, ?2, ?3, ?4, NULL)",
                rusqlite::params![
                    event.id,
                    event.account_id,
                    event.message_id,
                    event.detected_at
                ],
            )
            .map_err(db_err)?;
        if changed == 1 {
            out.push(event);
        }
    }
    tx.commit().map_err(db_err)?;
    Ok(out)
}

pub fn pending(conn: &Connection, limit: u32) -> Result<Vec<MailEvent>> {
    let mut stmt = conn
        .prepare(
            "SELECT e.id, e.account_id, e.message_id, e.detected_at
               FROM mail_events e
               JOIN mail_event_keys k ON k.event_id=e.id AND k.account_id=e.account_id
               JOIN messages m ON m.id=e.message_id
               JOIN folders f ON f.id=m.folder_id AND f.account_id=e.account_id
              WHERE e.delivered_at IS NULL AND f.kind='inbox'
                AND m.is_draft=0 AND m.is_deleted=0
              ORDER BY e.detected_at, e.id LIMIT ?1",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([limit], |row| {
            Ok(MailEvent {
                id: row.get(0)?,
                account_id: row.get(1)?,
                message_id: row.get(2)?,
                detected_at: row.get(3)?,
            })
        })
        .map_err(db_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<MailEvent>> {
    conn.query_row(
        "SELECT id, account_id, message_id, detected_at
           FROM mail_events WHERE id = ?1",
        [id],
        |row| {
            Ok(MailEvent {
                id: row.get(0)?,
                account_id: row.get(1)?,
                message_id: row.get(2)?,
                detected_at: row.get(3)?,
            })
        },
    )
    .optional()
    .map_err(db_err)
}

pub fn mark_delivered(conn: &Connection, ids: &[String]) -> Result<()> {
    for id in ids {
        conn.execute(
            "UPDATE mail_events SET delivered_at = ?2 WHERE id = ?1",
            rusqlite::params![id, jianxin_domain::now_ts()],
        )
        .map_err(db_err)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util;

    fn seed_message(
        conn: &Connection,
        account: &str,
        kind: jianxin_domain::FolderKind,
        header: Option<&str>,
    ) -> Message {
        let folder =
            jianxin_domain::Folder::new(account, &jianxin_domain::new_id(), "folder", kind);
        crate::folder_repo::upsert_many(conn, &[folder.clone()]).unwrap();
        let mut message = Message::new();
        message.account_id = account.into();
        message.folder_id = folder.id;
        message.remote_id = "1".into();
        message.message_id_header = header.map(str::to_string);
        crate::message_repo::upsert_many(conn, &[message.clone()]).unwrap();
        message
    }

    #[test]
    fn only_inbox_arrivals_generate_events() {
        use jianxin_domain::FolderKind::*;
        let db = crate::Database::open_in_memory().unwrap();
        let conn = db.lock();
        test_util::seed_account(&conn, "acc").unwrap();
        for kind in [Sent, Archive, Drafts, Spam, Trash, Custom] {
            let message = seed_message(&conn, "acc", kind, Some("<same@example.com>"));
            assert!(record_messages(&conn, "acc", &[message])
                .unwrap()
                .is_empty());
        }
        let inbox = seed_message(&conn, "acc", Inbox, Some("<same@example.com>"));
        assert_eq!(record_messages(&conn, "acc", &[inbox]).unwrap().len(), 1);
        assert_eq!(pending(&conn, 10).unwrap().len(), 1);
    }

    #[test]
    fn header_dedup_is_account_scoped_and_survives_delivery_and_deletion() {
        use jianxin_domain::FolderKind::Inbox;
        let db = crate::Database::open_in_memory().unwrap();
        let conn = db.lock();
        for account in ["a", "b"] {
            test_util::seed_account(&conn, account).unwrap();
        }
        let first = seed_message(&conn, "a", Inbox, Some(" <same@example.com>\r\n"));
        let copy = seed_message(&conn, "a", Inbox, Some("<same@example.com>"));
        let other_account = seed_message(&conn, "b", Inbox, Some("<same@example.com>"));
        let events = record_messages(&conn, "a", &[first.clone(), copy.clone()]).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(pending(&conn, 10).unwrap().len(), 1);
        mark_delivered(&conn, &[events[0].id.clone()]).unwrap();
        conn.execute("DELETE FROM messages WHERE id=?1", [&first.id])
            .unwrap();
        assert!(record_messages(&conn, "a", &[copy]).unwrap().is_empty());
        assert_eq!(
            record_messages(&conn, "b", &[other_account]).unwrap().len(),
            1
        );
    }

    #[test]
    fn missing_headers_do_not_merge_unrelated_mail_and_retries_keep_event_id() {
        let db = crate::Database::open_in_memory().unwrap();
        let conn = db.lock();
        test_util::seed_account(&conn, "acc").unwrap();
        let messages = [
            seed_message(&conn, "acc", jianxin_domain::FolderKind::Inbox, None),
            seed_message(&conn, "acc", jianxin_domain::FolderKind::Inbox, Some("  ")),
        ];
        assert_eq!(record_messages(&conn, "acc", &messages).unwrap().len(), 2);
        let before = pending(&conn, 10).unwrap();
        assert!(record_messages(&conn, "acc", &messages).unwrap().is_empty());
        let after = pending(&conn, 10).unwrap();
        assert_eq!(
            before.iter().map(|e| &e.id).collect::<Vec<_>>(),
            after.iter().map(|e| &e.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn upgrade_ignores_legacy_sent_and_prefers_already_delivered_duplicate() {
        use jianxin_domain::FolderKind::{Inbox, Sent};
        let db = crate::Database::open_in_memory().unwrap();
        let mut conn = db.lock();
        test_util::seed_account(&conn, "acc").unwrap();
        let first = seed_message(&conn, "acc", Inbox, Some("<same@example.com>"));
        let copy = seed_message(&conn, "acc", Inbox, Some(" <same@example.com> "));
        let sent = seed_message(&conn, "acc", Sent, Some("<sent@example.com>"));
        let fresh = seed_message(&conn, "acc", Inbox, Some("<fresh@example.com>"));
        conn.execute_batch("DROP TABLE mail_event_keys; PRAGMA user_version=3;")
            .unwrap();
        for (id, message, delivered) in [
            ("first", first, Some(20)),
            ("copy", copy, None),
            ("sent", sent, None),
            ("fresh", fresh, None),
        ] {
            conn.execute(
                "INSERT INTO mail_events VALUES (?1,'acc',?2,10,?3)",
                rusqlite::params![id, message.id, delivered],
            )
            .unwrap();
        }
        crate::migrations::migrate(&mut conn).unwrap();
        let events = pending(&conn, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, "fresh");
        let retained: i64 = conn
            .query_row("SELECT count(*) FROM mail_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(retained, 4);
    }

    #[test]
    fn records_each_message_once_and_tracks_delivery() {
        let db = crate::Database::open_in_memory().unwrap();
        let conn = db.lock();
        test_util::seed_account(&conn, "acc").unwrap();
        let folder = jianxin_domain::Folder::new(
            "acc",
            "INBOX",
            "INBOX",
            jianxin_domain::FolderKind::Inbox,
        );
        crate::folder_repo::upsert_many(&conn, &[folder.clone()]).unwrap();
        let mut message = Message::new();
        message.account_id = "acc".to_string();
        message.folder_id = folder.id;
        message.remote_id = "1".to_string();
        crate::message_repo::upsert_many(&conn, &[message.clone()]).unwrap();

        assert_eq!(
            record_messages(&conn, "acc", &[message.clone()])
                .unwrap()
                .len(),
            1
        );
        assert!(record_messages(&conn, "acc", &[message])
            .unwrap()
            .is_empty());
        let event = pending(&conn, 10).unwrap().pop().unwrap();
        mark_delivered(&conn, &[event.id.clone()]).unwrap();
        assert!(pending(&conn, 10).unwrap().is_empty());
        assert!(get(&conn, &event.id).unwrap().is_some());
    }
}
