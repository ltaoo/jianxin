//! The outbox: durable send queue.
//!
//! Sending must not be a fire-and-forget SMTP call — on a flaky network the
//! user would lose the mail and get no way to retry. The pipeline is
//! `Compose → Draft → Outbox → Provider → Sent` (§29), which means a crash
//! between "clicked send" and "SMTP accepted" still leaves a job to retry.

use jianxin_domain::{ComposeAttachment, ComposeMessage, MailError, MailErrorCode, Result};
use rusqlite::{Connection, OptionalExtension};

use crate::migrations::db_err;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboxStatus {
    Pending,
    Sending,
    Sent,
    Failed,
}

impl OutboxStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            OutboxStatus::Pending => "pending",
            OutboxStatus::Sending => "sending",
            OutboxStatus::Sent => "sent",
            OutboxStatus::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "sending" => OutboxStatus::Sending,
            "sent" => OutboxStatus::Sent,
            "failed" => OutboxStatus::Failed,
            _ => OutboxStatus::Pending,
        }
    }
}

impl std::fmt::Display for OutboxStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One queued send.
///
/// `payload` holds the composed message; `data` on attachments is skipped by
/// serde, so the bytes live in `outbox_attachments` and survive a restart.
#[derive(Debug, Clone, serde::Serialize)]
pub struct OutboxJob {
    pub id: String,
    pub account_id: String,
    /// Local id of the message row representing this mail.
    pub message_id: String,
    pub status: OutboxStatus,
    pub retry_count: u32,
    pub last_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Queue a composed message for delivery.
pub fn enqueue(
    conn: &Connection,
    account_id: &str,
    message_id: &str,
    compose: &ComposeMessage,
) -> Result<OutboxJob> {
    let id = jianxin_domain::new_id();
    let now = jianxin_domain::now_ts();
    let payload = serde_json::to_string(compose).map_err(|e| {
        MailError::new(MailErrorCode::Storage, format!("cannot serialise draft: {e}"))
    })?;

    conn.execute(
        "INSERT INTO outbox (id, account_id, message_id, status, retry_count, last_error, payload, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, 0, NULL, ?5, ?6, ?7)",
        rusqlite::params![id, account_id, message_id, OutboxStatus::Pending.as_str(), payload, now, now],
    )
    .map_err(db_err)?;

    {
        let mut stmt = conn
            .prepare(
                "INSERT INTO outbox_attachments (job_id, position, filename, mime_type, data)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .map_err(db_err)?;
        for (i, a) in compose.attachments.iter().enumerate() {
            stmt.execute(rusqlite::params![
                id,
                i as i64,
                a.filename,
                a.mime_type,
                a.data.as_slice(),
            ])
            .map_err(db_err)?;
        }
    }

    Ok(OutboxJob {
        id,
        account_id: account_id.to_string(),
        message_id: message_id.to_string(),
        status: OutboxStatus::Pending,
        retry_count: 0,
        last_error: None,
        created_at: now,
        updated_at: now,
    })
}

/// Rebuild the `ComposeMessage` for a job, reattaching cached payloads.
pub fn load_payload(conn: &Connection, job_id: &str) -> Result<ComposeMessage> {
    let payload: String = conn
        .query_row("SELECT payload FROM outbox WHERE id = ?1", [job_id], |r| r.get(0))
        .map_err(db_err)?;
    let mut compose: ComposeMessage = serde_json::from_str(&payload).map_err(|e| {
        MailError::new(MailErrorCode::Storage, format!("cannot parse draft: {e}"))
    })?;

    let mut stmt = conn
        .prepare(
            "SELECT filename, mime_type, data FROM outbox_attachments
              WHERE job_id = ?1 ORDER BY position",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([job_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Vec<u8>>(2)?,
            ))
        })
        .map_err(db_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_err)?;

    compose.attachments = rows
        .into_iter()
        .map(|(filename, mime_type, data)| ComposeAttachment {
            filename,
            mime_type,
            data,
        })
        .collect();

    Ok(compose)
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<OutboxJob>> {
    conn.query_row(
        "SELECT id, account_id, message_id, status, retry_count, last_error, created_at, updated_at
           FROM outbox WHERE id = ?1",
        [id],
        row_to_job,
    )
    .optional()
    .map_err(db_err)
}

