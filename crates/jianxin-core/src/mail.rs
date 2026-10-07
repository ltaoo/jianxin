//! Read and mutate mail.
//!
//! Reads come from the local database — that is the whole point of the offline
//! design (§28). Writes go to the provider first and only then to the database:
//! if the server rejected the change, showing it locally would be a lie.

use jianxin_config::AccountConfig;
use jianxin_domain::{Capability, Folder, Message, Result, Thread};
use jianxin_provider::SearchQuery;
use jianxin_store as store;

use crate::App;

pub struct MailService<'a> {
    app: &'a App,
}

/// A thread plus its messages, oldest first.
#[derive(Debug, Clone)]
pub struct ThreadDetail {
    pub thread: Thread,
    pub messages: Vec<Message>,
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
        self.app
            .sync()
            .refresh_folders(&*self.app.provider(cfg)?, &account.id)?;
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

    /// Every account's mail in one list, one row per real message (folder
    /// copies collapsed) — the TUI's "All Email" view.
    pub fn list_all(&self, limit: u32, offset: u32, unread_only: bool) -> Result<Vec<Message>> {
        store::message_repo::list_all(&self.app.db().lock(), limit, offset, unread_only)
    }

    /// Unread count behind the "All Email" row of the folder pane.
    pub fn count_all_unread(&self) -> Result<u64> {
        store::message_repo::count_all_unread(&self.app.db().lock())
    }

    pub fn count(&self, folder_id: &str, unread_only: bool) -> Result<u64> {
        store::message_repo::count(&self.app.db().lock(), folder_id, unread_only)
    }

    pub fn get(&self, id: &str) -> Result<Message> {
        let conn = self.app.db().lock();
        store::message_repo::get(&conn, id)?.ok_or_else(|| {
            jianxin_domain::MailError::new(
                jianxin_domain::MailErrorCode::MessageNotFound,
                format!("no local message with id {id}"),
            )
        })
    }

    /// The folder a message lives in, needed before any provider call.
    pub fn folder_of(&self, id: &str) -> Result<Folder> {
        let m = self.get(id)?;
        let conn = self.app.db().lock();
        store::folder_repo::get(&conn, &m.folder_id)?.ok_or_else(|| {
            jianxin_domain::MailError::new(
                jianxin_domain::MailErrorCode::FolderNotFound,
                "message points at a folder that no longer exists",
            )
        })
    }

