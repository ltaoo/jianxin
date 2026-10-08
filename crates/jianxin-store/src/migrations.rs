//! SQLite schema and migrations.
//!
//! Three rules from the architecture document shape every table here:
//!
//! * **Local ids are the primary keys.** Provider ids live in `remote_id` and
//!   are never used as keys — Graph ids can even change after a move (§15).
//! * **Cursors are opaque.** `sync_states` stores `cursor_type` next to the
//!   value because Graph uses a deltaLink, Gmail a `historyId` and IMAP
//!   `UIDVALIDITY:UID` (§19). Nothing here assumes `nextPageToken`.
//! * **No credentials.** The account row carries no token; secrets resolve from
//!   env / keychain / secret command at runtime (§22).
//!
//! Local search uses `LIKE` rather than FTS5 on purpose: `unicode61` does not
//! segment Chinese, and a wrong tokenizer is worse than a slightly slower scan
//! at Phase-1 mailbox sizes. FTS5 (with a trigram tokenizer) is a Phase-2 swap
//! that does not need a schema change.

use jianxin_domain::{MailError, MailErrorCode, Result};
use rusqlite::Connection;

/// Current `PRAGMA user_version`. Bump whenever a `SCHEMA_V*` block is added.
pub const LATEST_VERSION: u32 = 6;