/// Jobs waiting to be delivered, oldest first.
pub fn list_pending(conn: &Connection) -> Result<Vec<OutboxJob>> {
    list_by_status(conn, OutboxStatus::Pending)
}

pub fn list_by_status(conn: &Connection, status: OutboxStatus) -> Result<Vec<OutboxJob>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, account_id, message_id, status, retry_count, last_error, created_at, updated_at
               FROM outbox WHERE status = ?1 ORDER BY created_at",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([status.as_str()], row_to_job)
        .map_err(db_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

pub fn list_all(conn: &Connection) -> Result<Vec<OutboxJob>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, account_id, message_id, status, retry_count, last_error, created_at, updated_at
               FROM outbox ORDER BY created_at DESC",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], row_to_job)
        .map_err(db_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

pub fn list_for_account(conn: &Connection, account_id: &str) -> Result<Vec<OutboxJob>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, account_id, message_id, status, retry_count, last_error, created_at, updated_at
               FROM outbox WHERE account_id = ?1 ORDER BY created_at DESC",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([account_id], row_to_job)
        .map_err(db_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

/// Move a job to `sending` so two workers cannot pick it up twice.
pub fn claim(conn: &Connection, id: &str) -> Result<bool> {
    let n = conn
        .execute(
            "UPDATE outbox SET status = ?2, updated_at = ?3
              WHERE id = ?1 AND status = ?4",
            rusqlite::params![
                id,
                OutboxStatus::Sending.as_str(),
                jianxin_domain::now_ts(),
                OutboxStatus::Pending.as_str(),
            ],
        )
        .map_err(db_err)?;
    Ok(n > 0)
}

pub fn mark_sent(conn: &Connection, id: &str) -> Result<()> {
    conn.execute(
        "UPDATE outbox SET status = ?2, last_error = NULL, updated_at = ?3 WHERE id = ?1",
        rusqlite::params![id, OutboxStatus::Sent.as_str(), jianxin_domain::now_ts()],
    )
    .map_err(db_err)?;
    // Payloads can be large; once delivered the bytes are no longer needed.
    conn.execute("DELETE FROM outbox_attachments WHERE job_id = ?1", [id])
        .map_err(db_err)?;
    Ok(())
}

/// Record a failure and send the job back to the queue for another attempt.
pub fn mark_failed(conn: &Connection, id: &str, error: &str) -> Result<()> {
    conn.execute(
        "UPDATE outbox
            SET status = ?2, retry_count = retry_count + 1, last_error = ?3, updated_at = ?4
          WHERE id = ?1",
        rusqlite::params![
            id,
            OutboxStatus::Failed.as_str(),
            error,
            jianxin_domain::now_ts()
        ],
    )
    .map_err(db_err)?;
    Ok(())
}

/// Requeue a job for another attempt.
///
/// Accepts both `failed` (manual retry from the CLI) and `sending` (a send
/// attempt that did not complete) — `sending` is what a job is left in while
/// the provider call is in flight, so that is the state a failure arrives in.
pub fn retry(conn: &Connection, id: &str) -> Result<bool> {
    let n = conn
        .execute(
            "UPDATE outbox SET status = ?2, updated_at = ?3
              WHERE id = ?1 AND status IN (?4, ?5)",
            rusqlite::params![
                id,
                OutboxStatus::Pending.as_str(),
                jianxin_domain::now_ts(),
                OutboxStatus::Failed.as_str(),
                OutboxStatus::Sending.as_str(),
            ],
        )
        .map_err(db_err)?;
    Ok(n > 0)
}

/// Only retry when the failure looked transient — a 403 should not be hammered
/// forever (§30).
pub fn retry_if_transient(conn: &Connection, id: &str, error: &MailError) -> Result<bool> {
    if error.code.is_retriable() {
        retry(conn, id)
    } else {
        mark_failed(conn, id, &error.to_string())?;
        Ok(false)
    }
}

pub fn delete(conn: &Connection, id: &str) -> Result<bool> {
    let n = conn
        .execute("DELETE FROM outbox WHERE id = ?1", [id])
        .map_err(db_err)?;
    Ok(n > 0)
}

/// Drop delivered jobs older than `before` so the table does not grow forever.
pub fn prune_sent(conn: &Connection, before: i64) -> Result<usize> {
    let n = conn
        .execute(
            "DELETE FROM outbox WHERE status = ?1 AND updated_at < ?2",
            rusqlite::params![OutboxStatus::Sent.as_str(), before],
        )
        .map_err(db_err)?;
    Ok(n)
}

fn row_to_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<OutboxJob> {
    let status: String = row.get(3)?;
    Ok(OutboxJob {
        id: row.get(0)?,
        account_id: row.get(1)?,
        message_id: row.get(2)?,
        status: OutboxStatus::parse(&status),
        retry_count: row.get::<_, u32>(4)?,
        last_error: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;
    use jianxin_domain::{Address, FolderKind, MailErrorCode};

    fn seed(db: &Database) {
        let conn = db.lock();
        crate::test_util::seed_account(&conn, "acc").unwrap();
        crate::test_util::seed_folder(&conn, "drafts", "acc", "Drafts", FolderKind::Drafts).unwrap();
    }

    fn compose() -> ComposeMessage {
        ComposeMessage {
            from: Some(Address::new("a@example.com")),
            to: vec![Address::new("b@example.com")],
            cc: vec![],
            bcc: vec![],
            subject: "Hi".to_string(),
            text_body: Some("body".to_string()),
            html_body: None,
            attachments: vec![ComposeAttachment::new(
                "note.txt",
                "text/plain",
                b"hello".to_vec(),
            )],
            in_reply_to: None,
            references: vec![],
        }
    }

    #[test]
    fn attachment_bytes_survive_a_round_trip() {
        let db = Database::open_in_memory().unwrap();
        seed(&db);
        let conn = db.lock();
        let job = enqueue(&conn, "acc", "msg-1", &compose()).unwrap();
        assert_eq!(job.status, OutboxStatus::Pending);

        let loaded = load_payload(&conn, &job.id).unwrap();
        assert_eq!(loaded.subject, "Hi");
        assert_eq!(loaded.attachments.len(), 1);
        assert_eq!(loaded.attachments[0].data, b"hello".to_vec());
    }

    #[test]
    fn claim_is_exclusive() {
        let db = Database::open_in_memory().unwrap();
        seed(&db);
        let conn = db.lock();
        let job = enqueue(&conn, "acc", "msg-1", &compose()).unwrap();
        assert!(claim(&conn, &job.id).unwrap());
        assert!(!claim(&conn, &job.id).unwrap());
        assert_eq!(get(&conn, &job.id).unwrap().unwrap().status, OutboxStatus::Sending);
    }

    #[test]
    fn transient_errors_requeue_permanent_ones_do_not() {
        let db = Database::open_in_memory().unwrap();
        seed(&db);
        let conn = db.lock();
        let job = enqueue(&conn, "acc", "msg-1", &compose()).unwrap();

        // A transient failure while the send was in flight goes back to pending.
        claim(&conn, &job.id).unwrap();
        let transient = MailError::new(MailErrorCode::Timeout, "timed out");
        assert!(retry_if_transient(&conn, &job.id, &transient).unwrap());
        assert_eq!(
            get(&conn, &job.id).unwrap().unwrap().status,
            OutboxStatus::Pending
        );

        // A permanent failure must not be retried forever (§30).
        claim(&conn, &job.id).unwrap();
        let permanent = MailError::new(MailErrorCode::AuthenticationFailed, "bad password");
        assert!(!retry_if_transient(&conn, &job.id, &permanent).unwrap());
        assert_eq!(get(&conn, &job.id).unwrap().unwrap().status, OutboxStatus::Failed);
        assert_eq!(get(&conn, &job.id).unwrap().unwrap().retry_count, 1);
    }

    #[test]
    fn prunes_delivered_jobs() {
        let db = Database::open_in_memory().unwrap();
        seed(&db);
        let conn = db.lock();
        let job = enqueue(&conn, "acc", "msg-1", &compose()).unwrap();
        mark_sent(&conn, &job.id).unwrap();
        assert_eq!(prune_sent(&conn, jianxin_domain::now_ts() + 1).unwrap(), 1);
    }
}
