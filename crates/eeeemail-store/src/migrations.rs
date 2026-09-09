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

use eeeemail_domain::{MailError, MailErrorCode, Result};
use rusqlite::Connection;

/// Current `PRAGMA user_version`. Bump whenever a `SCHEMA_V*` block is added.
pub const LATEST_VERSION: u32 = 1;

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

/// Apply any outstanding migrations.
///
/// Runs inside a single transaction so an interrupted upgrade cannot leave a
/// half-migrated database behind.
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
    tx.pragma_update(None, "user_version", LATEST_VERSION)
        .map_err(db_err)?;
    tx.commit().map_err(db_err)?;

    Ok(())
}

/// Normalise any SQLite failure into the unified error model.
pub(crate) fn db_err(err: rusqlite::Error) -> MailError {
    MailError::new(MailErrorCode::Storage, err.to_string()).with_source(err)
}
