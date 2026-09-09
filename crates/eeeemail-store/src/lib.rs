//! # eeeemail-store
//!
//! Offline-first SQLite persistence.
//!
//! The UI never talks to a provider directly to render a list — it reads this
//! database and a background sync refreshes it. That is what makes the app
//! usable on a train and makes a failed network call a stale timestamp rather
//! than an empty window (§28).
//!
//! ```text
//! accounts ─┬─ folders ─── messages ─┬─ addresses
//!           │                        ├─ message_references
//!           │                        ├─ message_labels
//!           │                        └─ attachments
//!           ├─ threads
//!           ├─ sync_states
//!           └─ outbox ──── outbox_attachments
//! ```
//!
//! Every write goes through one of the `*_repo` modules; SQL does not escape
//! into the application layer.

pub mod account_repo;
pub mod folder_repo;
pub mod message_repo;
pub mod migrations;
pub mod outbox_repo;
pub mod sync_repo;
pub mod thread_repo;

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use eeeemail_domain::{MailError, MailErrorCode, Result};
use rusqlite::Connection;

pub use migrations::LATEST_VERSION;
pub use outbox_repo::{OutboxJob, OutboxStatus};

/// The local database.
///
/// Wrapped in a `Mutex` because `rusqlite::Connection` is `Send` but not
/// `Sync`; the TUI shares one `Arc<Database>` between the render loop and the
/// background sync thread.
pub struct Database {
    conn: Mutex<Connection>,
    path: Option<PathBuf>,
}

impl Database {
    /// Open (or create) the database at `path`, creating parent directories and
    /// applying migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    MailError::new(
                        MailErrorCode::Storage,
                        format!("cannot create data directory {}: {e}", parent.display()),
                    )
                    .with_source(e)
                })?;
            }
        }

        let mut conn = Connection::open(path).map_err(migrations::db_err)?;
        configure(&mut conn)?;
        migrations::migrate(&mut conn)?;

        Ok(Self {
            conn: Mutex::new(conn),
            path: Some(path.to_path_buf()),
        })
    }

    /// An ephemeral database, used by tests and `--dry-run` style commands.
    pub fn open_in_memory() -> Result<Self> {
        let mut conn = Connection::open_in_memory().map_err(migrations::db_err)?;
        configure(&mut conn)?;
        migrations::migrate(&mut conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            path: None,
        })
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Borrow the connection. Panics only if a previous holder panicked.
    pub fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Run `f` inside a transaction, rolling back and propagating on error.
    ///
    /// Repositories take `&Connection`, and `Transaction` derefs to one, so the
    /// same helpers work inside and outside a transaction.
    pub fn transaction<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let mut guard = self.lock();
        let tx = guard.transaction().map_err(migrations::db_err)?;
        let out = f(&tx)?;
        tx.commit().map_err(migrations::db_err)?;
        Ok(out)
    }
}

/// Pragmas that must be set on every connection.
fn configure(conn: &mut Connection) -> Result<()> {
    // ON DELETE CASCADE is part of the schema's integrity; without this the
    // foreign keys are documentation only.
    conn.pragma_update(None, "foreign_keys", true)
        .map_err(migrations::db_err)?;

    // WAL lets the sync thread write while the UI reads. It is a no-op for
    // in-memory databases, which is fine.
    let _: String = conn
        .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
        .map_err(migrations::db_err)?;

    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(migrations::db_err)?;
    conn.pragma_update(None, "busy_timeout", 5_000)
        .map_err(migrations::db_err)?;

    Ok(())
}

/// Build an `IN (?, ?, …)` placeholder list.
pub(crate) fn placeholders(n: usize) -> String {
    let mut s = String::with_capacity(n * 2);
    for i in 0..n {
        if i > 0 {
            s.push(',');
        }
        s.push('?');
    }
    s
}

#[cfg(test)]
pub(crate) mod test_util {
    use eeeemail_domain::{Account, AuthType, Folder, FolderKind, Result};
    use rusqlite::Connection;

    use crate::{account_repo, folder_repo};

    /// Insert an account with a caller-chosen id.
    ///
    /// Tests use short literal ids (`"acc"`) as foreign keys, and the schema
    /// enforces those keys, so the row has to exist with exactly that id.
    pub fn seed_account(conn: &Connection, id: &str) -> Result<()> {
        let mut a = Account::new("generic-imap", "a@example.com", AuthType::Password);
        a.id = id.to_string();
        account_repo::upsert(conn, &a)
    }

    /// Insert a folder with a caller-chosen id, for the same reason.
    pub fn seed_folder(
        conn: &Connection,
        id: &str,
        account_id: &str,
        remote_id: &str,
        kind: FolderKind,
    ) -> Result<()> {
        let f = Folder {
            id: id.to_string(),
            account_id: account_id.to_string(),
            remote_id: remote_id.to_string(),
            parent_id: None,
            name: remote_id.to_string(),
            kind,
            unread_count: 0,
            total_count: 0,
        };
        folder_repo::upsert_many(conn, std::slice::from_ref(&f))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_and_migrates() {
        let db = Database::open_in_memory().unwrap();
        let version: u32 = db
            .lock()
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, LATEST_VERSION);
    }

    #[test]
    fn migration_is_idempotent() {
        let db = Database::open_in_memory().unwrap();
        {
            let mut conn = db.lock();
            migrations::migrate(&mut conn).unwrap();
        }
        let version: u32 = db
            .lock()
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, LATEST_VERSION);
    }

    #[test]
    fn foreign_keys_are_enforced() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.lock();
        let on: bool = conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert!(on);
    }
}
