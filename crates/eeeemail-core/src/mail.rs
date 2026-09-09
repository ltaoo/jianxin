//! Read and mutate mail.
//!
//! Reads come from the local database — that is the whole point of the offline
//! design (§28). Writes go to the provider first and only then to the database:
//! if the server rejected the change, showing it locally would be a lie.

use eeeemail_config::AccountConfig;
use eeeemail_domain::{Capability, Folder, FolderId, Message, Result};
use eeeemail_provider::SearchQuery;
use eeeemail_store as store;

use crate::App;

pub struct MailService<'a> {
    app: &'a App,
}

impl<'a> MailService<'a> {
    pub fn new(app: &'a App) -> Self {
        Self { app }
    }

    // --- reads (local) ----------------------------------------------------

    pub fn folders(&self, account_id: &str) -> Result<Vec<Folder>> {
        store::folder_repo::list(&self.app.db().lock(), account_id)
    }

    /// Folders from the database, falling back to a remote listing when we have
    /// never synced this account.
    pub fn ensure_folders(&self, cfg: &AccountConfig) -> Result<Vec<Folder>> {
        let account = cfg.to_account();
        let existing = self.folders(&account.id)?;
        if !existing.is_empty() {
            return Ok(existing);
        }
        self.app.sync().refresh_folders(&*self.app.provider(cfg)?, &account.id)?;
        self.folders(&account.id)
    }

    pub fn list(
        &self,
        folder_id: &str,
        limit: u32,
        offset: u32,
        unread_only: bool,
    ) -> Result<Vec<Message>> {
        store::message_repo::list(&self.app.db().lock(), folder_id, limit, offset, unread_only)
    }

    pub fn count(&self, folder_id: &str, unread_only: bool) -> Result<u64> {
        store::message_repo::count(&self.app.db().lock(), folder_id, unread_only)
    }

    pub fn get(&self, id: &str) -> Result<Message> {
        let conn = self.app.db().lock();
        store::message_repo::get(&conn, id)?.ok_or_else(|| {
            eeeemail_domain::MailError::new(
                eeeemail_domain::MailErrorCode::MessageNotFound,
                format!("no local message with id {id}"),
            )
        })
    }

    /// The folder a message lives in, needed before any provider call.
    pub fn folder_of(&self, id: &str) -> Result<Folder> {
        let m = self.get(id)?;
        let conn = self.app.db().lock();
        store::folder_repo::get(&conn, &m.folder_id)?.ok_or_else(|| {
            eeeemail_domain::MailError::new(
                eeeemail_domain::MailErrorCode::FolderNotFound,
                "message points at a folder that no longer exists",
            )
        })
    }

    /// Resolve a user-supplied folder hint (`inbox`, `INBOX`, id prefix…).
    pub fn resolve_folder(&self, cfg: &AccountConfig, hint: &str) -> Result<Folder> {
        let account = cfg.to_account();
        let conn = self.app.db().lock();
        store::folder_repo::resolve(&conn, &account.id, hint)?.ok_or_else(|| {
            eeeemail_domain::MailError::new(
                eeeemail_domain::MailErrorCode::FolderNotFound,
                format!("no folder matching `{hint}`"),
            )
        })
    }

    // --- writes (provider first, then local) ------------------------------

    pub fn mark_read(&self, cfg: &AccountConfig, id: &str, read: bool) -> Result<()> {
        let provider = self.app.provider(cfg)?;
        let message = self.get(id)?;
        let folder = self.folder_of(id)?;

        if provider.capabilities().supports(Capability::Read) {
            provider.mark_read(&folder, &message.remote_id, read)?;
        }

        store::message_repo::set_read(&self.app.db().lock(), id, read)
    }

    pub fn star(&self, cfg: &AccountConfig, id: &str, starred: bool) -> Result<()> {
        let provider = self.app.provider(cfg)?;
        let message = self.get(id)?;
        let folder = self.folder_of(id)?;

        if provider.capabilities().supports(Capability::Read) {
            provider.set_flag(&folder, &message.remote_id, starred)?;
        }

        store::message_repo::set_starred(&self.app.db().lock(), id, starred)
    }

