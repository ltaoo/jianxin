//! Message rows plus their child rows (addresses, references, labels,
//! attachments).
//!
//! `remote_id` is opaque here. Nothing in this module may parse an IMAP UID or
//! a Graph id — it is just the string the provider gave us (§15).

use std::collections::{HashMap, HashSet};

use jianxin_domain::thread::normalise_subject;
use jianxin_domain::{Address, Attachment, FlagUpdate, Message, MessageId, Result};
use rusqlite::{Connection, OptionalExtension};

use crate::migrations::db_err;

const COLUMNS: &str = "id, account_id, folder_id, remote_id, immutable_id, thread_id, subject, \
     snippet, text_body, html_body, size, received_at, sent_at, message_id_header, in_reply_to, \
     is_read, is_starred, is_draft, is_deleted, provider_data, thread_index_root, server_thread_key";

/// Insert or replace messages and all their child rows.
///
/// Returns the number of rows written. Remote state wins: sync is the source of
/// truth for flags, and there is no local-only flag mutation queue in Phase 1.
pub fn upsert_many(conn: &Connection, messages: &[Message]) -> Result<usize> {
    let now = jianxin_domain::now_ts();

    let mut msg_stmt = conn
        .prepare(
            "INSERT INTO messages
                (id, account_id, folder_id, remote_id, immutable_id, thread_id, subject,
                 subject_normalized, snippet, text_body, html_body, size, received_at, sent_at,
                 message_id_header, in_reply_to, is_read, is_starred, is_draft, is_deleted,
                 provider_data, thread_index_root, server_thread_key, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                     ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)
             ON CONFLICT(account_id, folder_id, remote_id) DO UPDATE SET
                 immutable_id      = excluded.immutable_id,
                 -- Keep the thread we already computed: a re-sync that arrives
                 -- without one must not detach the message from its thread.
                 thread_id         = COALESCE(excluded.thread_id, messages.thread_id),
                 subject           = excluded.subject,
                 subject_normalized = excluded.subject_normalized,
                 snippet           = excluded.snippet,
                 text_body         = excluded.text_body,
                 html_body         = excluded.html_body,
                 size              = excluded.size,
                 received_at       = excluded.received_at,
                 sent_at           = excluded.sent_at,
                 message_id_header = excluded.message_id_header,
                 in_reply_to       = excluded.in_reply_to,
                 is_read           = excluded.is_read,
                 is_starred        = excluded.is_starred,
                 is_draft          = excluded.is_draft,
                 is_deleted        = excluded.is_deleted,
                 provider_data     = excluded.provider_data,
                 -- Threading signals arrive with body fetches only; a flag-only
                 -- re-sync passes NULL and must not erase what we stored.
                 thread_index_root = COALESCE(excluded.thread_index_root, messages.thread_index_root),
                 server_thread_key = COALESCE(excluded.server_thread_key, messages.server_thread_key),
                 updated_at        = excluded.updated_at",
        )
        .map_err(db_err)?;

    let mut touched_threads = HashSet::new();
    for m in messages {
        let stored_thread = || -> Result<Option<String>> {
            conn.query_row(
                "SELECT thread_id FROM messages WHERE account_id = ?1 AND folder_id = ?2 AND remote_id = ?3",
                rusqlite::params![m.account_id, m.folder_id, m.remote_id],
                |row| row.get::<_, Option<String>>(0),
            ).optional().map(|value| value.flatten()).map_err(db_err)
        };
        if let Some(id) = stored_thread()? {
            touched_threads.insert(id);
        }
        let provider_data = m
            .provider_data
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| {
                jianxin_domain::MailError::new(
                    jianxin_domain::MailErrorCode::Storage,
                    format!("cannot serialise provider_data: {e}"),
                )
            })?;

        msg_stmt
            .execute(rusqlite::params![
                m.id,
                m.account_id,
                m.folder_id,
                m.remote_id,
                m.immutable_id,
                m.thread_id,
                m.subject,
                normalise_subject(&m.subject),
                m.snippet,
                m.text_body,
                m.html_body,
                m.size as i64,
                m.received_at,
                m.sent_at,
                m.message_id_header,
                m.in_reply_to,
                m.is_read as i32,
                m.is_starred as i32,
                m.is_draft as i32,
                m.is_deleted as i32,
                provider_data,
                m.thread_index_root,
                m.server_thread_key,
                now,
            ])
            .map_err(db_err)?;

        write_children(conn, m)?;
        if let Some(id) = stored_thread()? {
            touched_threads.insert(id);
        }
    }
    for id in touched_threads {
        crate::thread_repo::recompute(conn, &id)?;
    }

    Ok(messages.len())
}

