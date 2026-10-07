//! The sync engine.
//!
//! ```text
//! Initial Sync -> Local Database -> Incremental Sync -> Local Database
//! ```
//!
//! Two properties matter:
//!
//! * **The network call never happens while the database lock is held.** The
//!   delta is fetched first, then applied in one transaction, so a slow IMAP
//!   server cannot freeze the UI's reads.
//! * **A failed folder does not abort the account.** One folder timing out
//!   leaves the others synced and the error is reported per folder.

use std::collections::HashSet;

use jianxin_domain::{AccountId, Folder, FolderId, MailError, Result, SyncCursor, SyncResult};
use jianxin_provider::{MailProvider, SyncDelta};
use jianxin_store as store;

use crate::App;

/// Outcome of syncing a whole account.
#[derive(Debug, Default)]
pub struct SyncReport {
    pub account_id: AccountId,
    pub folders: Vec<SyncResult>,
    pub failures: Vec<FolderFailure>,
    /// True when the remote folder list changed (added or removed folders).
    pub folders_changed: bool,
}

impl SyncReport {
    pub fn new(account_id: &str) -> Self {
        Self {
            account_id: account_id.to_string(),
            ..Default::default()
        }
    }

    pub fn total_upserted(&self) -> usize {
        self.folders.iter().map(|f| f.upserted).sum()
    }

    pub fn total_removed(&self) -> usize {
        self.folders.iter().map(|f| f.removed).sum()
    }

    pub fn total_flagged(&self) -> usize {
        self.folders.iter().map(|f| f.flagged).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.folders.iter().all(|f| f.total_touched() == 0) && self.failures.is_empty()
    }
}

/// A folder that could not be synced, kept out of `folders` so the UI can show
/// partial success.
#[derive(Debug)]
pub struct FolderFailure {
    pub folder_id: FolderId,
    pub folder_name: String,
    pub error: MailError,
}

pub struct SyncEngine<'a> {
    app: &'a App,
}

impl<'a> SyncEngine<'a> {
    pub fn new(app: &'a App) -> Self {
        Self { app }
    }

    /// Refresh the folder list, then sync every syncable folder.
    ///
    /// `full` discards the stored cursors and re-pulls from scratch; use it for
    /// the first sync or after a UIDVALIDITY change.
    pub fn sync_account(
        &self,
        provider: &dyn MailProvider,
        account_id: &str,
        full: bool,
    ) -> Result<SyncReport> {
        let mut report = SyncReport::new(account_id);

        match self.refresh_folders(provider, account_id) {
            Ok(changed) => report.folders_changed = changed,
            Err(e) => {
                report.failures.push(FolderFailure {
                    folder_id: String::new(),
                    folder_name: "(folder list)".to_string(),
                    error: e,
                });
                return Ok(report);
            }
        }

        let folders = {
            let conn = self.app.db().lock();
            store::folder_repo::list(&conn, account_id)?
        };

        for folder in folders.into_iter().filter(|f| self.should_sync(f)) {
            match self.sync_folder(provider, account_id, &folder, full) {
                Ok(result) => report.folders.push(result),
                Err(e) => report.failures.push(FolderFailure {
                    folder_id: folder.id.clone(),
                    folder_name: folder.name.clone(),
                    error: e,
                }),
            }
        }

        Ok(report)
    }

