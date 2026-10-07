//! Folder rows.
//!
//! `kind` stores the normalised `FolderKind`, never the remote name — the UI
//! switches on `kind`, so `INBOX`, `Inbox` and `收件箱` all render the same
//! pane (§16).

use jianxin_domain::{Folder, FolderId, FolderKind, Result};
use rusqlite::{Connection, OptionalExtension};

use crate::migrations::db_err;

/// Upsert a batch of folders, preserving any user-set `sync_enabled` override.
pub fn upsert_many(conn: &Connection, folders: &[Folder]) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "INSERT INTO folders
                (id, account_id, remote_id, parent_id, name, kind,
                 unread_count, total_count, sync_enabled, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(account_id, remote_id) DO UPDATE SET
                 parent_id    = excluded.parent_id,
                 name         = excluded.name,
                 kind         = excluded.kind,
                 unread_count = excluded.unread_count,
                 total_count  = excluded.total_count,
                 updated_at   = excluded.updated_at",
        )
        .map_err(db_err)?;

    let now = jianxin_domain::now_ts();
    for f in folders {
        stmt.execute(rusqlite::params![
            f.id,
            f.account_id,
            f.remote_id,
            f.parent_id,
            f.name,
            f.kind.as_str(),
            f.unread_count,
            f.total_count,
            f.kind.sync_by_default() as i32,
            now,
        ])
        .map_err(db_err)?;
    }
    Ok(())
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<Folder>> {
    conn.query_row(
        "SELECT id, account_id, remote_id, parent_id, name, kind, unread_count, total_count
           FROM folders WHERE id = ?1",
        [id],
        row_to_folder,
    )
    .optional()
    .map_err(db_err)
}

pub fn get_by_remote_id(conn: &Connection, account_id: &str, remote_id: &str) -> Result<Option<Folder>> {
    conn.query_row(
        "SELECT id, account_id, remote_id, parent_id, name, kind, unread_count, total_count
           FROM folders WHERE account_id = ?1 AND remote_id = ?2",
        rusqlite::params![account_id, remote_id],
        row_to_folder,
    )
    .optional()
    .map_err(db_err)
}

/// Folders for one account, ordered by role so Inbox always comes first.
pub fn list(conn: &Connection, account_id: &str) -> Result<Vec<Folder>> {
    let mut stmt = conn
        .prepare(
            // Role ordering is applied in Rust (`rank`) so the ordering rule
            // lives in exactly one place.
            "SELECT id, account_id, remote_id, parent_id, name, kind, unread_count, total_count
               FROM folders
              WHERE account_id = ?1
              ORDER BY name COLLATE NOCASE",
        )
        .map_err(db_err)?;
    let mut rows = stmt
        .query_map([account_id], row_to_folder)
        .map_err(db_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_err)?;
    rows.sort_by_key(|f| crate::folder_repo::rank(f.kind));
    Ok(rows)
}

/// Folders that should be synced, i.e. role-syncable *and* not disabled.
pub fn list_syncable(conn: &Connection, account_id: &str) -> Result<Vec<Folder>> {
    Ok(list(conn, account_id)?
        .into_iter()
        .filter(|f| f.kind.sync_by_default())
        .collect())
}

pub fn find_by_kind(conn: &Connection, account_id: &str, kind: FolderKind) -> Result<Option<Folder>> {
    conn.query_row(
        "SELECT id, account_id, remote_id, parent_id, name, kind, unread_count, total_count
           FROM folders WHERE account_id = ?1 AND kind = ?2
           ORDER BY name COLLATE NOCASE LIMIT 1",
        rusqlite::params![account_id, kind.as_str()],
        row_to_folder,
    )
    .optional()
    .map_err(db_err)
}

/// Refresh the cached counters shown in the folder pane.
pub fn set_counts(conn: &Connection, id: &str, unread: u32, total: u32) -> Result<()> {
    conn.execute(
        "UPDATE folders SET unread_count = ?2, total_count = ?3, updated_at = ?4 WHERE id = ?1",
        rusqlite::params![id, unread, total, jianxin_domain::now_ts()],
    )
    .map_err(db_err)?;
    Ok(())
}

/// Turn local sync on/off for a folder (a user override, not a role change).
pub fn set_sync_enabled(conn: &Connection, id: &str, enabled: bool) -> Result<()> {
    conn.execute(
        "UPDATE folders SET sync_enabled = ?2 WHERE id = ?1",
        rusqlite::params![id, enabled as i32],
    )
    .map_err(db_err)?;
    Ok(())
}

/// Drop folders whose remote id was not present in the latest listing.
///
/// Returns how many were removed; messages follow via `ON DELETE CASCADE`.
pub fn delete_missing(
    conn: &Connection,
    account_id: &str,
    keep_remote_ids: &[String],
) -> Result<usize> {
    if keep_remote_ids.is_empty() {
        return Ok(conn
            .execute("DELETE FROM folders WHERE account_id = ?1", [account_id])
            .map_err(db_err)?);
    }

    let placeholders = crate::placeholders(keep_remote_ids.len());
    let sql = format!(
        "DELETE FROM folders
          WHERE account_id = ?1
            AND remote_id NOT IN ({placeholders})"
    );
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(account_id.to_string())];
    for id in keep_remote_ids {
        params.push(Box::new(id.clone()));
    }
    conn.execute(&sql, rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())))
        .map_err(db_err)
}