/// Replace the child rows of one message.
fn write_children(conn: &Connection, m: &Message) -> Result<()> {
    conn.execute("DELETE FROM addresses WHERE message_id = ?1", [&m.id])
        .map_err(db_err)?;
    conn.execute(
        "DELETE FROM message_references WHERE message_id = ?1",
        [&m.id],
    )
    .map_err(db_err)?;
    conn.execute("DELETE FROM message_labels WHERE message_id = ?1", [&m.id])
        .map_err(db_err)?;
    conn.execute("DELETE FROM attachments WHERE message_id = ?1", [&m.id])
        .map_err(db_err)?;

    {
        let mut stmt = conn
            .prepare(
                "INSERT INTO addresses (message_id, role, position, name, email)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .map_err(db_err)?;
        for (role, list) in [
            ("from", &m.from),
            ("to", &m.to),
            ("cc", &m.cc),
            ("bcc", &m.bcc),
            ("reply_to", &m.reply_to),
        ] {
            for (i, addr) in list.iter().enumerate() {
                stmt.execute(rusqlite::params![
                    m.id, role, i as i64, addr.name, addr.email
                ])
                .map_err(db_err)?;
            }
        }
    }

    if !m.references.is_empty() {
        let mut stmt = conn
            .prepare(
                "INSERT INTO message_references (message_id, position, ref_value)
                 VALUES (?1, ?2, ?3)",
            )
            .map_err(db_err)?;
        for (i, r) in m.references.iter().enumerate() {
            stmt.execute(rusqlite::params![m.id, i as i64, r])
                .map_err(db_err)?;
        }
    }

    if !m.labels.is_empty() {
        let mut stmt = conn
            .prepare("INSERT OR IGNORE INTO message_labels (message_id, label) VALUES (?1, ?2)")
            .map_err(db_err)?;
        for l in &m.labels {
            stmt.execute(rusqlite::params![m.id, l]).map_err(db_err)?;
        }
    }

    if !m.attachments.is_empty() {
        let mut stmt = conn
            .prepare(
                "INSERT INTO attachments
                    (id, message_id, position, filename, mime_type, size, content_id, remote_id, local_path)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )
            .map_err(db_err)?;
        for (i, a) in m.attachments.iter().enumerate() {
            stmt.execute(rusqlite::params![
                a.id,
                m.id,
                i as i64,
                a.filename,
                a.mime_type,
                a.size as i64,
                a.content_id,
                a.remote_id,
                a.local_path,
            ])
            .map_err(db_err)?;
        }
    }

    Ok(())
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<Message>> {
    let sql = format!("SELECT {COLUMNS} FROM messages WHERE id = ?1");
    let mut m: Option<Message> = conn
        .query_row(&sql, [id], row_to_message)
        .optional()
        .map_err(db_err)?;
    if let Some(m) = m.as_mut() {
        hydrate(conn, std::slice::from_mut(m))?;
    }
    Ok(m)
}

pub fn get_by_remote_id(
    conn: &Connection,
    folder_id: &str,
    remote_id: &str,
) -> Result<Option<Message>> {
    let sql = format!("SELECT {COLUMNS} FROM messages WHERE folder_id = ?1 AND remote_id = ?2");
    let mut m: Option<Message> = conn
        .query_row(
            &sql,
            rusqlite::params![folder_id, remote_id],
            row_to_message,
        )
        .optional()
        .map_err(db_err)?;
    if let Some(m) = m.as_mut() {
        hydrate(conn, std::slice::from_mut(m))?;
    }
    Ok(m)
}

/// One page of a folder listing, newest first.
pub fn list(
    conn: &Connection,
    folder_id: &str,
    limit: u32,
    offset: u32,
    unread_only: bool,
) -> Result<Vec<Message>> {
    let unread_clause = if unread_only { "AND is_read = 0" } else { "" };
    let sql = format!(
        "SELECT {COLUMNS} FROM messages
          WHERE folder_id = ?1 AND is_deleted = 0 {unread_clause}
          ORDER BY COALESCE(sent_at, received_at, 0) DESC, remote_id DESC
          LIMIT ?2 OFFSET ?3"
    );
    let mut stmt = conn.prepare(&sql).map_err(db_err)?;
    let mut out = stmt
        .query_map(rusqlite::params![folder_id, limit, offset], row_to_message)
        .map_err(db_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_err)?;
    hydrate(conn, &mut out)?;
    Ok(out)
}

/// Every stored message of an account, oldest first — the input for a full
/// thread rebuild.
pub fn list_account(conn: &Connection, account_id: &str) -> Result<Vec<Message>> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {COLUMNS} FROM messages
              WHERE account_id = ?1 AND is_deleted = 0
              ORDER BY COALESCE(sent_at, received_at, 0) ASC, id ASC"
        ))
        .map_err(db_err)?;
    let mut out = stmt
        .query_map(rusqlite::params![account_id], row_to_message)
        .map_err(db_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_err)?;
    hydrate(conn, &mut out)?;
    Ok(out)
}

