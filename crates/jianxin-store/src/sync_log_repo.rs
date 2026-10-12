//! Completed sync attempts; credentials and raw provider errors never enter this table.
use crate::migrations::db_err;
use jianxin_domain::Result;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde::Serialize;

pub const RETENTION: u32 = 1000;

#[derive(Debug, Serialize)]
pub struct SyncLog {
    pub id: i64,
    pub account_id: String,
    pub completed_at: i64,
    pub status: String,
}

pub fn record(conn: &Connection, account: &str, status: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO sync_logs(account_id,completed_at,status) VALUES (?1,?2,?3)",
        params![account, chrono::Utc::now().timestamp(), status],
    )
    .map_err(db_err)?;
    conn.execute(
        "DELETE FROM sync_logs WHERE account_id=?1 AND id NOT IN
        (SELECT id FROM sync_logs WHERE account_id=?1 ORDER BY id DESC LIMIT ?2)",
        params![account, RETENTION],
    )
    .map_err(db_err)?;
    Ok(())
}

pub fn latest(conn: &Connection, account: &str) -> Result<Option<SyncLog>> {
    conn.query_row("SELECT id,account_id,completed_at,status FROM sync_logs WHERE account_id=?1 ORDER BY id DESC LIMIT 1",
        [account], row).optional().map_err(db_err)
}

pub fn list(
    conn: &Connection,
    accounts: &[String],
    limit: u32,
    offset: u32,
) -> Result<Vec<SyncLog>> {
    if accounts.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!("SELECT id,account_id,completed_at,status FROM sync_logs WHERE account_id IN ({}) ORDER BY id DESC LIMIT ? OFFSET ?", crate::placeholders(accounts.len()));
    let mut args: Vec<rusqlite::types::Value> = accounts.iter().cloned().map(Into::into).collect();
    args.push(limit.min(101).into());
    args.push(offset.into());
    conn.prepare(&sql)
        .map_err(db_err)?
        .query_map(params_from_iter(args), row)
        .map_err(db_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_err)
}

fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SyncLog> {
    Ok(SyncLog {
        id: row.get(0)?,
        account_id: row.get(1)?,
        completed_at: row.get(2)?,
        status: row.get(3)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_is_bounded_ordered_scoped_and_rejects_raw_errors() {
        let db = crate::Database::open_in_memory().unwrap();
        for id in ["a", "b"] {
            crate::test_util::seed_account(&db.lock(), id).unwrap();
        }
        db.transaction(|conn| {
            record(conn, "b", "login_failed")?;
            for _ in 0..RETENTION + 2 {
                record(conn, "a", "synced")?;
            }
            Ok(())
        })
        .unwrap();
        let conn = db.lock();
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM sync_logs WHERE account_id='a'",
                [],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
            RETENTION
        );
        assert!(record(&conn, "a", "password-secret").is_err());
        let rows = list(&conn, &["a".into()], 2, 0).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows[0].id > rows[1].id);
        assert_eq!(list(&conn, &["a".into()], 1, 1).unwrap()[0].id, rows[1].id);
        assert_eq!(latest(&conn, "b").unwrap().unwrap().status, "login_failed");
        assert!(latest(&conn, "missing").unwrap().is_none());
        assert!(list(&conn, &[], 10, 0).unwrap().is_empty());
    }
}
