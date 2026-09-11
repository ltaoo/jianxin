//! # eeeemail-core
//!
//! The application layer: it owns the config, the local database and the
//! provider registry, and exposes three services on top of them.
//!
//! ```text
//! CLI ─┐
//!      ├─ MailService ──┐
//! TUI ─┘                ├─ SyncEngine ── MailProvider
//!        ComposeService ┘        │
//!                            SQLite
//! ```
//!
//! The UI never constructs a provider or writes SQL directly. It calls a
//! service, and the service decides whether it can be served from the local
//! database (the common case) or needs the network.

pub mod compose;
pub mod mail;
pub mod sync;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use eeeemail_config::{AccountConfig, AppConfig, SyncConfig};
use eeeemail_domain::{Account, AccountId, MailError, MailErrorCode, Result};
use eeeemail_provider::{MailProvider, ProviderRegistry};
use eeeemail_store::Database;

pub use compose::ComposeService;
pub use mail::{MailService, ThreadDetail};
pub use sync::{SyncEngine, SyncReport};

/// Wires together config, storage and providers.
///
/// `Send + Sync` on purpose: the TUI holds one `Arc<App>` and runs sync on a
/// background thread while the render loop keeps painting.
pub struct App {
    config: AppConfig,
    db: Arc<Database>,
    registry: ProviderRegistry,
    /// Built lazily and cached — constructing a provider validates config but
    /// does not open a socket.
    providers: RwLock<HashMap<AccountId, Arc<dyn MailProvider>>>,
}

impl App {
    /// Open (or create) the database under the configured data directory.
    pub fn new(config: AppConfig) -> Result<Self> {
        let db = Database::open(db_path(&config))?;
        Self::with_database(config, db)
    }

    /// Build around an existing database — used by tests and by commands that
    /// want an in-memory store.
    pub fn with_database(config: AppConfig, db: Database) -> Result<Self> {
        let app = Self {
            config,
            db: Arc::new(db),
            registry: ProviderRegistry::builtin(),
            providers: RwLock::new(HashMap::new()),
        };
        app.import_accounts()?;
        Ok(app)
    }

    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    pub fn sync_config(&self) -> &SyncConfig {
        &self.config.general.sync
    }

    pub fn db(&self) -> &Arc<Database> {
        &self.db
    }

    pub fn database_path(&self) -> Option<PathBuf> {
        self.db.path().map(|p| p.to_path_buf())
    }

    // --- accounts ---------------------------------------------------------

    pub fn account_configs(&self) -> &[AccountConfig] {
        &self.config.accounts
    }

    /// Replace the in-memory config after an account-management tool edits it.
    pub fn reload_config(&mut self, config: AppConfig) -> Result<Vec<Account>> {
        self.config = config;
        self.providers.write().unwrap().clear();
        self.import_accounts()
    }

    /// Copy configured accounts into the local database.
    ///
    /// Ids are deterministic (`provider + email`), so re-running this never
    /// duplicates rows and never loses a sync cursor.
    pub fn import_accounts(&self) -> Result<Vec<Account>> {
        let conn = self.db.lock();
        let mut out = Vec::with_capacity(self.config.accounts.len());
        for cfg in &self.config.accounts {
            cfg.validate()?;
            let account = cfg.to_account();
            eeeemail_store::account_repo::upsert(&conn, &account)?;
            out.push(account);
        }
        // Drop rows for accounts that were removed from the config.
        let keep: Vec<String> = self
            .config
            .accounts
            .iter()
            .map(|c| c.to_account().id)
            .collect();
        for stale in eeeemail_store::account_repo::list(&conn)?
            .into_iter()
            .filter(|a| !keep.contains(&a.id))
        {
            tracing::info!(account = %stale.email, "removing account no longer in config");
            eeeemail_store::account_repo::delete(&conn, &stale.id)?;
        }
        Ok(out)
    }

    pub fn accounts(&self) -> Result<Vec<Account>> {
        eeeemail_store::account_repo::list(&self.db.lock())
    }