/// Collapse per-folder copies of one mail into a single row.
///
/// A message is stored once per folder (`ON CONFLICT(account_id, folder_id,
/// remote_id)`), and Gmail materialises every label as a folder, so the same
/// mail appears two or three times. The surviving copy is the one in the most
/// significant folder — inbox first — because that is the row a flag action
/// should target. `immutable_id` is the stable key when the provider gives
/// one; `message_id_header` is the next best; the row id never collapses.
pub(crate) const DEDUP_RANK: &str = "ROW_NUMBER() OVER (
    PARTITION BY m.account_id,
                 COALESCE(NULLIF(m.immutable_id, ''), m.message_id_header, m.id)
    ORDER BY CASE COALESCE(f.kind, '')
                 WHEN 'inbox'   THEN 0
                 WHEN 'archive' THEN 1
                 WHEN 'sent'    THEN 2
                 WHEN 'drafts'  THEN 3
                 WHEN 'custom'  THEN 4
                 WHEN 'spam'    THEN 5
                 ELSE 6 END,
             m.id
)";

/// One page across every account, newest first, one row per real message.
pub fn list_all(
    conn: &Connection,
    limit: u32,
    offset: u32,
    unread_only: bool,
) -> Result<Vec<Message>> {
    let unread_clause = if unread_only { "AND m.is_read = 0" } else { "" };
    let sql = format!(
        "SELECT {COLUMNS} FROM (
             SELECT m.*, {DEDUP_RANK} AS dedup_rank
               FROM messages m
               LEFT JOIN folders f ON f.id = m.folder_id
              WHERE m.is_deleted = 0 {unread_clause}
         )
         WHERE dedup_rank = 1
         ORDER BY COALESCE(sent_at, received_at, 0) DESC, id DESC
         LIMIT ?1 OFFSET ?2"
    );
    let mut stmt = conn.prepare(&sql).map_err(db_err)?;
    let mut out = stmt
        .query_map(rusqlite::params![limit, offset], row_to_message)
        .map_err(db_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_err)?;
    hydrate(conn, &mut out)?;
    Ok(out)
}

/// Unread count for the merged view, deduplicated like `list_all`.
pub fn count_all_unread(conn: &Connection) -> Result<u64> {
    let sql = format!(
        "SELECT COUNT(*) FROM (
             SELECT {DEDUP_RANK} AS dedup_rank
               FROM messages m
               LEFT JOIN folders f ON f.id = m.folder_id
              WHERE m.is_deleted = 0 AND m.is_read = 0
         )
         WHERE dedup_rank = 1"
    );
    conn.query_row(&sql, [], |r| r.get(0)).map_err(db_err)
}

