//! Persistent new-mail events.

use eeeemail_domain::{MailError, MailErrorCode, Message, Result};
use rusqlite::{Connection, OptionalExtension};

use crate::migrations::db_err;

#[derive(Debug, Clone)]
pub struct MailEvent {
    pub id: String,
    pub account_id: String,
    pub message_id: String,
    pub detected_at: i64,
}

/// Record only messages that have not generated an event before.
pub fn record_messages(
    conn: &Connection,
    account_id: &str,
    messages: &[Message],
) -> Result<Vec<MailEvent>> {
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
        let event = MailEvent {
            id: eeeemail_domain::new_id(),
            account_id: account_id.to_string(),
            message_id: message.id.clone(),
            detected_at: eeeemail_domain::now_ts(),
        };
        let changed = conn
            .execute(
                "INSERT OR IGNORE INTO mail_events
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
    Ok(out)
}

pub fn pending(conn: &Connection, limit: u32) -> Result<Vec<MailEvent>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, account_id, message_id, detected_at
               FROM mail_events WHERE delivered_at IS NULL
              ORDER BY detected_at, id LIMIT ?1",
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
            rusqlite::params![id, eeeemail_domain::now_ts()],
        )
        .map_err(db_err)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util;

    #[test]
    fn records_each_message_once_and_tracks_delivery() {
        let db = crate::Database::open_in_memory().unwrap();
        let conn = db.lock();
        test_util::seed_account(&conn, "acc").unwrap();
        let folder = eeeemail_domain::Folder::new(
            "acc",
            "INBOX",
            "INBOX",
            eeeemail_domain::FolderKind::Inbox,
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
