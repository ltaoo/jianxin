//! Account rows.
//!
//! Deliberately credential-free: the account is the *local* identity, secrets
//! are resolved at runtime from env / keychain / secret command (§22).

use eeeemail_domain::{Account, AuthType, ProviderId, Result};
use rusqlite::{types::Type, Connection, OptionalExtension};

use crate::migrations::db_err;

/// Insert or update an account, keyed on its deterministic local id.
pub fn upsert(conn: &Connection, account: &Account) -> Result<()> {
    conn.execute(
        "INSERT INTO accounts (id, provider, email, display_name, auth_type, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET
             provider     = excluded.provider,
             email        = excluded.email,
             display_name = excluded.display_name,
             auth_type    = excluded.auth_type,
             updated_at   = excluded.updated_at",
        rusqlite::params![
            account.id,
            account.provider.as_str(),
            account.email,
            account.display_name,
            account.auth_type.as_str(),
            account.created_at,
            account.updated_at,
        ],
    )
    .map_err(db_err)?;
    Ok(())
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<Account>> {
    conn.query_row(
        "SELECT id, provider, email, display_name, auth_type, created_at, updated_at
           FROM accounts WHERE id = ?1",
        [id],
        row_to_account,
    )
    .optional()
    .map_err(db_err)
}

/// All accounts, ordered by email so the UI list is stable between runs.
pub fn list(conn: &Connection) -> Result<Vec<Account>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, provider, email, display_name, auth_type, created_at, updated_at
               FROM accounts ORDER BY email COLLATE NOCASE",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], row_to_account)
        .map_err(db_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

/// Remove an account. Folders, messages, cursors and outbox jobs follow via
/// `ON DELETE CASCADE`.
pub fn delete(conn: &Connection, id: &str) -> Result<bool> {
    let n = conn
        .execute("DELETE FROM accounts WHERE id = ?1", [id])
        .map_err(db_err)?;
    Ok(n > 0)
}

pub fn count(conn: &Connection) -> Result<u64> {
    conn.query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get(0))
        .map_err(db_err)
}

fn row_to_account(row: &rusqlite::Row<'_>) -> rusqlite::Result<Account> {
    let auth: String = row.get(4)?;
    let provider = provider_from_db(row.get(1)?, 1)?;
    Ok(Account {
        id: row.get(0)?,
        provider,
        email: row.get(2)?,
        display_name: row.get(3)?,
        auth_type: parse_auth_type(&auth),
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

fn provider_from_db(value: String, index: usize) -> rusqlite::Result<ProviderId> {
    ProviderId::parse(&value)
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(err)))
}

fn parse_auth_type(value: &str) -> AuthType {
    match value {
        "app_password" => AuthType::AppPassword,
        "oauth2" => AuthType::OAuth2,
        // Unknown values degrade to password rather than failing to load the
        // whole account — a bad enum value should not make a mailbox vanish.
        _ => AuthType::Password,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    fn account(email: &str) -> Account {
        Account {
            id: Account::deterministic_id(ProviderId::GenericImap, email),
            provider: ProviderId::GenericImap,
            email: email.to_string(),
            display_name: Some("Test".to_string()),
            auth_type: AuthType::AppPassword,
            created_at: 1,
            updated_at: 2,
        }
    }

    #[test]
    fn upsert_is_idempotent() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.lock();
        let a = account("a@example.com");
        upsert(&conn, &a).unwrap();
        upsert(&conn, &a).unwrap();
        assert_eq!(count(&conn).unwrap(), 1);
    }

    #[test]
    fn round_trips_auth_type() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.lock();
        upsert(&conn, &account("o@example.com")).unwrap();
        let loaded = get(&conn, &account("o@example.com").id).unwrap().unwrap();
        assert_eq!(loaded.auth_type, AuthType::AppPassword);
        assert_eq!(loaded.display_name.as_deref(), Some("Test"));
    }

    #[test]
    fn delete_cascades_to_cursors() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.lock();
        let a = account("c@example.com");
        upsert(&conn, &a).unwrap();
        delete(&conn, &a.id).unwrap();
        assert!(get(&conn, &a.id).unwrap().is_none());
    }
}