/// Search across every account, newest first, deduplicated like `list_all`.
pub fn search_all(conn: &Connection, query: &str, limit: u32) -> Result<Vec<Message>> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let like = format!("%{}%", query.replace('%', "\\%"));

    let sql = format!(
        "SELECT {COLUMNS} FROM (
             SELECT m.*, {DEDUP_RANK} AS dedup_rank
               FROM messages m
               LEFT JOIN folders f ON f.id = m.folder_id
              WHERE m.is_deleted = 0
                AND (
                      m.subject   LIKE ?1 ESCAPE '\\'
                   OR m.snippet    LIKE ?1 ESCAPE '\\'
                   OR m.text_body LIKE ?1 ESCAPE '\\'
                   OR EXISTS (
                        SELECT 1 FROM addresses a
                         WHERE a.message_id = m.id
                           AND (a.email LIKE ?1 ESCAPE '\\' OR a.name LIKE ?1 ESCAPE '\\'))
                )
         )
         WHERE dedup_rank = 1
         ORDER BY COALESCE(sent_at, received_at, 0) DESC
         LIMIT ?2"
    );
    let mut stmt = conn.prepare(&sql).map_err(db_err)?;
    let mut out = stmt
        .query_map(rusqlite::params![like, limit], row_to_message)
        .map_err(db_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_err)?;
    hydrate(conn, &mut out)?;
    Ok(out)
}

pub fn count(conn: &Connection, folder_id: &str, unread_only: bool) -> Result<u64> {
    let sql = if unread_only {
        "SELECT COUNT(*) FROM messages WHERE folder_id = ?1 AND is_deleted = 0 AND is_read = 0"
    } else {
        "SELECT COUNT(*) FROM messages WHERE folder_id = ?1 AND is_deleted = 0"
    };
    conn.query_row(sql, [folder_id], |r| r.get(0))
        .map_err(db_err)
}

/// Apply flag-only changes reported by an incremental sync.
pub fn apply_flag_updates(
    conn: &Connection,
    folder_id: &str,
    updates: &[FlagUpdate],
) -> Result<usize> {
    if updates.is_empty() {
        return Ok(0);
    }
    let mut stmt = conn
        .prepare(
            "UPDATE messages SET is_read = ?3, is_starred = ?4, updated_at = ?5
              WHERE folder_id = ?1 AND remote_id = ?2",
        )
        .map_err(db_err)?;
    let now = jianxin_domain::now_ts();
    let mut n = 0;
    for u in updates {
        n += stmt
            .execute(rusqlite::params![
                folder_id,
                u.remote_id,
                u.is_read as i32,
                u.is_starred as i32,
                now,
            ])
            .map_err(db_err)?;
    }
    Ok(n)
}