    /// Resolve a user-supplied folder hint (`inbox`, `INBOX`, id prefix…).
    pub fn resolve_folder(&self, cfg: &AccountConfig, hint: &str) -> Result<Folder> {
        let account = cfg.to_account();
        let conn = self.app.db().lock();
        store::folder_repo::resolve(&conn, &account.id, hint)?.ok_or_else(|| {
            jianxin_domain::MailError::new(
                jianxin_domain::MailErrorCode::FolderNotFound,
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

        self.app.db().transaction(|conn| {
            store::message_repo::set_read(conn, id, read)?;
            let unread = store::message_repo::count(conn, &message.folder_id, true)?;
            let total = store::message_repo::count(conn, &message.folder_id, false)?;
            store::folder_repo::set_counts(
                conn,
                &message.folder_id,
                unread.min(u32::MAX as u64) as u32,
                total.min(u32::MAX as u64) as u32,
            )?;
            if let Some(thread_id) = &message.thread_id {
                store::thread_repo::recompute(conn, thread_id)?;
            }
            Ok(())
        })
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
            new_remote.unwrap_or_else(|| format!("moved-{}", jianxin_domain::new_id()));

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

    /// Local search spanning every configured account.
    pub fn search_all_local(&self, query: &str, limit: u32) -> Result<Vec<Message>> {
        store::message_repo::search_all(&self.app.db().lock(), query, limit)
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
            let mut page = store::message_repo::list(&conn, &folder.id, limit, 0, true)?;
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
            0 => Err(jianxin_domain::MailError::new(
                jianxin_domain::MailErrorCode::MessageNotFound,
                format!("no message matching `{hint}`"),
            )),
            n => Err(jianxin_domain::MailError::new(
                jianxin_domain::MailErrorCode::MessageNotFound,
                format!("`{hint}` is ambiguous ({n} matches)"),
            )),
        }
    }

    /// Locally grouped conversations, most recent activity first.
    pub fn threads(
        &self,
        account_id: &str,
        limit: u32,
        offset: u32,
        unread_only: bool,
    ) -> Result<Vec<Thread>> {
        store::thread_repo::list(
            &self.app.db().lock(),
            account_id,
            limit,
            offset,
            unread_only,
        )
    }

    /// Conversations of every account merged, most recent activity first.
    pub fn threads_folder(&self, folder_id: &str, limit: u32) -> Result<Vec<Thread>> {
        store::thread_repo::list_folder(&self.app.db().lock(), folder_id, limit)
    }

    /// Conversations of every account merged, most recent activity first.
    pub fn threads_all(&self, limit: u32, offset: u32, unread_only: bool) -> Result<Vec<Thread>> {
        store::thread_repo::list_all(&self.app.db().lock(), limit, offset, unread_only)
    }

    /// Wipe and rebuild every local thread of an account from what is stored,
    /// returning the resulting thread count. Useful after grouping rules
    /// improve: new mail picks them up, old mail needs this.
    pub fn rebuild_threads(&self, account_id: &str) -> Result<usize> {
        let account_id = account_id.to_string();
        self.app
            .db()
            .transaction(|conn| store::thread_repo::rebuild(conn, &account_id))
    }

    /// Convenience used by the CLI: thread id or unique prefix → thread. A
    /// message id/prefix also works and resolves to that message's thread.
    pub fn resolve_thread(&self, account_id: &str, hint: &str) -> Result<Thread> {
        let mut prefix_matches: Vec<Thread> = {
            let conn = self.app.db().lock();
            if let Some(thread) = store::thread_repo::get(&conn, hint)? {
                if thread.account_id == account_id {
                    return Ok(thread);
                }
            }
            store::thread_repo::list(&conn, account_id, u32::MAX, 0, false)?
                .into_iter()
                .filter(|t| !hint.is_empty() && t.id.starts_with(hint))
                .collect()
        };
        match prefix_matches.len() {
            1 => return Ok(prefix_matches.pop().unwrap()),
            0 => {}
            n => {
                return Err(jianxin_domain::MailError::new(
                    jianxin_domain::MailErrorCode::MessageNotFound,
                    format!("`{hint}` is ambiguous ({n} threads match)"),
                ))
            }
        }

        // Fall back to the thread of whatever message the hint points at.
        let message = self.resolve_message(account_id, hint)?;
        let thread_id = message.thread_id.clone().ok_or_else(|| {
            jianxin_domain::MailError::new(
                jianxin_domain::MailErrorCode::MessageNotFound,
                format!("message `{hint}` belongs to no thread"),
            )
        })?;
        Ok(self.thread(account_id, &thread_id)?.thread)
    }

    /// Load one locally grouped conversation, oldest message first.
    pub fn thread(&self, account_id: &str, thread_id: &str) -> Result<ThreadDetail> {
        let conn = self.app.db().lock();
        let thread = store::thread_repo::get(&conn, thread_id)?
            .filter(|thread| thread.account_id == account_id)
            .ok_or_else(|| {
                jianxin_domain::MailError::new(
                    jianxin_domain::MailErrorCode::MessageNotFound,
                    format!("no local thread with id {thread_id}"),
                )
            })?;
        let mut messages = Vec::new();
        for id in store::thread_repo::message_ids(&conn, thread_id)? {
            let message = store::message_repo::get(&conn, &id)?.ok_or_else(|| {
                jianxin_domain::MailError::new(
                    jianxin_domain::MailErrorCode::MessageNotFound,
                    format!("thread {thread_id} points at missing message {id}"),
                )
            })?;
            messages.push(message);
        }
        Ok(ThreadDetail { thread, messages })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jianxin_domain::{Address, FolderId, FolderKind, Message};

    fn app_with(email: &str) -> App {
        let json = serde_json::json!({
            "accounts": [{
                "email": email,
                "auth": {"type": "password", "password": "x"}
            }]
        });
        let cfg: jianxin_config::AppConfig = serde_json::from_value(json).unwrap();
        App::with_database(cfg, jianxin_store::Database::open_in_memory().unwrap()).unwrap()
    }

    fn seed(app: &App, folder_remote: &str, kind: FolderKind, subjects: &[&str]) -> FolderId {
        let conn = app.db().lock();
        let mut a = jianxin_domain::Account::new(
            jianxin_domain::ProviderId::GenericImap,
            "a@example.com",
            jianxin_domain::AuthType::Password,
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
    fn read_flag_sync_updates_counters_only_after_remote_success() {
        use jianxin_domain::{
            ComposeMessage, MailError, MailErrorCode, MessagePage, ProviderCapabilities,
            ProviderId, SendResult,
        };
        use jianxin_provider::{ListMessagesOptions, MailProvider};
        struct FlagProvider {
            fail: bool,
        }
        impl MailProvider for FlagProvider {
            fn id(&self) -> ProviderId {
                ProviderId::GenericImap
            }
            fn account_id(&self) -> &str {
                "acc"
            }
            fn account_email(&self) -> &str {
                "a@example.com"
            }
            fn capabilities(&self) -> ProviderCapabilities {
                ProviderCapabilities::imap()
            }
            fn list_folders(&self) -> Result<Vec<Folder>> {
                unreachable!()
            }
            fn list_messages(&self, _: &Folder, _: ListMessagesOptions) -> Result<MessagePage> {
                unreachable!()
            }
            fn get_message(&self, _: &Folder, _: &str) -> Result<Message> {
                unreachable!()
            }
            fn send_message(&self, _: &ComposeMessage) -> Result<SendResult> {
                unreachable!()
            }
            fn mark_read(&self, _: &Folder, _: &str, _: bool) -> Result<()> {
                if self.fail {
                    Err(MailError::new(
                        MailErrorCode::NetworkUnavailable,
                        "test remote failure",
                    ))
                } else {
                    Ok(())
                }
            }
        }
        let app = app_with("a@example.com");
        let folder_id = seed(&app, "INBOX", FolderKind::Inbox, &["thread"]);
        let message = app.mail().list(&folder_id, 1, 0, false).unwrap().remove(0);
        let thread_id = message.thread_id.as_ref().unwrap();
        {
            let conn = app.db().lock();
            store::folder_repo::set_counts(&conn, &folder_id, 1, 1).unwrap();
            store::thread_repo::recompute(&conn, thread_id).unwrap();
        }
        let cfg = app.resolve_account(None).unwrap();
        let cfg_id = cfg.to_account().id;
        app.providers.write().unwrap().insert(
            cfg_id.clone(),
            std::sync::Arc::new(FlagProvider { fail: true }),
        );
        assert!(app.mail().mark_read(&cfg, &message.id, true).is_err());
        assert!(!app.mail().get(&message.id).unwrap().is_read);
        assert_eq!(
            app.mail()
                .thread("acc", thread_id)
                .unwrap()
                .thread
                .unread_count,
            1
        );
        app.providers
            .write()
            .unwrap()
            .insert(cfg_id, std::sync::Arc::new(FlagProvider { fail: false }));
        app.mail().mark_read(&cfg, &message.id, true).unwrap();
        assert!(app.mail().get(&message.id).unwrap().is_read);
        assert_eq!(
            app.mail()
                .thread("acc", thread_id)
                .unwrap()
                .thread
                .unread_count,
            0
        );
        assert_eq!(
            store::folder_repo::get(&app.db().lock(), &folder_id)
                .unwrap()
                .unwrap()
                .unread_count,
            0
        );
        app.mail().mark_read(&cfg, &message.id, false).unwrap();
        assert_eq!(
            app.mail()
                .thread("acc", thread_id)
                .unwrap()
                .thread
                .unread_count,
            1
        );
        assert_eq!(
            store::folder_repo::get(&app.db().lock(), &folder_id)
                .unwrap()
                .unwrap()
                .unread_count,
            1
        );
    }

    #[test]
    fn marks_read_locally_without_a_provider() {
        let app = app_with("a@example.com");
        let folder = seed(&app, "INBOX", FolderKind::Inbox, &["a"]);
        let m = app
            .mail()
            .list(&folder, 10, 0, true)
            .unwrap()
            .pop()
            .unwrap();
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
        assert_eq!(
            app.mail().resolve_message("acc", prefix).unwrap().id,
            all[0].id
        );
    }

    #[test]
    fn ambiguous_prefix_is_an_error() {
        let app = app_with("a@example.com");
        seed(&app, "INBOX", FolderKind::Inbox, &["a", "b", "c"]);
        // Every id starts with the same nothing, so use a shared prefix.
        let err = app.mail().resolve_message("acc", "").unwrap_err();
        assert_eq!(err.code, jianxin_domain::MailErrorCode::MessageNotFound);
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