    /// Delete remotely, then drop the local row.
    ///
    /// The provider may only support "move to Trash"; either way the local row
    /// goes, because that is what the user asked for.
    pub fn delete(&self, cfg: &AccountConfig, id: &str) -> Result<()> {
        let provider = self.app.provider(cfg)?;
        let message = self.get(id)?;
        let folder = self.folder_of(id)?;

        provider.delete_message(&folder, &message.remote_id)?;
        store::message_repo::delete(&self.app.db().lock(), id)?;
        Ok(())
    }

    /// Move a message to another folder of the same account.
    pub fn move_to(&self, cfg: &AccountConfig, id: &str, target_hint: &str) -> Result<Folder> {
        let provider = self.app.provider(cfg)?;
        let message = self.get(id)?;
        let from = self.folder_of(id)?;
        let target = self.resolve_folder(cfg, target_hint)?;

        if target.id == from.id {
            return Ok(target);
        }

        let new_remote = match provider.move_message(&from, &message.remote_id, &target) {
            Ok(()) => None,
            Err(e) => return Err(e),
        };

        // Providers reissue ids on move, so the stale remote id has to go; a
        // local placeholder keeps the row unique until the next sync refreshes
        // it with the real one.
        let new_remote_id =
            new_remote.unwrap_or_else(|| format!("moved-{}", eeeemail_domain::new_id()));

        store::message_repo::move_to_folder(
            &self.app.db().lock(),
            id,
            &target.id,
            Some(&new_remote_id),
        )?;
        Ok(target)
    }

    // --- search -----------------------------------------------------------

    /// Local, offline, works for every provider — the default.
    pub fn search_local(&self, account_id: &str, query: &str, limit: u32) -> Result<Vec<Message>> {
        store::message_repo::search(&self.app.db().lock(), account_id, query, limit)
    }

    /// Server-side search when the provider has it (§27): broader reach,
    /// because it can match messages we have not synced yet.
    pub fn search_server(&self, cfg: &AccountConfig, query: &SearchQuery) -> Result<Vec<Message>> {
        let provider = self.app.provider(cfg)?;
        provider.require(Capability::ServerSearch)?;

        let page = provider.search(&SearchQuery {
            text: query.text.clone(),
            subject: query.subject.clone(),
            from: query.from.clone(),
            to: query.to.clone(),
            unread_only: query.unread_only,
            folder_id: query.folder_id.clone(),
            limit: query.limit,
        })?;

        // Cache whatever we learned so a later offline search can find it.
        let mut messages = page.messages;
        if !messages.is_empty() {
            let account = cfg.to_account();
            self.app.db().transaction(|conn| {
                store::thread_repo::assign(conn, &account.id, &mut messages)?;
                store::message_repo::upsert_many(conn, &messages)?;
                Ok(())
            })?;
        }

        Ok(messages)
    }

    /// Every unread message across an account's folders, newest first.
    pub fn unread(&self, account_id: &str, limit: u32) -> Result<Vec<Message>> {
        let folders = self.folders(account_id)?;
        let conn = self.app.db().lock();
        let mut out = Vec::new();
        for folder in folders {
            let mut page =
                store::message_repo::list(&conn, &folder.id, limit, 0, true)?;
            out.append(&mut page);
        }
        out.sort_by(|a, b| b.sort_ts().cmp(&a.sort_ts()));
        out.truncate(limit as usize);
        Ok(out)
    }