/// Every remote id currently stored for a folder.
///
/// IMAP has no change feed, so the sync engine hands back the remote ids it
/// still sees and this set is diffed locally to find deletions.
pub fn remote_ids_in_folder(conn: &Connection, folder_id: &str) -> Result<Vec<String>> {
    let mut stmt = conn
        .prepare("SELECT remote_id FROM messages WHERE folder_id = ?1")
        .map_err(db_err)?;
    let rows = stmt
        .query_map([folder_id], |r| r.get::<_, String>(0))
        .map_err(db_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

/// Every local message id for an account, used by the watcher to diff a sync.
pub fn ids_for_account(conn: &Connection, account_id: &str) -> Result<Vec<MessageId>> {
    let mut stmt = conn
        .prepare("SELECT id FROM messages WHERE account_id = ?1")
        .map_err(db_err)?;
    let rows = stmt
        .query_map([account_id], |row| row.get::<_, String>(0))
        .map_err(db_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

/// Delete messages that vanished remotely.
pub fn remove_by_remote_ids(
    conn: &Connection,
    folder_id: &str,
    remote_ids: &[String],
) -> Result<usize> {
    if remote_ids.is_empty() {
        return Ok(0);
    }
    let placeholders = crate::placeholders(remote_ids.len());
    let sql =
        format!("DELETE FROM messages WHERE folder_id = ?1 AND remote_id IN ({placeholders})");
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(folder_id.to_string())];
    for id in remote_ids {
        params.push(Box::new(id.clone()));
    }
    conn.execute(
        &sql,
        rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
    )
    .map_err(db_err)
}

pub fn set_read(conn: &Connection, id: &str, read: bool) -> Result<()> {
    conn.execute(
        "UPDATE messages SET is_read = ?2, updated_at = ?3 WHERE id = ?1",
        rusqlite::params![id, read as i32, jianxin_domain::now_ts()],
    )
    .map_err(db_err)?;
    Ok(())
}

pub fn set_starred(conn: &Connection, id: &str, starred: bool) -> Result<()> {
    conn.execute(
        "UPDATE messages SET is_starred = ?2, updated_at = ?3
         WHERE (account_id, COALESCE(NULLIF(immutable_id, ''), message_id_header, id)) IN (
           SELECT account_id, COALESCE(NULLIF(immutable_id, ''), message_id_header, id)
           FROM messages WHERE id = ?1
         )",
        rusqlite::params![id, starred as i32, jianxin_domain::now_ts()],
    )
    .map_err(db_err)?;
    Ok(())
}

/// Move a message between folders.
///
/// The remote id is replaced because providers reissue ids on move (Graph
/// explicitly warns about this), so keeping the old one would silently corrupt
/// the next incremental sync.
pub fn move_to_folder(
    conn: &Connection,
    id: &str,
    folder_id: &str,
    new_remote_id: Option<&str>,
) -> Result<()> {
    match new_remote_id {
        Some(remote) => conn
            .execute(
                "UPDATE messages SET folder_id = ?2, remote_id = ?3, updated_at = ?4 WHERE id = ?1",
                rusqlite::params![id, folder_id, remote, jianxin_domain::now_ts()],
            )
            .map_err(db_err)?,
        None => conn
            .execute(
                "UPDATE messages SET folder_id = ?2, updated_at = ?3 WHERE id = ?1",
                rusqlite::params![id, folder_id, jianxin_domain::now_ts()],
            )
            .map_err(db_err)?,
    };
    Ok(())
}

/// A soft delete: the row stays until the provider confirms removal.
pub fn set_deleted(conn: &Connection, id: &str, deleted: bool) -> Result<()> {
    conn.execute(
        "UPDATE messages SET is_deleted = ?2, updated_at = ?3 WHERE id = ?1",
        rusqlite::params![id, deleted as i32, jianxin_domain::now_ts()],
    )
    .map_err(db_err)?;
    Ok(())
}

pub fn delete(conn: &Connection, id: &str) -> Result<bool> {
    let n = conn
        .execute("DELETE FROM messages WHERE id = ?1", [id])
        .map_err(db_err)?;
    Ok(n > 0)
}

/// Free-text search over the local copy.
///
/// Uses `LIKE` rather than FTS5: the `unicode61` tokenizer does not segment
/// Chinese, and returning nothing for `发票` would be worse than scanning a few
/// thousand rows.
pub fn search(
    conn: &Connection,
    account_id: &str,
    query: &str,
    limit: u32,
) -> Result<Vec<Message>> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let like = format!("%{}%", query.replace('%', "\\%"));

    let sql = format!(
        "SELECT {COLUMNS} FROM messages m
          WHERE m.account_id = ?1
            AND m.is_deleted = 0
            AND (
                  m.subject LIKE ?2 ESCAPE '\\'
               OR m.snippet  LIKE ?2 ESCAPE '\\'
               OR m.text_body LIKE ?2 ESCAPE '\\'
               OR EXISTS (
                    SELECT 1 FROM addresses a
                     WHERE a.message_id = m.id
                       AND (a.email LIKE ?2 ESCAPE '\\' OR a.name LIKE ?2 ESCAPE '\\'))
            )
          ORDER BY COALESCE(m.sent_at, m.received_at, 0) DESC
          LIMIT ?3"
    );

    let mut stmt = conn.prepare(&sql).map_err(db_err)?;
    let mut out = stmt
        .query_map(rusqlite::params![account_id, like, limit], row_to_message)
        .map_err(db_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_err)?;
    hydrate(conn, &mut out)?;
    Ok(out)
}

/// Fill in the child collections for a batch of messages in four queries
/// instead of 4×N.
fn hydrate(conn: &Connection, messages: &mut [Message]) -> Result<()> {
    if messages.is_empty() {
        return Ok(());
    }
    // Owned clones: a borrowed `Vec<&str>` would keep `messages` immutably
    // borrowed for the whole function and block the mutation below.
    let ids: Vec<String> = messages.iter().map(|m| m.id.clone()).collect();
    let placeholders = crate::placeholders(ids.len());
    let param = || -> Vec<Box<dyn rusqlite::ToSql>> {
        ids.iter()
            .map(|id| Box::new(id.clone()) as Box<dyn rusqlite::ToSql>)
            .collect()
    };

    // Map id -> position rather than holding `&mut Message`: the ids vector
    // borrows `messages` immutably for the whole function.
    let mut index: HashMap<String, usize> = HashMap::with_capacity(messages.len());
    for (i, m) in messages.iter().enumerate() {
        index.insert(m.id.clone(), i);
    }

    // Addresses.
    {
        let sql = format!(
            "SELECT message_id, role, name, email FROM addresses
              WHERE message_id IN ({placeholders}) ORDER BY position"
        );
        let mut stmt = conn.prepare(&sql).map_err(db_err)?;
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(param().iter().map(|p| p.as_ref())),
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                },
            )
            .map_err(db_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_err)?;

        for (mid, role, name, email) in rows {
            if let Some(&i) = index.get(&mid) {
                let addr = Address { name, email };
                match role.as_str() {
                    "from" => messages[i].from.push(addr),
                    "to" => messages[i].to.push(addr),
                    "cc" => messages[i].cc.push(addr),
                    "bcc" => messages[i].bcc.push(addr),
                    _ => messages[i].reply_to.push(addr),
                }
            }
        }
    }

    // References.
    {
        let sql = format!(
            "SELECT message_id, ref_value FROM message_references
              WHERE message_id IN ({placeholders}) ORDER BY position"
        );
        let mut stmt = conn.prepare(&sql).map_err(db_err)?;
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(param().iter().map(|p| p.as_ref())),
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .map_err(db_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_err)?;
        for (mid, r) in rows {
            if let Some(&i) = index.get(&mid) {
                messages[i].references.push(r);
            }
        }
    }

    // Labels.
    {
        let sql = format!(
            "SELECT message_id, label FROM message_labels WHERE message_id IN ({placeholders})"
        );
        let mut stmt = conn.prepare(&sql).map_err(db_err)?;
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(param().iter().map(|p| p.as_ref())),
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .map_err(db_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_err)?;
        for (mid, l) in rows {
            if let Some(&i) = index.get(&mid) {
                messages[i].labels.push(l);
            }
        }
    }

    // Attachments.
    {
        let sql = format!(
            "SELECT id, message_id, filename, mime_type, size, content_id, remote_id, local_path
               FROM attachments WHERE message_id IN ({placeholders}) ORDER BY position"
        );
        let mut stmt = conn.prepare(&sql).map_err(db_err)?;
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(param().iter().map(|p| p.as_ref())),
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, i64>(4)?,
                        r.get::<_, Option<String>>(5)?,
                        r.get::<_, Option<String>>(6)?,
                        r.get::<_, Option<String>>(7)?,
                    ))
                },
            )
            .map_err(db_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_err)?;

        for (id, mid, filename, mime_type, size, content_id, remote_id, local_path) in rows {
            if let Some(&i) = index.get(&mid) {
                messages[i].attachments.push(Attachment {
                    id,
                    message_id: mid,
                    filename,
                    mime_type,
                    size: size.max(0) as u64,
                    content_id,
                    remote_id,
                    local_path,
                });
            }
        }
    }

    Ok(())
}

