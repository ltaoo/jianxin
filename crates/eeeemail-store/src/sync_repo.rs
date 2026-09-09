//! Per-folder sync checkpoints.
//!
//! The cursor value is opaque and only meaningful together with `cursor_type`:
//! Graph stores a `deltaLink`, Gmail a `historyId`, IMAP `UIDVALIDITY:UID`.
//! Assuming `nextPageToken` semantics here would break two of the three (§19).

use eeeemail_domain::{Result, SyncCursor};
use rusqlite::{Connection, OptionalExtension};

use crate::migrations::db_err;

pub fn get(conn: &Connection, account_id: &str, folder_id: &str) -> Result<Option<SyncCursor>> {
    conn.query_row(
        "SELECT provider, account_id, folder_id, cursor_type, cursor
           FROM sync_states WHERE account_id = ?1 AND folder_id = ?2",
        rusqlite::params![account_id, folder_id],
        |row| {
            Ok(SyncCursor {
                provider: row.get(0)?,
                account_id: row.get(1)?,
                folder_id: row.get(2)?,
                cursor_type: row.get(3)?,
                cursor: row.get(4)?,
            })
        },
    )
    .optional()
    .map_err(db_err)
}

pub fn set(conn: &Connection, cursor: &SyncCursor) -> Result<()> {
    conn.execute(
        "INSERT INTO sync_states
             (account_id, folder_id, provider, cursor_type, cursor, last_synced_at, last_error)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)
         ON CONFLICT(account_id, folder_id) DO UPDATE SET
             provider       = excluded.provider,
             cursor_type    = excluded.cursor_type,
             cursor         = excluded.cursor,
             last_synced_at = excluded.last_synced_at,
             last_error     = NULL",
        rusqlite::params![
            cursor.account_id,
            cursor.folder_id,
            cursor.provider,
            cursor.cursor_type,
            cursor.cursor,
            eeeemail_domain::now_ts(),
        ],
    )
    .map_err(db_err)?;
    Ok(())
}

/// Record a failed sync without destroying the checkpoint — the next run
/// resumes from the same place.
pub fn record_error(conn: &Connection, account_id: &str, folder_id: &str, error: &str) -> Result<()> {
    conn.execute(
        "UPDATE sync_states SET last_error = ?3 WHERE account_id = ?1 AND folder_id = ?2",
        rusqlite::params![account_id, folder_id, error],
    )
    .map_err(db_err)?;
    Ok(())
}

/// Forget a checkpoint, forcing a full resync of that folder.
pub fn clear(conn: &Connection, account_id: &str, folder_id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM sync_states WHERE account_id = ?1 AND folder_id = ?2",
        rusqlite::params![account_id, folder_id],
    )
    .map_err(db_err)?;
    Ok(())
}

pub fn clear_account(conn: &Connection, account_id: &str) -> Result<()> {
    conn.execute("DELETE FROM sync_states WHERE account_id = ?1", [account_id])
        .map_err(db_err)?;
    Ok(())
}

/// A folder's sync state as the UI shows it.
#[derive(Debug, Clone)]
pub struct SyncStateRow {
    pub account_id: String,
    pub folder_id: String,
    pub provider: String,
    pub cursor_type: String,
    pub cursor: String,
    pub last_synced_at: Option<i64>,
    pub last_error: Option<String>,
}

pub fn list_for_account(conn: &Connection, account_id: &str) -> Result<Vec<SyncStateRow>> {
    let mut stmt = conn
        .prepare(
            "SELECT account_id, folder_id, provider, cursor_type, cursor, last_synced_at, last_error
               FROM sync_states WHERE account_id = ?1 ORDER BY folder_id",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([account_id], |row| {
            Ok(SyncStateRow {
                account_id: row.get(0)?,
                folder_id: row.get(1)?,
                provider: row.get(2)?,
                cursor_type: row.get(3)?,
                cursor: row.get(4)?,
                last_synced_at: row.get(5)?,
                last_error: row.get(6)?,
            })
        })
        .map_err(db_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    fn db_with_folder() -> Database {
        let db = Database::open_in_memory().unwrap();
        let conn = db.lock();
        crate::test_util::seed_account(&conn, "acc").unwrap();
        crate::test_util::seed_folder(
            &conn,
            "f1",
            "acc",
            "INBOX",
            eeeemail_domain::FolderKind::Inbox,
        )
        .unwrap();
        drop(conn);
        db
    }

    fn cursor(folder: &str, value: &str) -> SyncCursor {
        SyncCursor {
            provider: "generic-imap".to_string(),
            account_id: "acc".to_string(),
            folder_id: folder.to_string(),
            cursor_type: "uid".to_string(),
            cursor: value.to_string(),
        }
    }

    #[test]
    fn round_trips_cursor() {
        let db = db_with_folder();
        let conn = db.lock();
        set(&conn, &cursor("f1", "42:900")).unwrap();
        let loaded = get(&conn, "acc", "f1").unwrap().unwrap();
        assert_eq!(loaded.cursor, "42:900");
        assert_eq!(loaded.cursor_type, "uid");
    }

    #[test]
    fn set_overwrites_and_clears_error() {
        let db = db_with_folder();
        let conn = db.lock();
        set(&conn, &cursor("f1", "1:1")).unwrap();
        record_error(&conn, "acc", "f1", "timeout").unwrap();
        assert!(get(&conn, "acc", "f1").unwrap().is_some());

        set(&conn, &cursor("f1", "1:2")).unwrap();
        let state = list_for_account(&conn, "acc").unwrap();
        assert_eq!(state.len(), 1);
        assert!(state[0].last_error.is_none());
        assert_eq!(state[0].cursor, "1:2");
    }

    #[test]
    fn clear_forces_resync() {
        let db = db_with_folder();
        let conn = db.lock();
        set(&conn, &cursor("f1", "1:1")).unwrap();
        clear(&conn, "acc", "f1").unwrap();
        assert!(get(&conn, "acc", "f1").unwrap().is_none());
    }
}