/// Everything needed to create a database from scratch.
const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS accounts (
    id            TEXT PRIMARY KEY,
    provider      TEXT NOT NULL,
    email         TEXT NOT NULL,
    display_name  TEXT,
    auth_type     TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS folders (
    id            TEXT PRIMARY KEY,
    account_id    TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    remote_id     TEXT NOT NULL,
    parent_id     TEXT,
    name          TEXT NOT NULL,
    kind          TEXT NOT NULL,
    unread_count  INTEGER NOT NULL DEFAULT 0,
    total_count   INTEGER NOT NULL DEFAULT 0,
    sync_enabled  INTEGER NOT NULL DEFAULT 1,
    updated_at    INTEGER NOT NULL,
    UNIQUE (account_id, remote_id)
);

CREATE TABLE IF NOT EXISTS threads (
    id              TEXT PRIMARY KEY,
    account_id      TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    subject         TEXT NOT NULL,
    message_count   INTEGER NOT NULL DEFAULT 0,
    unread_count    INTEGER NOT NULL DEFAULT 0,
    last_message_at INTEGER NOT NULL DEFAULT 0,
    updated_at      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS messages (
    id                TEXT PRIMARY KEY,
    account_id        TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    folder_id         TEXT NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    remote_id         TEXT NOT NULL,
    immutable_id      TEXT,
    thread_id         TEXT,
    subject           TEXT NOT NULL DEFAULT '',
    subject_normalized TEXT NOT NULL DEFAULT '',
    snippet           TEXT,
    text_body         TEXT,
    html_body         TEXT,
    size              INTEGER NOT NULL DEFAULT 0,
    received_at       INTEGER,
    sent_at           INTEGER,
    message_id_header TEXT,
    in_reply_to       TEXT,
    is_read           INTEGER NOT NULL DEFAULT 0,
    is_starred        INTEGER NOT NULL DEFAULT 0,
    is_draft          INTEGER NOT NULL DEFAULT 0,
    is_deleted        INTEGER NOT NULL DEFAULT 0,
    provider_data     TEXT,
    updated_at        INTEGER NOT NULL,
    UNIQUE (account_id, folder_id, remote_id)
);

CREATE TABLE IF NOT EXISTS addresses (
    message_id  TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    role        TEXT NOT NULL,
    position    INTEGER NOT NULL,
    name        TEXT,
    email       TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS message_references (
    message_id  TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    ref_value   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS message_labels (
    message_id  TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    label       TEXT NOT NULL,
    PRIMARY KEY (message_id, label)
);

CREATE TABLE IF NOT EXISTS attachments (
    id          TEXT PRIMARY KEY,
    message_id  TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL DEFAULT 0,
    filename    TEXT,
    mime_type   TEXT NOT NULL,
    size        INTEGER NOT NULL DEFAULT 0,
    content_id  TEXT,
    remote_id   TEXT,
    local_path  TEXT
);

CREATE TABLE IF NOT EXISTS sync_states (
    account_id      TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    folder_id       TEXT NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    provider        TEXT NOT NULL,
    cursor_type     TEXT NOT NULL,
    cursor          TEXT NOT NULL,
    last_synced_at  INTEGER,
    last_error      TEXT,
    PRIMARY KEY (account_id, folder_id)
);

CREATE TABLE IF NOT EXISTS outbox (
    id            TEXT PRIMARY KEY,
    account_id    TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    message_id    TEXT NOT NULL,
    status        TEXT NOT NULL,
    retry_count   INTEGER NOT NULL DEFAULT 0,
    last_error    TEXT,
    payload       TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS outbox_attachments (
    job_id      TEXT NOT NULL REFERENCES outbox(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    filename    TEXT NOT NULL,
    mime_type   TEXT NOT NULL,
    data        BLOB NOT NULL,
    PRIMARY KEY (job_id, position)
);

CREATE INDEX IF NOT EXISTS idx_folders_account  ON folders(account_id);
CREATE INDEX IF NOT EXISTS idx_messages_folder  ON messages(folder_id, sent_at DESC);
CREATE INDEX IF NOT EXISTS idx_messages_account ON messages(account_id);
CREATE INDEX IF NOT EXISTS idx_messages_remote  ON messages(folder_id, remote_id);
CREATE INDEX IF NOT EXISTS idx_messages_thread  ON messages(thread_id);
CREATE INDEX IF NOT EXISTS idx_messages_subject ON messages(account_id, subject_normalized);
CREATE INDEX IF NOT EXISTS idx_addresses_msg    ON addresses(message_id);
CREATE INDEX IF NOT EXISTS idx_addresses_email  ON addresses(email);
CREATE INDEX IF NOT EXISTS idx_attachments_msg  ON attachments(message_id);
CREATE INDEX IF NOT EXISTS idx_outbox_status    ON outbox(status);
"#;

const SCHEMA_V2: &str = r#"
CREATE TABLE IF NOT EXISTS mail_events (
    id           TEXT PRIMARY KEY,
    account_id   TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    message_id   TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    detected_at  INTEGER NOT NULL,
    delivered_at INTEGER,
    UNIQUE (message_id)
);

CREATE INDEX IF NOT EXISTS idx_mail_events_pending ON mail_events(delivered_at, detected_at);
"#;

/// Threading signals beyond RFC 5322 headers: the Outlook `Thread-Index` root
/// and provider thread keys (`gmail-thrid:…`, `imap-thread:…`), plus the index
/// the ancestor lookup always needed.
const SCHEMA_V3: &str = r#"
ALTER TABLE messages ADD COLUMN thread_index_root TEXT;
ALTER TABLE messages ADD COLUMN server_thread_key TEXT;

CREATE INDEX IF NOT EXISTS idx_messages_msgid      ON messages(account_id, message_id_header);
CREATE INDEX IF NOT EXISTS idx_messages_thread_key ON messages(account_id, server_thread_key);
CREATE INDEX IF NOT EXISTS idx_messages_tidx_root  ON messages(account_id, thread_index_root);
"#;

const SCHEMA_V4: &str = r#"
-- Deliberately not tied to message deletion: re-syncing a removed local copy
-- must not send the same notification again.
CREATE TABLE IF NOT EXISTS mail_event_keys (
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    dedup_key TEXT NOT NULL,
    event_id TEXT NOT NULL,
    PRIMARY KEY (account_id, dedup_key)
);
CREATE INDEX IF NOT EXISTS idx_mail_event_keys_event ON mail_event_keys(event_id);
INSERT OR IGNORE INTO mail_event_keys (account_id, dedup_key, event_id)
SELECT account_id, dedup_key, id FROM (
    SELECT e.account_id, e.id,
           CASE WHEN length(trim(coalesce(m.message_id_header, ''), char(32,9,10,13))) > 0
                THEN 'header:' || trim(m.message_id_header, char(32,9,10,13))
                ELSE 'local:' || m.id END AS dedup_key,
           row_number() OVER (
               PARTITION BY e.account_id,
                   CASE WHEN length(trim(coalesce(m.message_id_header, ''), char(32,9,10,13))) > 0
                        THEN 'header:' || trim(m.message_id_header, char(32,9,10,13))
                        ELSE 'local:' || m.id END
               ORDER BY e.delivered_at IS NULL, e.detected_at, e.id
           ) AS position
    FROM mail_events e JOIN messages m ON m.id=e.message_id
    JOIN folders f ON f.id=m.folder_id
    WHERE f.kind='inbox' AND f.account_id=e.account_id
) WHERE position=1;
"#;

// Repair cached conversation dates left behind by writes without recomputation.
const SCHEMA_V5: &str = r#"
UPDATE threads SET
    message_count = (SELECT COUNT(*) FROM messages WHERE thread_id = threads.id),
    unread_count = (SELECT COUNT(*) FROM messages WHERE thread_id = threads.id AND is_read = 0),
    last_message_at = COALESCE((SELECT MAX(COALESCE(sent_at, received_at, 0))
                               FROM messages WHERE thread_id = threads.id), 0);
"#;

/// Apply outstanding migrations atomically.
pub fn migrate(conn: &mut Connection) -> Result<()> {
    let current: u32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(db_err)?;

    if current >= LATEST_VERSION {
        return Ok(());
    }

    tracing::info!(from = current, to = LATEST_VERSION, "migrating database");

    let tx = conn.transaction().map_err(db_err)?;
    if current < 1 {
        tx.execute_batch(SCHEMA_V1).map_err(db_err)?;
    }
    if current < 2 {
        tx.execute_batch(SCHEMA_V2).map_err(db_err)?;
    }
    if current < 3 {
        tx.execute_batch(SCHEMA_V3).map_err(db_err)?;
    }
    if current < 4 {
        tx.execute_batch(SCHEMA_V4).map_err(db_err)?;
    }
    if current < 5 {
        tx.execute_batch(SCHEMA_V5).map_err(db_err)?;
    }
    if current < 6 {
        // Existing folder copies also inflated the cached conversation counters.
        let ids = tx
            .prepare("SELECT id FROM threads")
            .map_err(db_err)?
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(db_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_err)?;
        for id in ids {
            crate::thread_repo::recompute(&tx, &id)?;
        }
    }
    tx.pragma_update(None, "user_version", LATEST_VERSION)
        .map_err(db_err)?;
    tx.commit().map_err(db_err)?;

    Ok(())
}

/// Normalise any SQLite failure into the unified error model.
pub(crate) fn db_err(err: rusqlite::Error) -> MailError {
    MailError::new(MailErrorCode::Storage, err.to_string()).with_source(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upgrades_v1_databases_to_mail_events() {
        let db = crate::Database::open_in_memory().unwrap();
        {
            let mut conn = db.lock();
            conn.execute_batch(
                "DROP TABLE mail_events;
                 DROP INDEX IF EXISTS idx_messages_msgid;
                 DROP INDEX IF EXISTS idx_messages_thread_key;
                 DROP INDEX IF EXISTS idx_messages_tidx_root;
                 ALTER TABLE messages DROP COLUMN thread_index_root;
                 ALTER TABLE messages DROP COLUMN server_thread_key;
                 PRAGMA user_version = 1;",
            )
            .map_err(db_err)
            .unwrap();
            migrate(&mut conn).unwrap();
            let version: u32 = conn
                .query_row("PRAGMA user_version", [], |row| row.get(0))
                .unwrap();
            assert_eq!(version, LATEST_VERSION);
            let columns: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('mail_events')",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(columns, 5);
        }
    }

    #[test]
    fn upgrades_v2_databases_to_threading_signals() {
        let db = crate::Database::open_in_memory().unwrap();
        {
            let mut conn = db.lock();
            // The DB above is already at LATEST_VERSION; unwind the v3 changes
            // so the upgrade under test starts from an honest v2 shape.
            conn.execute_batch(
                "DROP INDEX IF EXISTS idx_messages_msgid;
                 DROP INDEX IF EXISTS idx_messages_thread_key;
                 DROP INDEX IF EXISTS idx_messages_tidx_root;
                 ALTER TABLE messages DROP COLUMN thread_index_root;
                 ALTER TABLE messages DROP COLUMN server_thread_key;
                 PRAGMA user_version = 2;",
            )
            .map_err(db_err)
            .unwrap();
            migrate(&mut conn).unwrap();
            let version: u32 = conn
                .query_row("PRAGMA user_version", [], |row| row.get(0))
                .unwrap();
            assert_eq!(version, LATEST_VERSION);
            for column in ["thread_index_root", "server_thread_key"] {
                let found: i64 = conn
                    .query_row(
                        &format!(
                            "SELECT COUNT(*) FROM pragma_table_info('messages')
                              WHERE name = '{column}'"
                        ),
                        [],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(found, 1, "messages.{column} missing after migration");
            }
            let indexes: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index'
                      AND name IN ('idx_messages_msgid',
                                   'idx_messages_thread_key',
                                   'idx_messages_tidx_root')",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(indexes, 3);
        }
    }
}