/// Record that a payload was cached locally so the reader can use it offline.
pub fn set_attachment_path(conn: &Connection, attachment_id: &str, path: &str) -> Result<()> {
    conn.execute(
        "UPDATE attachments SET local_path = ?2 WHERE id = ?1",
        rusqlite::params![attachment_id, path],
    )
    .map_err(db_err)?;
    Ok(())
}

fn row_to_message(row: &rusqlite::Row<'_>) -> rusqlite::Result<Message> {
    let provider_data: Option<String> = row.get(19)?;
    let provider_data =
        provider_data.and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());

    let m = Message {
        id: row.get::<_, MessageId>(0)?,
        account_id: row.get(1)?,
        folder_id: row.get(2)?,
        remote_id: row.get(3)?,
        immutable_id: row.get(4)?,
        thread_id: row.get(5)?,
        subject: row.get(6)?,
        snippet: row.get(7)?,
        text_body: row.get(8)?,
        html_body: row.get(9)?,
        size: row.get::<_, i64>(10)?.max(0) as u64,
        received_at: row.get(11)?,
        sent_at: row.get(12)?,
        message_id_header: row.get(13)?,
        in_reply_to: row.get(14)?,
        is_read: row.get::<_, i32>(15)? != 0,
        is_starred: row.get::<_, i32>(16)? != 0,
        is_draft: row.get::<_, i32>(17)? != 0,
        is_deleted: row.get::<_, i32>(18)? != 0,
        thread_index_root: row.get(20)?,
        server_thread_key: row.get(21)?,
        labels: Vec::new(),
        attachments: Vec::new(),
        from: Vec::new(),
        to: Vec::new(),
        cc: Vec::new(),
        bcc: Vec::new(),
        reply_to: Vec::new(),
        references: Vec::new(),
        provider_data,
    };
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;
    use jianxin_domain::Folder;

    fn seed(db: &Database) -> String {
        let conn = db.lock();
        crate::test_util::seed_account(&conn, "acc").unwrap();
        let f = Folder::new("acc", "INBOX", "Inbox", jianxin_domain::FolderKind::Inbox);
        let folder_id = f.id.clone();
        crate::folder_repo::upsert_many(&conn, &[f]).unwrap();
        folder_id
    }

    fn msg(folder_id: &str, remote: &str, subject: &str) -> Message {
        let mut m = Message::new();
        m.id = Message::deterministic_id("acc", folder_id, remote);
        m.account_id = "acc".to_string();
        m.folder_id = folder_id.to_string();
        m.remote_id = remote.to_string();
        m.subject = subject.to_string();
        m.snippet = Some("hello world".to_string());
        m.text_body = Some("invoice attached".to_string());
        m.sent_at = Some(1000);
        m.from = vec![Address::with_name("Bob", "bob@example.com")];
        m.to = vec![Address::new("a@example.com")];
        m.references = vec!["<root@example.com>".to_string()];
        m.labels = vec!["important".to_string()];
        m.attachments = vec![Attachment::new(
            &m.id,
            Some("a.pdf".to_string()),
            "application/pdf",
            42,
        )];
        m
    }

    #[test]
    fn round_trips_children() {
        let db = Database::open_in_memory().unwrap();
        let folder = seed(&db);
        let conn = db.lock();
        let m = msg(&folder, "1", "Hello");
        let id = m.id.clone();
        upsert_many(&conn, &[m]).unwrap();

        let loaded = get(&conn, &id).unwrap().unwrap();
        assert_eq!(loaded.subject, "Hello");
        assert_eq!(loaded.from.len(), 1);
        assert_eq!(loaded.from[0].name.as_deref(), Some("Bob"));
        assert_eq!(loaded.to[0].email, "a@example.com");
        assert_eq!(loaded.references, vec!["<root@example.com>"]);
        assert_eq!(loaded.labels, vec!["important"]);
        assert_eq!(loaded.attachments.len(), 1);
        assert_eq!(loaded.attachments[0].filename.as_deref(), Some("a.pdf"));
    }

    #[test]
    fn upsert_does_not_duplicate() {
        let db = Database::open_in_memory().unwrap();
        let folder = seed(&db);
        let conn = db.lock();
        upsert_many(&conn, &[msg(&folder, "1", "One")]).unwrap();
        upsert_many(&conn, &[msg(&folder, "1", "Renamed")]).unwrap();
        assert_eq!(count(&conn, &folder, false).unwrap(), 1);
        let all = list(&conn, &folder, 10, 0, false).unwrap();
        assert_eq!(all[0].subject, "Renamed");
    }

    #[test]
    fn flag_updates_and_removals() {
        let db = Database::open_in_memory().unwrap();
        let folder = seed(&db);
        let conn = db.lock();
        upsert_many(&conn, &[msg(&folder, "1", "A"), msg(&folder, "2", "B")]).unwrap();

        let n = apply_flag_updates(
            &conn,
            &folder,
            &[FlagUpdate {
                remote_id: "1".to_string(),
                is_read: true,
                is_starred: true,
            }],
        )
        .unwrap();
        assert_eq!(n, 1);
        assert!(count(&conn, &folder, true).unwrap() == 1);

        let removed = remove_by_remote_ids(&conn, &folder, &["2".to_string()]).unwrap();
        assert_eq!(removed, 1);
        assert_eq!(count(&conn, &folder, false).unwrap(), 1);
    }

    #[test]
    fn searches_subject_body_and_sender() {
        let db = Database::open_in_memory().unwrap();
        let folder = seed(&db);
        let conn = db.lock();
        upsert_many(&conn, &[msg(&folder, "1", "Hello")]).unwrap();

        assert_eq!(search(&conn, "acc", "hello", 10).unwrap().len(), 1);
        assert_eq!(search(&conn, "acc", "invoice", 10).unwrap().len(), 1);
        assert_eq!(search(&conn, "acc", "bob@", 10).unwrap().len(), 1);
        assert_eq!(search(&conn, "acc", "发票", 10).unwrap().len(), 0);
        assert_eq!(search(&conn, "acc", "", 10).unwrap().len(), 0);
    }

    #[test]
    fn lists_newest_first() {
        let db = Database::open_in_memory().unwrap();
        let folder = seed(&db);
        let conn = db.lock();
        let mut a = msg(&folder, "1", "A");
        a.sent_at = Some(10);
        let mut b = msg(&folder, "2", "B");
        b.sent_at = Some(99);
        upsert_many(&conn, &[a, b]).unwrap();

        let all = list(&conn, &folder, 10, 0, false).unwrap();
        assert_eq!(all[0].subject, "B");
    }

    #[test]
    fn list_all_merges_accounts_and_collapses_folder_copies() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.lock();
        let mut seed_two_accounts = |tag: &str| -> (String, String) {
            crate::test_util::seed_account(&conn, tag).unwrap();
            let inbox = Folder::new(tag, "INBOX", "Inbox", jianxin_domain::FolderKind::Inbox);
            let archive = Folder::new(tag, "All", "All", jianxin_domain::FolderKind::Archive);
            let ids = (inbox.id.clone(), archive.id.clone());
            crate::folder_repo::upsert_many(&conn, &[inbox, archive]).unwrap();
            ids
        };
        let (a_inbox, a_archive) = seed_two_accounts("accA");
        let (b_inbox, _) = seed_two_accounts("accB");

        // One mail filed in two folders of account A (the Gmail label case),
        // plus a distinct mail in account B.
        let mut copy1 = msg(&a_inbox, "1", "Duplicated");
        copy1.account_id = "accA".to_string();
        copy1.message_id_header = Some("<dup@example.com>".to_string());
        let mut copy2 = msg(&a_archive, "9", "Duplicated");
        copy2.account_id = "accA".to_string();
        copy2.message_id_header = Some("<dup@example.com>".to_string());
        let mut other = msg(&b_inbox, "1", "Only in B");
        other.account_id = "accB".to_string();
        other.message_id_header = Some("<b@example.com>".to_string());
        upsert_many(&conn, &[copy1, copy2, other]).unwrap();

        let all = list_all(&conn, 10, 0, false).unwrap();
        assert_eq!(all.len(), 2);
        // The inbox copy survives, not the archive one.
        assert!(
            all.iter()
                .all(|m| m.subject != "Duplicated" || m.folder_id == a_inbox)
        );
        assert!(
            all.iter()
                .any(|m| m.subject == "Only in B" && m.account_id == "accB")
        );

        let found = search_all(&conn, "duplicated", 10).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].folder_id, a_inbox);
    }
}