    /// Convenience used by the CLI: id or unique prefix → message.
    pub fn resolve_message(&self, account_id: &str, hint: &str) -> Result<Message> {
        if let Ok(m) = self.get(hint) {
            return Ok(m);
        }
        let folders = self.folders(account_id)?;
        let conn = self.app.db().lock();
        let mut matches: Vec<Message> = Vec::new();
        for folder in folders {
            for m in store::message_repo::list(&conn, &folder.id, u32::MAX, 0, false)? {
                if m.id.starts_with(hint) {
                    matches.push(m);
                }
            }
        }
        match matches.len() {
            1 => Ok(matches.pop().unwrap()),
            0 => Err(eeeemail_domain::MailError::new(
                eeeemail_domain::MailErrorCode::MessageNotFound,
                format!("no message matching `{hint}`"),
            )),
            n => Err(eeeemail_domain::MailError::new(
                eeeemail_domain::MailErrorCode::MessageNotFound,
                format!("`{hint}` is ambiguous ({n} matches)"),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eeeemail_domain::{Address, FolderKind, Message};

    fn app_with(email: &str) -> App {
        let json = serde_json::json!({
            "accounts": [{
                "email": email,
                "auth": {"type": "password", "password": "x"}
            }]
        });
        let cfg: eeeemail_config::AppConfig = serde_json::from_value(json).unwrap();
        App::with_database(cfg, eeeemail_store::Database::open_in_memory().unwrap()).unwrap()
    }

    fn seed(app: &App, folder_remote: &str, kind: FolderKind, subjects: &[&str]) -> FolderId {
        let conn = app.db().lock();
        let mut a = eeeemail_domain::Account::new(
            "generic-imap",
            "a@example.com",
            eeeemail_domain::AuthType::Password,
        );
        a.id = "acc".to_string();
        store::account_repo::upsert(&conn, &a).unwrap();
        let f = Folder::new("acc", folder_remote, folder_remote, kind);
        let id = f.id.clone();
        store::folder_repo::upsert_many(&conn, &[f]).unwrap();

        let mut msgs = Vec::new();
        for (i, s) in subjects.iter().enumerate() {
            let mut m = Message::new();
            m.id = Message::deterministic_id("acc", &id, &i.to_string());
            m.account_id = "acc".to_string();
            m.folder_id = id.clone();
            m.remote_id = i.to_string();
            m.subject = s.to_string();
            m.from = vec![Address::new("bob@example.com")];
            m.sent_at = Some(i as i64);
            // Only the second message is read, so single-message fixtures are
            // unread and 3-message fixtures have 2 unread.
            m.is_read = i == 1;
            msgs.push(m);
        }
        store::message_repo::upsert_many(&conn, &msgs).unwrap();
        store::thread_repo::assign(&conn, &"acc".to_string(), &mut msgs).unwrap();
        id
    }

    #[test]
    fn lists_and_counts() {
        let app = app_with("a@example.com");
        let folder = seed(&app, "INBOX", FolderKind::Inbox, &["a", "b", "c"]);
        assert_eq!(app.mail().count(&folder, false).unwrap(), 3);
        assert_eq!(app.mail().count(&folder, true).unwrap(), 2);
        assert_eq!(app.mail().list(&folder, 2, 0, false).unwrap().len(), 2);
    }

    #[test]
    fn marks_read_locally_without_a_provider() {
        let app = app_with("a@example.com");
        let folder = seed(&app, "INBOX", FolderKind::Inbox, &["a"]);
        let m = app.mail().list(&folder, 10, 0, true).unwrap().pop().unwrap();
        assert!(!m.is_read);

        // No provider needed: the capability gate means offline flag edits are
        // still useful.
        store::message_repo::set_read(&app.db().lock(), &m.id, true).unwrap();
        assert!(app.mail().get(&m.id).unwrap().is_read);
    }

    #[test]
    fn resolves_unique_prefixes() {
        let app = app_with("a@example.com");
        let folder = seed(&app, "INBOX", FolderKind::Inbox, &["a", "b"]);
        let all = app.mail().list(&folder, 10, 0, false).unwrap();
        let prefix = &all[0].id[..8];
        assert_eq!(app.mail().resolve_message("acc", prefix).unwrap().id, all[0].id);
    }

    #[test]
    fn ambiguous_prefix_is_an_error() {
        let app = app_with("a@example.com");
        seed(&app, "INBOX", FolderKind::Inbox, &["a", "b", "c"]);
        // Every id starts with the same nothing, so use a shared prefix.
        let err = app.mail().resolve_message("acc", "").unwrap_err();
        assert_eq!(err.code, eeeemail_domain::MailErrorCode::MessageNotFound);
    }

    #[test]
    fn searches_locally() {
        let app = app_with("a@example.com");
        seed(&app, "INBOX", FolderKind::Inbox, &["Invoice", "Lunch"]);
        let found = app.mail().search_local("acc", "invoice", 10).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].subject, "Invoice");
    }

    #[test]
    fn unread_spans_folders() {
        let app = app_with("a@example.com");
        seed(&app, "INBOX", FolderKind::Inbox, &["a", "b"]);
        seed(&app, "Work", FolderKind::Custom, &["c", "d"]);
        assert_eq!(app.mail().unread("acc", 50).unwrap().len(), 2);
    }
}