    /// Sync exactly one folder.
    pub fn sync_folder(
        &self,
        provider: &dyn MailProvider,
        account_id: &str,
        folder: &Folder,
        full: bool,
    ) -> Result<SyncResult> {
        if full {
            let conn = self.app.db().lock();
            store::sync_repo::clear(&conn, account_id, &folder.id)?;
        }

        let mut total = SyncResult::new(&folder.id);
        total.folder_name = folder.name.clone();

        let mut cursor: Option<SyncCursor> = {
            let conn = self.app.db().lock();
            store::sync_repo::get(&conn, account_id, &folder.id)?
        };

        // Bound the loop: `has_more` comes from a remote server and a hostile
        // or buggy one must not be able to hang us.
        for _ in 0..MAX_PAGES {
            // Network first — no database lock held here.
            let mut delta = match provider.sync(folder, cursor.as_ref()) {
                Ok(d) => d,
                Err(e) => {
                    let conn = self.app.db().lock();
                    store::sync_repo::record_error(&conn, account_id, &folder.id, &e.to_string())?;
                    return Err(e);
                }
            };

            let applied = self.apply(account_id, folder, &mut delta)?;

            total.upserted += applied.upserted;
            total.removed += applied.removed;
            total.flagged += applied.flagged;
            total.has_more = delta.has_more;
            total.cursor = delta.cursor.clone().or(total.cursor);

            cursor = delta.cursor;
            if !delta.has_more || cursor.is_none() {
                break;
            }
        }

        Ok(total)
    }

    /// Write one delta into the database.
    ///
    /// Runs in a single transaction: a crash between "messages inserted" and
    /// "cursor advanced" would otherwise replay the same delta forever.
    fn apply(&self, account_id: &str, folder: &Folder, delta: &mut SyncDelta) -> Result<SyncResult> {
        // Thread assignment mutates the batch; transfer it rather than copying
        // every body and attachment while the provider's delta is still alive.
        let mut new_messages = std::mem::take(&mut delta.new_messages);

        self.app.db().transaction(|conn| {
            let upserted = if new_messages.is_empty() {
                0
            } else {
                store::message_repo::upsert_many(conn, &new_messages)?
            };

            // Threading needs the rows present: a reply resolves its parent by
            // looking up `Message-ID` in the same batch.
            store::thread_repo::assign(conn, &account_id.to_string(), &mut new_messages)?;

            let flagged =
                store::message_repo::apply_flag_updates(conn, &folder.id, &delta.flag_updates)?;

            // Deletions. Providers with a change feed tell us directly; IMAP
            // hands back the ids it can still see and we diff locally.
            let removed = if !delta.removed_remote_ids.is_empty() {
                store::message_repo::remove_by_remote_ids(
                    conn,
                    &folder.id,
                    &delta.removed_remote_ids,
                )?
            } else if !delta.present_remote_ids.is_empty() {
                let present: HashSet<&str> = delta
                    .present_remote_ids
                    .iter()
                    .map(|s| s.as_str())
                    .collect();
                let stale: Vec<String> =
                    store::message_repo::remote_ids_in_folder(conn, &folder.id)?
                        .into_iter()
                        .filter(|id| !present.contains(id.as_str()))
                        .collect();
                store::message_repo::remove_by_remote_ids(conn, &folder.id, &stale)?
            } else {
                0
            };

            store::thread_repo::recompute_for(conn, &new_messages)?;

            if let Some(cursor) = &delta.cursor {
                store::sync_repo::set(conn, cursor)?;
            }

            let total = store::message_repo::count(conn, &folder.id, false)?;
            let unread = store::message_repo::count(conn, &folder.id, true)?;
            store::folder_repo::set_counts(conn, &folder.id, unread as u32, total as u32)?;

            Ok(SyncResult {
                folder_id: folder.id.clone(),
                folder_name: folder.name.clone(),
                upserted,
                removed,
                flagged,
                cursor: delta.cursor.clone(),
                has_more: delta.has_more,
            })
        })
    }

    /// Re-list folders remotely and reconcile with the local rows.
    ///
    /// Returns whether the set of folders changed.
    pub fn refresh_folders(&self, provider: &dyn MailProvider, account_id: &str) -> Result<bool> {
        let remote = provider.list_folders()?;

        self.app.db().transaction(|conn| {
            let before: HashSet<String> = store::folder_repo::list(conn, account_id)?
                .into_iter()
                .map(|f| f.remote_id)
                .collect();

            store::folder_repo::upsert_many(conn, &remote)?;

            let keep: Vec<String> = remote.iter().map(|f| f.remote_id.clone()).collect();
            store::folder_repo::delete_missing(conn, account_id, &keep)?;

            let after: HashSet<String> = store::folder_repo::list(conn, account_id)?
                .into_iter()
                .map(|f| f.remote_id)
                .collect();

            Ok(before != after)
        })
    }