    /// Pick an account by alias, email, local id, or — when `selector` is
    /// `None` and there is exactly one — the only account.
    pub fn resolve_account(&self, selector: Option<&str>) -> Result<AccountConfig> {
        match selector {
            Some(s) => self
                .config
                .find(s)
                .cloned()
                .or_else(|| {
                    self.config
                        .accounts
                        .iter()
                        .find(|cfg| cfg.to_account().id == s)
                        .cloned()
                })
                .ok_or_else(|| {
                    MailError::new(
                        MailErrorCode::InvalidConfig,
                        format!(
                            "no account matching `{s}` (known: {})",
                            self.known_aliases()
                        ),
                    )
                }),
            None if self.config.accounts.len() == 1 => Ok(self.config.accounts[0].clone()),
            None => Err(MailError::new(
                MailErrorCode::InvalidConfig,
                format!(
                    "which account? pass --account (known: {})",
                    self.known_aliases()
                ),
            )),
        }
    }

    /// The local `Account` row for a configured account.
    pub fn account_row(&self, cfg: &AccountConfig) -> Account {
        cfg.to_account()
    }

    fn known_aliases(&self) -> String {
        let names: Vec<String> = self
            .config
            .accounts
            .iter()
            .map(|a| a.alias().to_string())
            .collect();
        if names.is_empty() {
            "(none configured)".to_string()
        } else {
            names.join(", ")
        }
    }

    // --- providers --------------------------------------------------------

    /// The provider for one configured account, built on first use.
    pub fn provider(&self, cfg: &AccountConfig) -> Result<Arc<dyn MailProvider>> {
        let id = cfg.to_account().id;

        if let Some(p) = self.providers.read().unwrap().get(&id) {
            return Ok(Arc::clone(p));
        }

        let provider: Arc<dyn MailProvider> =
            Arc::from(self.registry.build(cfg, self.sync_config())?);
        self.providers
            .write()
            .unwrap()
            .insert(id, Arc::clone(&provider));
        Ok(provider)
    }

    /// Forget cached providers so the next call re-reads secrets.
    pub fn reset_providers(&self) {
        self.providers.write().unwrap().clear();
    }

    // --- services ---------------------------------------------------------

    pub fn sync(&self) -> SyncEngine<'_> {
        SyncEngine::new(self)
    }

    pub fn mail(&self) -> MailService<'_> {
        MailService::new(self)
    }

    pub fn compose(&self) -> ComposeService<'_> {
        ComposeService::new(self)
    }
}

/// `<data_dir>/eeeemail.sqlite`.
pub fn db_path(config: &AppConfig) -> PathBuf {
    config.data_dir().join("eeeemail.sqlite")
}

/// Attachment cache root: `<data_dir>/attachments/<account_id>`.
pub fn attachment_dir(config: &AppConfig, account_id: &str) -> PathBuf {
    config.data_dir().join("attachments").join(account_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with(email: &str) -> AppConfig {
        let json = serde_json::json!({
            "accounts": [{
                "email": email,
                "auth": { "type": "password", "password": "secret" }
            }]
        });
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn imports_accounts_idempotently() {
        let app = App::with_database(
            config_with("a@example.com"),
            Database::open_in_memory().unwrap(),
        )
        .unwrap();
        app.import_accounts().unwrap();
        app.import_accounts().unwrap();
        assert_eq!(app.accounts().unwrap().len(), 1);
    }

    #[test]
    fn resolves_single_account_without_a_selector() {
        let app = App::with_database(
            config_with("a@example.com"),
            Database::open_in_memory().unwrap(),
        )
        .unwrap();
        assert_eq!(app.resolve_account(None).unwrap().email, "a@example.com");
    }

    #[test]
    fn unknown_selector_lists_what_is_available() {
        let app = App::with_database(
            config_with("a@example.com"),
            Database::open_in_memory().unwrap(),
        )
        .unwrap();
        let err = app.resolve_account(Some("nope")).unwrap_err();
        assert_eq!(err.code, MailErrorCode::InvalidConfig);
        assert!(err.message.contains("a@example.com"));
    }

    #[test]
    fn removed_accounts_disappear_from_the_database() {
        let mut cfg = config_with("a@example.com");
        let app = App::with_database(cfg.clone(), Database::open_in_memory().unwrap()).unwrap();
        assert_eq!(app.accounts().unwrap().len(), 1);

        cfg.accounts.clear();
        let app = App::with_database(cfg, Database::open_in_memory().unwrap()).unwrap();
        app.import_accounts().unwrap();
        assert_eq!(app.accounts().unwrap().len(), 0);
    }
}