/// Display ordering: Inbox, Archive, Drafts, Sent, Custom, Spam, Trash.
pub fn rank(kind: FolderKind) -> u8 {
    match kind {
        FolderKind::Inbox => 0,
        FolderKind::Archive => 1,
        FolderKind::Drafts => 2,
        FolderKind::Sent => 3,
        FolderKind::Custom => 4,
        FolderKind::Spam => 5,
        FolderKind::Trash => 6,
    }
}

/// Resolve a folder id from something the user typed (`inbox`, `INBOX`, a
/// prefix of the local id, or the display name).
pub fn resolve(
    conn: &Connection,
    account_id: &str,
    hint: &str,
) -> Result<Option<Folder>> {
    let hint = hint.trim();
    if hint.is_empty() {
        return Ok(None);
    }

    // Exact local id.
    if let Some(f) = get(conn, hint)? {
        if f.account_id == account_id {
            return Ok(Some(f));
        }
    }

    let folders = list(conn, account_id)?;

    // Role name.
    if let Some(f) = folders.iter().find(|f| f.kind.as_str() == hint.to_ascii_lowercase()) {
        return Ok(Some(f.clone()));
    }
    // Display name or remote id.
    if let Some(f) = folders.iter().find(|f| {
        f.name.eq_ignore_ascii_case(hint) || f.remote_id.eq_ignore_ascii_case(hint)
    }) {
        return Ok(Some(f.clone()));
    }
    // Unique id prefix.
    let matches: Vec<Folder> = folders
        .into_iter()
        .filter(|f| f.id.starts_with(hint))
        .collect();
    if matches.len() == 1 {
        return Ok(Some(matches.into_iter().next().unwrap()));
    }
    Ok(None)
}

fn row_to_folder(row: &rusqlite::Row<'_>) -> rusqlite::Result<Folder> {
    let kind: String = row.get(5)?;
    Ok(Folder {
        id: row.get::<_, FolderId>(0)?,
        account_id: row.get(1)?,
        remote_id: row.get(2)?,
        parent_id: row.get(3)?,
        name: row.get(4)?,
        kind: parse_kind(&kind),
        unread_count: row.get(6)?,
        total_count: row.get(7)?,
    })
}

fn parse_kind(value: &str) -> FolderKind {
    match value {
        "inbox" => FolderKind::Inbox,
        "sent" => FolderKind::Sent,
        "drafts" => FolderKind::Drafts,
        "trash" => FolderKind::Trash,
        "spam" => FolderKind::Spam,
        "archive" => FolderKind::Archive,
        _ => FolderKind::Custom,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::seed_account;
    use crate::Database;

    fn folder(account: &str, remote: &str, name: &str, kind: FolderKind) -> Folder {
        Folder {
            id: Folder::deterministic_id(account, remote),
            account_id: account.to_string(),
            remote_id: remote.to_string(),
            parent_id: None,
            name: name.to_string(),
            kind,
            unread_count: 0,
            total_count: 0,
        }
    }

    #[test]
    fn upsert_is_idempotent_and_keeps_ordering() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.lock();
        seed_account(&conn, "acc").unwrap();
        let folders = vec![
            folder("acc", "INBOX", "Inbox", FolderKind::Inbox),
            folder("acc", "Sent", "Sent", FolderKind::Sent),
            folder("acc", "Trash", "Trash", FolderKind::Trash),
        ];
        upsert_many(&conn, &folders).unwrap();
        upsert_many(&conn, &folders).unwrap();
        let all = list(&conn, "acc").unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].kind, FolderKind::Inbox);
        assert_eq!(all[2].kind, FolderKind::Trash);
    }

    #[test]
    fn syncable_excludes_spam_and_trash() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.lock();
        seed_account(&conn, "acc").unwrap();
        upsert_many(
            &conn,
            &[
                folder("acc", "INBOX", "Inbox", FolderKind::Inbox),
                folder("acc", "Junk", "Junk", FolderKind::Spam),
            ],
        )
        .unwrap();
        assert_eq!(list_syncable(&conn, "acc").unwrap().len(), 1);
    }

    #[test]
    fn delete_missing_removes_stale_folders() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.lock();
        seed_account(&conn, "acc").unwrap();
        upsert_many(
            &conn,
            &[
                folder("acc", "INBOX", "Inbox", FolderKind::Inbox),
                folder("acc", "Gone", "Gone", FolderKind::Custom),
            ],
        )
        .unwrap();
        let removed =
            delete_missing(&conn, "acc", &["INBOX".to_string()]).unwrap();
        assert_eq!(removed, 1);
        assert_eq!(list(&conn, "acc").unwrap().len(), 1);
    }

    #[test]
    fn resolves_role_names() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.lock();
        seed_account(&conn, "acc").unwrap();
        upsert_many(&conn, &[folder("acc", "INBOX", "Inbox", FolderKind::Inbox)]).unwrap();
        let f = resolve(&conn, "acc", "inbox").unwrap().unwrap();
        assert_eq!(f.remote_id, "INBOX");
    }
}