    /// Honour `sync.skip_folders` (role names) as well as the folder's own role.
    fn should_sync(&self, folder: &Folder) -> bool {
        if !folder.kind.sync_by_default() {
            return false;
        }
        let skipped: Vec<String> = self
            .app
            .sync_config()
            .skip_folders
            .iter()
            .map(|s| s.trim().to_ascii_lowercase())
            .collect();
        !skipped.contains(&folder.kind.as_str().to_string())
            && !skipped
                .iter()
                .any(|s| folder.name.to_ascii_lowercase().contains(s))
    }
}

/// Safety valve for the `has_more` paging loop.
const MAX_PAGES: usize = 200;

#[cfg(test)]
mod tests {
    use super::*;
    use jianxin_domain::MailErrorCode;
    use jianxin_domain::{
        Address, Capability, ComposeMessage, FolderKind, Message, MessagePage,
        ProviderCapabilities, SendResult,
    };
    use jianxin_provider::{ListMessagesOptions, SearchQuery};

    /// A provider that never touches the network: a fixed folder list and a
    /// page of messages that tests can vary between calls.
    struct FakeProvider {
        account_id: String,
        folders: Vec<Folder>,
        messages: Vec<Message>,
        removed: Vec<String>,
    }

    impl MailProvider for FakeProvider {
        fn id(&self) -> jianxin_domain::ProviderId {
            jianxin_domain::ProviderId::GenericImap
        }
        fn account_id(&self) -> &str {
            &self.account_id
        }
        fn account_email(&self) -> &str {
            "a@example.com"
        }
        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities::imap()
        }
        fn list_folders(&self) -> Result<Vec<Folder>> {
            Ok(self.folders.clone())
        }
        fn list_messages(
            &self,
            folder: &Folder,
            _opts: ListMessagesOptions,
        ) -> Result<MessagePage> {
            Ok(MessagePage::new(
                self.messages
                    .iter()
                    .filter(|m| m.folder_id == folder.id)
                    .cloned()
                    .collect(),
            ))
        }
        fn get_message(&self, folder: &Folder, remote_id: &str) -> Result<Message> {
            self.messages
                .iter()
                .find(|m| m.folder_id == folder.id && m.remote_id == remote_id)
                .cloned()
                .ok_or_else(|| MailError::new(MailErrorCode::MessageNotFound, "gone"))
        }
        fn send_message(&self, _m: &ComposeMessage) -> Result<SendResult> {
            Err(MailError::unsupported("send"))
        }
        fn sync(&self, folder: &Folder, _cursor: Option<&SyncCursor>) -> Result<SyncDelta> {
            let mut delta = SyncDelta::new(&folder.id, &folder.name);
            delta.new_messages = self
                .messages
                .iter()
                .filter(|m| m.folder_id == folder.id)
                .cloned()
                .collect();
            delta.removed_remote_ids = self.removed.clone();
            delta.present_remote_ids = self
                .messages
                .iter()
                .filter(|m| m.folder_id == folder.id)
                .map(|m| m.remote_id.clone())
                .collect();
            delta.cursor = Some(SyncCursor {
                provider: jianxin_domain::ProviderId::GenericImap,
                account_id: self.account_id.clone(),
                folder_id: folder.id.clone(),
                cursor_type: "uid".to_string(),
                cursor: "1:1".to_string(),
            });
            Ok(delta)
        }
        fn search(&self, _q: &SearchQuery) -> Result<MessagePage> {
            Ok(MessagePage::new(Vec::new()))
        }
    }

    struct Fixture {
        app: App,
        /// Local account id, derived from provider + email as in production.
        account_id: String,
    }

    fn fixture() -> Fixture {
        let json = serde_json::json!({
            "accounts": [{
                "email": "a@example.com",
                "auth": {"type": "password", "password": "x"}
            }]
        });
        let cfg: jianxin_config::AppConfig = serde_json::from_value(json).unwrap();
        let app =
            App::with_database(cfg, jianxin_store::Database::open_in_memory().unwrap()).unwrap();
        let account_id = app.resolve_account(None).unwrap().to_account().id;
        Fixture { app, account_id }
    }

    impl Fixture {
        fn folder(&self, remote: &str, kind: FolderKind) -> Folder {
            Folder::new(&self.account_id, remote, remote, kind)
        }

        fn message(&self, folder_id: &str, remote: &str) -> Message {
            let mut m = Message::new();
            m.id = Message::deterministic_id(&self.account_id, folder_id, remote);
            m.account_id = self.account_id.clone();
            m.folder_id = folder_id.to_string();
            m.remote_id = remote.to_string();
            m.subject = format!("subject {remote}");
            m.from = vec![Address::new("bob@example.com")];
            m.sent_at = Some(remote.parse::<i64>().unwrap_or(0));
            m
        }

        fn provider(&self, folders: Vec<Folder>, messages: Vec<Message>) -> FakeProvider {
            FakeProvider {
                account_id: self.account_id.clone(),
                folders,
                messages,
                removed: vec![],
            }
        }
    }

    #[test]
    fn syncs_folders_and_messages() {
        let f = fixture();
        let inbox = f.folder("INBOX", FolderKind::Inbox);
        let provider = f.provider(
            vec![inbox.clone()],
            vec![f.message(&inbox.id, "1"), f.message(&inbox.id, "2")],
        );

        let report = f
            .app
            .sync()
            .sync_account(&provider, &f.account_id, true)
            .unwrap();

        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert_eq!(report.total_upserted(), 2);

        let conn = f.app.db().lock();
        assert_eq!(
            store::message_repo::list(&conn, &inbox.id, 50, 0, false)
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            store::folder_repo::list(&conn, &f.account_id)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn second_sync_does_not_duplicate() {
        let f = fixture();
        let inbox = f.folder("INBOX", FolderKind::Inbox);
        let provider = f.provider(vec![inbox.clone()], vec![f.message(&inbox.id, "1")]);

        f.app
            .sync()
            .sync_account(&provider, &f.account_id, true)
            .unwrap();
        let again = f
            .app
            .sync()
            .sync_account(&provider, &f.account_id, false)
            .unwrap();
        assert_eq!(again.total_upserted(), 1);

        let conn = f.app.db().lock();
        assert_eq!(
            store::message_repo::count(&conn, &inbox.id, false).unwrap(),
            1
        );
    }

    #[test]
    fn explicit_removals_are_applied() {
        let f = fixture();
        let inbox = f.folder("INBOX", FolderKind::Inbox);
        f.app
            .sync()
            .sync_account(
                &f.provider(vec![inbox.clone()], vec![f.message(&inbox.id, "1")]),
                &f.account_id,
                true,
            )
            .unwrap();

        // Server no longer has message "1".
        let mut provider = f.provider(vec![inbox.clone()], vec![]);
        provider.removed = vec!["1".to_string()];
        let report = f
            .app
            .sync()
            .sync_account(&provider, &f.account_id, false)
            .unwrap();
        assert_eq!(report.total_removed(), 1);

        let conn = f.app.db().lock();
        assert_eq!(
            store::message_repo::count(&conn, &inbox.id, false).unwrap(),
            0
        );
    }

    #[test]
    fn imap_style_presence_diff_finds_deletions() {
        let f = fixture();
        let inbox = f.folder("INBOX", FolderKind::Inbox);
        f.app
            .sync()
            .sync_account(
                &f.provider(
                    vec![inbox.clone()],
                    vec![f.message(&inbox.id, "1"), f.message(&inbox.id, "2")],
                ),
                &f.account_id,
                true,
            )
            .unwrap();

        // No `removed_remote_ids`, only what is still present — the IMAP case.
        let report = f
            .app
            .sync()
            .sync_account(
                &f.provider(vec![inbox.clone()], vec![f.message(&inbox.id, "2")]),
                &f.account_id,
                false,
            )
            .unwrap();
        assert_eq!(report.total_removed(), 1);
    }

    #[test]
    fn spam_and_trash_are_skipped() {
        let f = fixture();
        let inbox = f.folder("INBOX", FolderKind::Inbox);
        let spam = f.folder("Spam", FolderKind::Spam);
        let provider = f.provider(
            vec![inbox.clone(), spam.clone()],
            vec![f.message(&spam.id, "9")],
        );
        let report = f
            .app
            .sync()
            .sync_account(&provider, &f.account_id, true)
            .unwrap();
        assert_eq!(report.folders.len(), 1);
        assert_eq!(report.folders[0].folder_id, inbox.id);
    }

    #[test]
    fn folder_list_changes_are_detected() {
        let f = fixture();
        let inbox = f.folder("INBOX", FolderKind::Inbox);
        let provider = f.provider(vec![inbox], vec![]);
        assert!(
            f.app
                .sync()
                .sync_account(&provider, &f.account_id, true)
                .unwrap()
                .folders_changed
        );
        assert!(
            !f.app
                .sync()
                .sync_account(&provider, &f.account_id, false)
                .unwrap()
                .folders_changed
        );
    }

    #[test]
    fn thread_ids_are_assigned_during_sync() {
        let f = fixture();
        let inbox = f.folder("INBOX", FolderKind::Inbox);
        let mut a = f.message(&inbox.id, "1");
        a.subject = "Budget".to_string();
        a.message_id_header = Some("<root@x>".to_string());
        let mut b = f.message(&inbox.id, "2");
        b.subject = "Re: Budget".to_string();
        b.in_reply_to = Some("<root@x>".to_string());

        f.app
            .sync()
            .sync_account(
                &f.provider(vec![inbox.clone()], vec![a, b]),
                &f.account_id,
                true,
            )
            .unwrap();

        let conn = f.app.db().lock();
        let stored = store::message_repo::list(&conn, &inbox.id, 50, 0, false).unwrap();
        assert!(stored[0].thread_id.is_some());
        assert_eq!(stored[0].thread_id, stored[1].thread_id);
    }

    #[test]
    fn folder_counts_refresh_after_sync() {
        let f = fixture();
        let inbox = f.folder("INBOX", FolderKind::Inbox);
        let mut m = f.message(&inbox.id, "1");
        m.is_read = false;
        f.app
            .sync()
            .sync_account(
                &f.provider(vec![inbox.clone()], vec![m]),
                &f.account_id,
                true,
            )
            .unwrap();

        let conn = f.app.db().lock();
        let folder = store::folder_repo::get(&conn, &inbox.id).unwrap().unwrap();
        assert_eq!(folder.total_count, 1);
        assert_eq!(folder.unread_count, 1);
    }

    #[test]
    fn a_failing_folder_does_not_abort_the_account() {
        struct Broken;
        impl MailProvider for Broken {
            fn id(&self) -> jianxin_domain::ProviderId {
                jianxin_domain::ProviderId::GenericImap
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
                Err(MailError::new(MailErrorCode::NetworkUnavailable, "offline"))
            }
            fn list_messages(&self, _f: &Folder, _o: ListMessagesOptions) -> Result<MessagePage> {
                unimplemented!()
            }
            fn get_message(&self, _f: &Folder, _r: &str) -> Result<Message> {
                unimplemented!()
            }
            fn send_message(&self, _m: &ComposeMessage) -> Result<SendResult> {
                unimplemented!()
            }
        }

        let f = fixture();
        let report = f
            .app
            .sync()
            .sync_account(&Broken, &f.account_id, true)
            .unwrap();
        assert_eq!(report.failures.len(), 1);
        assert!(report.folders.is_empty());
    }

    #[test]
    fn capability_gate_still_works() {
        // Guards against a refactor that makes `require` a no-op.
        let f = fixture();
        let p = f.provider(vec![], vec![]);
        assert!(p.require(Capability::Read).is_ok());
        assert!(p.require(Capability::Labels).is_err());
    }
}
