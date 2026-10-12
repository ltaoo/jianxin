//! UI state for the terminal client.
//!
//! The state owns no business logic — every action goes through
//! [`jianxin_core`] services. What it *does* own is the one thing the UI
//! cannot do without: which account, folder and message the user is looking at,
//! and how to keep painting while the network is busy.
//!
//! The folder pane shows every configured account at once, headed by an
//! "All Email" pseudo-folder that merges them. Cursor-visible rows drive the
//! "active account" (the one `c` composes as), while message actions always
//! target the account that owns the highlighted message — the merged list can
//! contain both.
//!
//! Network work runs on a worker thread and reports back over a channel, so a
//! slow IMAP server never freezes the render loop.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use jianxin_config::AccountConfig;
use jianxin_core::App;
use jianxin_domain::{Account, Address, ComposeMessage, Folder, Message, Thread};

/// Which pane receives the arrow keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Folders,
    List,
    Preview,
}

/// Which listing the message pane shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListMode {
    /// One row per message.
    Flat,
    /// One row per conversation; Enter drills into one.
    Threads,
}

/// One row of the folder pane. Account headers structure the list but are
/// never selectable; the cursor skips over them.
#[derive(Debug, Clone, PartialEq)]
pub enum FolderRow {
    /// Pseudo-folder at the top: every account's mail merged into one list.
    AllEmail,
    /// Group header for account `account_configs()[i]`.
    Account(usize),
    /// A real folder: (account index, index within that account's folders).
    Folder(usize, usize),
}

/// A conversation opened from the thread list.
#[derive(Debug)]
pub struct OpenThread {
    pub thread: Thread,
    /// Oldest first, matching `MailService::thread`.
    pub messages: Vec<Message>,
    pub msg_idx: usize,
}

impl Focus {
    pub fn next(self) -> Self {
        match self {
            Focus::Folders => Focus::List,
            Focus::List => Focus::Preview,
            Focus::Preview => Focus::Folders,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Focus::Folders => "folders",
            Focus::List => "list",
            Focus::Preview => "message",
        }
    }
}

/// A message from a worker thread.
#[derive(Debug)]
pub enum WorkerMsg {
    BrowserOpened(bool),
    /// Sync finished; the payload is a human summary.
    Synced(String),
    /// Mail went out.
    Sent(String),
    /// Both of the above can fail.
    Failed(String),
    /// An optimistic change (flag toggle, delete) could not be persisted; the
    /// lists are stale and must be reloaded from the store, which rolls the
    /// change back.
    Revert(String),
}

/// The compose form. Fields are plain strings — parsing happens at send time so
/// a half-typed address is not an error until the user asks us to send.
#[derive(Debug, Clone, Default)]
pub struct ComposeForm {
    pub to: String,
    pub subject: String,
    pub body: String,
    /// Threading headers when this form is a reply.
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    /// 0 = To, 1 = Subject, 2 = Body, 3 = Send, 4 = Cancel.
    pub field: usize,
    pub error: Option<String>,
    pub sending: bool,
}

impl ComposeForm {
    pub const FIELD_COUNT: usize = 5;

    /// Cursor column inside the text field, or `None` when a button is active.
    pub fn cursor_col(&self) -> Option<u16> {
        let len = match self.field {
            0 => self.to.chars().count(),
            1 => self.subject.chars().count(),
            2 => self.body.lines().last().unwrap_or("").chars().count(),
            _ => return None,
        };
        Some(len as u16)
    }
}

#[derive(Debug)]
pub enum Overlay {
    None,
    Compose(ComposeForm),
    Search(String),
    Help,
}

impl Overlay {
    fn is_none(&self) -> bool {
        matches!(self, Overlay::None)
    }
}

/// What the folder cursor was on before a rebuild, so it can be put back.
enum CursorKey {
    All,
    Folder(String, String),
}

pub struct State {
    pub app: Arc<App>,

    /// The account `c` composes as: the account of the folder under the
    /// cursor. The All Email row keeps the previous active account.
    pub cfg: AccountConfig,
    pub account: Account,
    pub account_idx: usize,

    /// Folders per configured account, parallel to `app.account_configs()`.
    pub folders_by_account: Vec<Vec<Folder>>,
    /// The flattened pane: All Email, then each account's header and folders.
    pub rows: Vec<FolderRow>,
    pub row_idx: usize,
    /// Deduplicated unread count behind the All Email row.
    pub all_unread: u32,

    pub messages: Vec<Message>,
    pub msg_idx: usize,

    /// Conversation view state; `t` toggles between this and the flat list.
    pub list_mode: ListMode,
    pub threads: Vec<Thread>,
    pub thread_idx: usize,
    pub open_thread: Option<OpenThread>,

    /// Preview scroll offset in lines.
    pub scroll: u16,
    pub focus: Focus,
    /// Status line: text plus whether it is an error.
    pub status: Option<(String, bool)>,
    pub busy: bool,
    pub overlay: Overlay,
    /// Active local search, if any.
    pub query: Option<String>,
    pub quit: bool,
    pub tick: usize,

    tx: Sender<WorkerMsg>,
    rx: Receiver<WorkerMsg>,
    last_refresh: Instant,
    database_version: i64,
    pub account_statuses: Vec<String>,
    pub browser_config: Option<std::path::PathBuf>,
}

#[cfg(test)]
mod refresh_tests {
    use super::*;
    use jianxin_domain::{Folder, FolderKind};
    use jianxin_store::{Database, folder_repo, message_repo, thread_repo};

    #[test]
    fn external_mail_refresh_preserves_open_message() {
        let directory = std::env::temp_dir().join(format!("jianxin-tui-test-{}", jianxin_domain::new_id()));
        let path = directory.join("mail.sqlite");
        let config = serde_json::from_value(serde_json::json!({
            "general": {"data_dir": directory},
            "accounts": [{"email": "test@example.com", "auth": {"type": "password", "password": "test"}}]
        })).unwrap();
        let app = Arc::new(App::with_database(config, Database::open(&path).unwrap()).unwrap());
        let account_id = app.account_configs()[0].to_account().id;
        let folder = Folder::new(&account_id, "INBOX", "Inbox", FolderKind::Inbox);
        folder_repo::upsert_many(&app.db().lock(), &[folder.clone()]).unwrap();
        let mut root = Message::new();
        root.account_id = account_id.clone();
        root.folder_id = folder.id.clone();
        root.remote_id = "1".into();
        root.subject = "Conversation".into();
        root.sent_at = Some(100);
        {
            let conn = app.db().lock();
            message_repo::upsert_many(&conn, &[root.clone()]).unwrap();
            thread_repo::assign(&conn, &account_id, std::slice::from_mut(&mut root)).unwrap();
            thread_repo::recompute_for(&conn, &[root.clone()]).unwrap();
        }
        let mut state = State::new(app.clone()).unwrap();
        assert!(state.list_mode == ListMode::Threads);
        state.open_selected_thread();
        state.last_refresh = Instant::now() - Duration::from_secs(2);
        state.poll();
        let external = Database::open(&path).unwrap();
        let mut reply = root.clone();
        reply.id = jianxin_domain::new_id();
        reply.remote_id = "2".into();
        reply.sent_at = Some(200);
        {
            let conn = external.lock();
            message_repo::upsert_many(&conn, &[reply.clone()]).unwrap();
            thread_repo::recompute_for(&conn, &[reply]).unwrap();
        }
        state.scroll = 3;
        state.last_refresh = Instant::now() - Duration::from_secs(2);
        state.poll();
        assert_eq!(state.threads[0].message_count, 2);
        assert_eq!(state.open_thread.as_ref().unwrap().messages.len(), 2);
        assert_eq!(state.selected_message().unwrap().id, root.id);
        assert_eq!(state.scroll, 3);
        // Status changes must repaint even without another database write.
        jianxin_core::record_account_sync_status(app.config(), &account_id, Some("AUTHENTICATION_FAILED"));
        state.last_refresh = Instant::now() - Duration::from_secs(2);
        state.poll();
        assert!(state.account_statuses[0].contains("登录失败"));
        jianxin_core::record_account_sync_status(app.config(), &account_id, None);
        state.tx.send(WorkerMsg::Synced("ok".into())).unwrap();
        state.poll();
        assert_eq!(state.account_statuses[0], "已同步");
        drop(state);
        drop(app);
        drop(external);
        std::fs::remove_dir_all(directory).unwrap();
    }
}

impl State {
    pub fn new(app: Arc<App>) -> anyhow::Result<Self> {
        let cfg =
            app.account_configs().first().cloned().ok_or_else(|| {
                anyhow::anyhow!("no accounts configured — add one to config.json")
            })?;

        let (tx, rx) = mpsc::channel();
        let mut state = Self {
            account: cfg.to_account(),
            account_idx: 0,
            cfg,
            app,
            folders_by_account: Vec::new(),
            rows: Vec::new(),
            row_idx: 0,
            all_unread: 0,
            messages: Vec::new(),
            msg_idx: 0,
            list_mode: ListMode::Threads,
            threads: Vec::new(),
            thread_idx: 0,
            open_thread: None,
            scroll: 0,
            focus: Focus::List,
            status: None,
            busy: false,
            overlay: Overlay::None,
            query: None,
            quit: false,
            tick: 0,
            tx,
            rx,
            last_refresh: Instant::now(),
            database_version: -1,
            account_statuses: Vec::new(),
            browser_config: None,
        };
        state.reload_folders();
        state.reload_messages();
        state.reload_threads();
        state.refresh_account_statuses();
        Ok(state)
    }

    // --- navigation -------------------------------------------------------

    pub fn open_in_browser(&mut self) {
        let Some(message) = self.selected_message() else {
            self.status = Some(("先按 Enter 展开对话，再选择要在浏览器阅读的邮件".into(), false));
            return;
        };
        let id = message.id.clone();
        let Some(config) = self.browser_config.clone() else {
            self.status = Some(("缺少配置路径，无法打开 Web 阅读页".into(), true));
            return;
        };
        let tx = self.tx.clone();
        self.status = Some(("正在浏览器中打开邮件…".into(), false));
        thread::spawn(move || {
            let result = std::env::current_exe().and_then(|executable| {
                std::process::Command::new(executable)
                    .arg("--config").arg(config).arg("open").arg(id)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null()).status()
            });
            let _ = tx.send(WorkerMsg::BrowserOpened(result.is_ok_and(|status| status.success())));
        });
    }

    pub fn selected_row(&self) -> Option<&FolderRow> {
        self.rows.get(self.row_idx)
    }

    /// The folder under the cursor, with its account index.
    pub fn selected_folder(&self) -> Option<(usize, &Folder)> {
        match self.rows.get(self.row_idx) {
            Some(FolderRow::Folder(ai, fi)) => self
                .folders_by_account
                .get(*ai)
                .and_then(|v| v.get(*fi))
                .map(|f| (*ai, f)),
            _ => None,
        }
    }

    pub fn selected_thread(&self) -> Option<&Thread> {
        self.threads.get(self.thread_idx)
    }

    /// The message under the cursor: from the flat list, or from the
    /// conversation drilled into.
    pub fn selected_message(&self) -> Option<&Message> {
        match self.list_mode {
            ListMode::Flat => self.messages.get(self.msg_idx),
            ListMode::Threads => self
                .open_thread
                .as_ref()
                .and_then(|t| t.messages.get(t.msg_idx)),
        }
    }

    /// The message rows the cursor moves through, mutable, with the cursor.
    fn active_message_list(&mut self) -> Option<(&mut Vec<Message>, &mut usize)> {
        match self.list_mode {
            ListMode::Flat => Some((&mut self.messages, &mut self.msg_idx)),
            ListMode::Threads => self
                .open_thread
                .as_mut()
                .map(|t| (&mut t.messages, &mut t.msg_idx)),
        }
    }

    fn selectable_row(&self, i: usize) -> bool {
        matches!(
            self.rows.get(i),
            Some(FolderRow::AllEmail) | Some(FolderRow::Folder(..))
        )
    }

    /// Nearest selectable row at or after `from`, scanning forward or backward.
    fn nearest_selectable(&self, from: usize, forward: bool) -> Option<usize> {
        let n = self.rows.len();
        if n == 0 {
            return None;
        }
        let from = from.min(n - 1);
        for step in 0..n {
            let i = if forward {
                (from + step) % n
            } else {
                (from + n - step % n) % n
            };
            if self.selectable_row(i) {
                return Some(i);
            }
        }
        None
    }

    pub fn move_folder(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let n = self.rows.len() as isize;
        let i = (self.row_idx as isize + delta).clamp(0, n - 1) as usize;
        // Header rows are skipped: land on the next real row in travel direction.
        if let Some(i) = self.nearest_selectable(i, delta > 0) {
            self.select_row(i);
        }
    }

    fn select_row(&mut self, i: usize) {
        if i == self.row_idx {
            return;
        }
        self.row_idx = i;
        self.sync_active_account();
        self.reload_messages();
        self.open_thread = None;
        if self.list_mode == ListMode::Threads { self.reload_threads(); }
    }

    /// A folder row also decides which account new mail is composed as.
    fn sync_active_account(&mut self) {
        if let Some(FolderRow::Folder(ai, _)) = self.rows.get(self.row_idx) {
            self.set_active_account(*ai);
        }
    }

    fn set_active_account(&mut self, ai: usize) {
        if let Some(cfg) = self.app.account_configs().get(ai).cloned() {
            self.account_idx = ai;
            self.cfg = cfg.clone();
            self.account = self.cfg.to_account();
        }
    }

    pub fn move_message(&mut self, delta: isize) {
        if self.list_mode == ListMode::Threads && self.open_thread.is_none() {
            if self.threads.is_empty() {
                return;
            }
            let n = self.threads.len() as isize;
            self.thread_idx = (self.thread_idx as isize + delta).clamp(0, n - 1) as usize;
            self.scroll = 0;
            return;
        }
        let Some((messages, idx)) = self.active_message_list() else {
            return;
        };
        if messages.is_empty() {
            return;
        }
        let n = messages.len() as isize;
        *idx = (*idx as isize + delta).clamp(0, n - 1) as usize;
        self.scroll = 0;
    }

    pub fn jump(&mut self, to_end: bool) {
        if self.focus == Focus::Folders {
            let edge = if to_end {
                self.rows.len().saturating_sub(1)
            } else {
                0
            };
            if let Some(i) = self.nearest_selectable(edge, !to_end) {
                self.select_row(i);
            }
            return;
        }
        if self.list_mode == ListMode::Threads && self.open_thread.is_none() {
            self.thread_idx = if to_end {
                self.threads.len().saturating_sub(1)
            } else {
                0
            };
        } else if let Some((messages, idx)) = self.active_message_list() {
            *idx = if to_end {
                messages.len().saturating_sub(1)
            } else {
                0
            };
        }
        self.scroll = 0;
    }

    /// `a`/`A`: jump to the previous/next account's first folder row. An
    /// account that has never been synced has no folders, so fall back to its
    /// header row.
    pub fn cycle_account(&mut self, delta: isize) -> bool {
        let n = self.app.account_configs().len();
        if n < 2 {
            return false;
        }
        let target = (self.account_idx as isize + delta).rem_euclid(n as isize) as usize;
        let row = self
            .rows
            .iter()
            .position(|r| matches!(r, FolderRow::Folder(ai, _) if *ai == target))
            .or_else(|| {
                self.rows
                    .iter()
                    .position(|r| matches!(r, FolderRow::Account(ai) if *ai == target))
            });
        if let Some(i) = row {
            self.row_idx = i;
            self.set_active_account(target);
            self.reload_messages();
            if self.list_mode == ListMode::Threads {
                self.reload_threads();
            }
        }
        true
    }

    // --- data -------------------------------------------------------------

    pub fn reload_folders(&mut self) {
        let configs: Vec<AccountConfig> = self.app.account_configs().to_vec();
        let mut folders_by_account = Vec::with_capacity(configs.len());
        let mut first_error = None;
        for cfg in &configs {
            let account = cfg.to_account();
            match self.app.mail().folders(&account.id) {
                Ok(folders) => folders_by_account.push(folders),
                Err(e) => {
                    if first_error.is_none() {
                        first_error = Some(format!("cannot read folders for {}: {e}", cfg.alias()));
                    }
                    folders_by_account.push(Vec::new());
                }
            }
        }
        if let Some(e) = first_error {
            self.status = Some((e, true));
        }
        self.folders_by_account = folders_by_account;
        self.all_unread = self
            .app
            .mail()
            .count_all_unread()
            .map(|n| n.min(u32::MAX as u64) as u32)
            .unwrap_or(0);
        self.rebuild_rows();
    }

    /// Flatten the pane: All Email, then each account's header and folders.
    /// The cursor stays on the same target when the rows are rebuilt.
    fn rebuild_rows(&mut self) {
        let keep = match self.rows.get(self.row_idx) {
            Some(FolderRow::AllEmail) => Some(CursorKey::All),
            Some(FolderRow::Folder(ai, fi)) => self
                .folders_by_account
                .get(*ai)
                .and_then(|v| v.get(*fi))
                .map(|f| CursorKey::Folder(f.account_id.clone(), f.id.clone())),
            _ => None,
        };

        let mut rows = vec![FolderRow::AllEmail];
        for ai in 0..self.folders_by_account.len() {
            rows.push(FolderRow::Account(ai));
            for fi in 0..self.folders_by_account[ai].len() {
                rows.push(FolderRow::Folder(ai, fi));
            }
        }
        self.rows = rows;

        let target = match &keep {
            Some(CursorKey::All) => self
                .rows
                .iter()
                .position(|r| *r == FolderRow::AllEmail),
            Some(CursorKey::Folder(account_id, folder_id)) => self.rows.iter().position(|r| {
                match r {
                    FolderRow::Folder(ai, fi) => self
                        .folders_by_account
                        .get(*ai)
                        .and_then(|v| v.get(*fi))
                        .is_some_and(|f| &f.account_id == account_id && &f.id == folder_id),
                    _ => false,
                }
            }),
            None => None,
        };
        self.row_idx = target
            .or_else(|| {
                self.nearest_selectable(self.row_idx, true)
            })
            .unwrap_or(0);
        self.sync_active_account();
    }

    pub fn reload_messages(&mut self) {
        let limit = self.app.sync_config().page_size.clamp(20, 500);
        let result = if let Some(q) = self.query.clone() {
            // A hit in any mailbox is a hit: search always spans accounts.
            self.app.mail().search_all_local(&q, limit)
        } else {
            match self.selected_row() {
                Some(FolderRow::AllEmail) => self.app.mail().list_all(limit, 0, false),
                Some(FolderRow::Folder(ai, fi)) => self
                    .folders_by_account
                    .get(*ai)
                    .and_then(|v| v.get(*fi))
                    .map(|folder| self.app.mail().list(&folder.id, limit, 0, false))
                    .unwrap_or_else(|| Ok(Vec::new())),
                _ => Ok(Vec::new()),
            }
        };

        match result {
            Ok(mut messages) => {
                // Newest first is what every mail client does.
                messages.sort_by(|a, b| b.sort_ts().cmp(&a.sort_ts()));
                self.messages = messages;
                self.msg_idx = self.msg_idx.min(self.messages.len().saturating_sub(1));
                self.scroll = 0;
            }
            Err(e) => self.status = Some((format!("cannot read messages: {e}"), true)),
        }
    }

    pub fn reload_threads(&mut self) {
        let limit = self.app.sync_config().page_size.clamp(20, 500);
        // Folder views include conversations with at least one message there.
        let result = match self.selected_row() {
            Some(FolderRow::AllEmail) => self.app.mail().threads_all(limit, 0, false),
            Some(FolderRow::Folder(ai, fi)) => {
                if let Some(folder) = self.folders_by_account.get(*ai).and_then(|v| v.get(*fi)) {
                    self.app.mail().threads_folder(&folder.id, limit)
                } else {
                    return;
                }
            }
            _ => {
                let account_id = self.account.id.clone();
                self.app.mail().threads(&account_id, limit, 0, false)
            }
        };
        match result {
            Ok(threads) => {
                self.threads = threads;
                self.thread_idx = self.thread_idx.min(self.threads.len().saturating_sub(1));
            }
            Err(e) => self.status = Some((format!("cannot read conversations: {e}"), true)),
        }
    }

    /// `t`: swap the message pane between the flat list and conversations.
    pub fn toggle_list_mode(&mut self) {
        self.list_mode = match self.list_mode {
            ListMode::Flat => ListMode::Threads,
            ListMode::Threads => ListMode::Flat,
        };
        self.open_thread = None;
        self.scroll = 0;
        match self.list_mode {
            ListMode::Flat => self.reload_messages(),
            ListMode::Threads => {
                // Search applies to messages, not conversations.
                self.query = None;
                self.reload_threads();
            }
        }
    }

    /// Enter on a conversation row: load its messages, newest selected.
    pub fn open_selected_thread(&mut self) {
        let Some(thread) = self.selected_thread().cloned() else {
            return;
        };
        // A conversation belongs to one account even in the merged view.
        match self.app.mail().thread(&thread.account_id, &thread.id) {
            Ok(detail) => {
                let msg_idx = 0;
                self.open_thread = Some(OpenThread {
                    thread: detail.thread,
                    messages: detail.messages,
                    msg_idx,
                });
                self.scroll = 0;
            }
            Err(e) => self.status = Some((e.to_string(), true)),
        }
    }

    /// Esc inside a conversation: back to the conversation list.
    pub fn close_thread(&mut self) {
        self.open_thread = None;
        self.scroll = 0;
        self.reload_threads();
    }

    // --- actions ----------------------------------------------------------

    // --- flag toggles (optimistic) ----------------------------------------

    /// Config of whichever account owns a message — flag actions must run
    /// against the right provider even in the merged view.
    fn cfg_of(&self, account_id: &str) -> Option<AccountConfig> {
        self.app
            .account_configs()
            .iter()
            .find(|c| c.to_account().id == account_id)
            .cloned()
    }

    /// Mark the highlighted message read (or unread). The UI flips
    /// immediately; the provider call runs on a worker so a slow server never
    /// freezes the event loop.
    pub fn toggle_read(&mut self) {
        let Some(message) = self.selected_message().cloned() else {
            return;
        };
        let Some(cfg) = self.cfg_of(&message.account_id) else {
            self.status = Some((
                format!(
                    "account of message {} is no longer configured",
                    &message.id[..8.min(message.id.len())]
                ),
                true,
            ));
            return;
        };
        let target = !message.is_read;
        self.apply_flag_local(&message.id, &message.folder_id, Some(target), None);
        self.status = Some((
            if target {
                "marked read"
            } else {
                "marked unread"
            }
            .to_string(),
            false,
        ));

        let id = message.id.clone();
        self.run_persist_worker(cfg, move |app, cfg| app.mail().mark_read(cfg, &id, target));
    }

    pub fn toggle_star(&mut self) {
        let Some(message) = self.selected_message().cloned() else {
            return;
        };
        let Some(cfg) = self.cfg_of(&message.account_id) else {
            self.status = Some(("account of message is no longer configured".to_string(), true));
            return;
        };
        let target = !message.is_starred;
        self.apply_flag_local(&message.id, &message.folder_id, None, Some(target));

        let id = message.id.clone();
        self.run_persist_worker(cfg, move |app, cfg| app.mail().star(cfg, &id, target));
    }

    /// Patch every in-memory copy the panes are painted from — the flat list,
    /// the open conversation, the conversation row's unread count and the
    /// folder's — so the flip is visible before the worker confirms it.
    fn apply_flag_local(
        &mut self,
        id: &str,
        folder_id: &str,
        read: Option<bool>,
        starred: Option<bool>,
    ) {
        let patch = |m: &mut Message| {
            if let Some(v) = read {
                m.is_read = v;
            }
            if let Some(v) = starred {
                m.is_starred = v;
            }
        };
        for m in self.messages.iter_mut().chain(
            self.open_thread
                .as_mut()
                .map(|t| t.messages.as_mut_slice())
                .unwrap_or_default(),
        ) {
            if m.id == id {
                patch(m);
            }
        }

        let Some(read) = read else { return };
        if self.list_mode == ListMode::Threads {
            // The conversation rows show unread dots and counts.
            if let Some(t) = self.threads.get_mut(self.thread_idx) {
                t.unread_count = if read {
                    t.unread_count.saturating_sub(1)
                } else {
                    t.unread_count.saturating_add(1).min(t.message_count)
                };
            }
        } else if self.query.is_none() {
            // The flat list shows real folders — unless a search is active,
            // which spans folders. Update the folder the message lives in.
            if let Some(f) = self.folder_mut_by_id(folder_id) {
                f.unread_count = if read {
                    f.unread_count.saturating_sub(1)
                } else {
                    f.unread_count.saturating_add(1)
                };
            }
            self.all_unread = if read {
                self.all_unread.saturating_sub(1)
            } else {
                self.all_unread.saturating_add(1)
            };
        }
    }

    fn folder_mut_by_id(&mut self, folder_id: &str) -> Option<&mut Folder> {
        self.folders_by_account
            .iter_mut()
            .flat_map(|v| v.iter_mut())
            .find(|f| f.id == folder_id)
    }

    /// Persist a change off the UI thread, against the account that owns the
    /// message. Success is silent (the UI already shows it optimistically);
    /// failure comes back as [`WorkerMsg::Revert`].
    fn run_persist_worker<E: ToString>(
        &self,
        cfg: AccountConfig,
        job: impl FnOnce(&App, &AccountConfig) -> Result<(), E> + Send + 'static,
    ) {
        let app = Arc::clone(&self.app);
        let tx = self.tx.clone();
        thread::spawn(move || {
            if let Err(e) = job(&app, &cfg) {
                let _ = tx.send(WorkerMsg::Revert(e.to_string()));
            }
        });
    }

    /// Delete the highlighted message. It leaves the list immediately; the
    /// provider/local delete runs on a worker, so the UI never waits.
    pub fn delete_selected(&mut self) {
        let Some(message) = self.selected_message().cloned() else {
            return;
        };
        let Some(cfg) = self.cfg_of(&message.account_id) else {
            self.status = Some(("account of message is no longer configured".to_string(), true));
            return;
        };

        // Optimistic: drop the row and fix the counters in memory — a reload
        // now would read the store and bring the message back.
        if let Some((messages, idx)) = self.active_message_list() {
            if *idx < messages.len() {
                messages.remove(*idx);
            }
            if *idx >= messages.len() {
                *idx = messages.len().saturating_sub(1);
            }
        }

        if self.list_mode == ListMode::Threads {
            // Keep the conversation row's counts in step, and drop the row
            // once the last message in it went.
            if let Some(t) = self.threads.get_mut(self.thread_idx) {
                t.message_count = t.message_count.saturating_sub(1);
                if !message.is_read {
                    t.unread_count = t.unread_count.saturating_sub(1);
                }
            }
            if self
                .threads
                .get(self.thread_idx)
                .is_some_and(|t| t.message_count == 0)
            {
                self.threads.remove(self.thread_idx);
                self.thread_idx = self.thread_idx.min(self.threads.len().saturating_sub(1));
            }
            // Deleting the last message inside an open conversation closes it.
            if self
                .open_thread
                .as_ref()
                .is_some_and(|t| t.messages.is_empty())
            {
                self.open_thread = None;
            }
        } else if self.query.is_none() {
            // The flat list shows real folders — unless a search is active,
            // which spans folders. Update the folder the message lives in.
            if let Some(f) = self.folder_mut_by_id(&message.folder_id) {
                f.total_count = f.total_count.saturating_sub(1);
                if !message.is_read {
                    f.unread_count = f.unread_count.saturating_sub(1);
                }
            }
            if !message.is_read {
                self.all_unread = self.all_unread.saturating_sub(1);
            }
        }

        self.status = Some(("deleted".to_string(), false));

        let id = message.id.clone();
        self.run_persist_worker(cfg, move |app, cfg| app.mail().delete(cfg, &id));
    }

    /// Open the highlighted message: mark it read and move focus to the reader.
    pub fn open_message(&mut self) {
        let Some(message) = self.selected_message().cloned() else {
            return;
        };
        self.focus = Focus::Preview;
        self.scroll = 0;
        if !message.is_read {
            // Optimistic, like `u`: flip now, persist on a worker — against
            // the account that owns the message.
            self.apply_flag_local(&message.id, &message.folder_id, Some(true), None);
            if let Some(cfg) = self.cfg_of(&message.account_id) {
                let id = message.id.clone();
                self.run_persist_worker(cfg, move |app, cfg| {
                    app.mail().mark_read(cfg, &id, true)
                });
            }
        }
    }

    pub fn set_query(&mut self, q: String) {
        if q.trim().is_empty() {
            self.query = None;
        } else {
            // Search results are plain messages; leave the conversation view.
            self.list_mode = ListMode::Flat;
            self.open_thread = None;
            self.query = Some(q);
            self.focus = Focus::List;
        }
        self.reload_messages();
    }

    pub fn clear_query(&mut self) {
        if self.query.take().is_some() {
            self.reload_messages();
        }
    }

    // --- background work --------------------------------------------------

    pub fn start_sync(&mut self) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.status = Some(("syncing…".to_string(), false));

        let app = Arc::clone(&self.app);
        let tx = self.tx.clone();
        thread::spawn(move || {
            let _ = tx.send(sync_all_accounts(&app));
        });
    }

    /// Build and send the compose form currently on screen.
    pub fn send_compose(&mut self) {
        let form = match &self.overlay {
            Overlay::Compose(f) if !f.sending => f.clone(),
            _ => return,
        };

        let signature = self
            .app
            .config()
            .general
            .compose
            .signature
            .clone()
            .unwrap_or_default();

        let mut compose = ComposeMessage::new();
        compose.from = Some(Address::with_name(
            self.account
                .display_name
                .as_deref()
                .unwrap_or(&self.account.email),
            &self.account.email,
        ));
        for address in jianxin_domain::parse_address_list(&form.to) {
            compose.to.push(address);
        }
        compose.subject = form.subject.trim().to_string();
        compose.text_body = Some(with_signature(&form.body, &signature));
        compose.in_reply_to = form.in_reply_to.clone();
        compose.references = form.references.clone();

        if !compose.has_recipients() {
            if let Overlay::Compose(f) = &mut self.overlay {
                f.sending = false;
                f.error = Some("no recipients — fill in To".to_string());
            }
            return;
        }

        self.busy = true;
        if let Overlay::Compose(f) = &mut self.overlay {
            f.sending = true;
            f.error = None;
        }
        self.status = Some(("sending…".to_string(), false));

        let app = Arc::clone(&self.app);
        let cfg = self.cfg.clone();
        let tx = self.tx.clone();
        thread::spawn(move || {
            let outcome = match app.compose().send(&cfg, &compose) {
                Ok(result) => WorkerMsg::Sent(
                    result
                        .remote_id
                        .clone()
                        .or(result.message_id_header.clone())
                        .unwrap_or_else(|| "ok".to_string()),
                ),
                Err(e) => WorkerMsg::Failed(e.to_string()),
            };
            let _ = tx.send(outcome);
        });
    }

    /// Drain whatever the workers have produced. Called once per frame.
    pub fn poll(&mut self) {
        if self.last_refresh.elapsed() >= Duration::from_secs(1) {
            self.last_refresh = Instant::now();
            let version = {
                let conn = self.app.db().lock();
                conn.query_row("PRAGMA data_version", [], |row| row.get::<_, i64>(0)).unwrap_or(-1)
            };
            self.refresh_account_statuses();
            if version != self.database_version {
                self.database_version = version;
                let message_id = self.selected_message().map(|m| m.id.clone());
                let thread_id = self.selected_thread().map(|t| t.id.clone());
                let scroll = self.scroll;
                self.reload_folders();
                self.reload_messages();
                if self.list_mode == ListMode::Threads {
                    self.reload_threads();
                    if let Some(id) = thread_id { if let Some(index) = self.threads.iter().position(|t| t.id == id) { self.thread_idx = index; } }
                    self.refresh_open_thread();
                    if let Some(id) = message_id {
                        if let Some(open) = self.open_thread.as_mut() { if let Some(index) = open.messages.iter().position(|m| m.id == id) { open.msg_idx = index; } }
                    }
                } else if let Some(id) = message_id { if let Some(index) = self.messages.iter().position(|m| m.id == id) { self.msg_idx = index; } }
                self.scroll = scroll;
            }
        }
        while let Ok(msg) = self.rx.try_recv() {
            // Workers persist their account outcome before notifying the UI.
            // Refresh even when SQLite did not change (e.g. auth failures).
            self.refresh_account_statuses();
            match msg {
                WorkerMsg::BrowserOpened(success) => {
                    self.status = Some((if success { "邮件已在浏览器打开" }
                        else { "无法打开浏览器；可用 jianxin web 获取本地阅读网址" }.into(), !success));
                }
                WorkerMsg::Synced(summary) => {
                    self.busy = false;
                    self.status = Some((format!("synced — {summary}"), false));
                    self.reload_folders();
                    self.reload_messages();
                    if self.list_mode == ListMode::Threads {
                        self.reload_threads();
                        self.refresh_open_thread();
                    }
                }
                WorkerMsg::Sent(id) => {
                    self.busy = false;
                    self.status = Some((format!("sent ({id})"), false));
                    self.overlay = Overlay::None;
                    self.reload_folders();
                    self.reload_messages();
                    if self.list_mode == ListMode::Threads {
                        self.reload_threads();
                        self.refresh_open_thread();
                    }
                }
                WorkerMsg::Failed(e) => {
                    self.busy = false;
                    self.status = Some((e.clone(), true));
                    if let Overlay::Compose(f) = &mut self.overlay {
                        f.sending = false;
                        f.error = Some(e);
                    }
                }
                WorkerMsg::Revert(e) => {
                    // The optimistic change did not stick — repaint from the
                    // store, which rolls it back everywhere it is shown.
                    self.status = Some((e, true));
                    self.reload_folders();
                    self.reload_messages();
                    if self.list_mode == ListMode::Threads {
                        self.reload_threads();
                        self.refresh_open_thread();
                    }
                }
            }
        }
    }

    fn refresh_account_statuses(&mut self) {
        self.account_statuses = self.app.account_configs().iter().enumerate().map(|(index, cfg)| {
            jianxin_core::read_account_sync_status(
                self.app.config(),
                &cfg.to_account().id,
                self.folders_by_account.get(index).is_some_and(|folders| !folders.is_empty()),
            ).tui_label().to_string()
        }).collect();
    }

    /// Re-pull the open conversation, keeping the cursor on the same message.
    fn refresh_open_thread(&mut self) {
        let Some(open) = &self.open_thread else {
            return;
        };
        let thread_id = open.thread.id.clone();
        let idx = open.msg_idx;
        match self.app.mail().thread(&open.thread.account_id, &thread_id) {
            Ok(detail) => {
                if let Some(open) = self.open_thread.as_mut() {
                    open.thread = detail.thread;
                    open.msg_idx = idx.min(detail.messages.len().saturating_sub(1));
                    open.messages = detail.messages;
                }
            }
            // The conversation vanished (last message deleted) — back out.
            Err(_) => self.open_thread = None,
        }
    }

    pub fn close_overlay(&mut self) {
        if !self.overlay.is_none() {
            self.overlay = Overlay::None;
        }
    }

    /// Reply to the highlighted message: prefilled To and subject, quoted body.
    pub fn reply_form(&self) -> ComposeForm {
        let mut form = ComposeForm::default();
        let Some(message) = self.selected_message() else {
            return form;
        };

        // Never reply to the account that actually owns the message.
        let own_email = self
            .cfg_of(&message.account_id)
            .map(|c| c.email)
            .unwrap_or_default();

        // Prefer Reply-To, else the sender.
        let targets = if !message.reply_to.is_empty() {
            message.reply_to.clone()
        } else {
            message.from.clone()
        };
        form.to = targets
            .iter()
            .filter(|a| a.email != own_email)
            .map(|a| a.email.clone())
            .collect::<Vec<_>>()
            .join(", ");
        if form.to.is_empty() {
            form.to = targets
                .iter()
                .map(|a| a.email.clone())
                .collect::<Vec<_>>()
                .join(", ");
        }

        let subject = message.subject.trim();
        form.subject = if subject.to_ascii_lowercase().starts_with("re:") {
            subject.to_string()
        } else {
            format!("Re: {subject}")
        };

        // Carry the threading headers so the reply lands in the same
        // conversation on every client, not just ours.
        form.in_reply_to = message.message_id_header.clone();
        form.references = jianxin_domain::thread::build_references(message);

        let quoted: String = message
            .body_preferred()
            .lines()
            .map(|l| format!("> {l}"))
            .collect::<Vec<_>>()
            .join("\n");
        form.body = format!("\n\n{} wrote:\n{}", message.sender().display(), quoted);
        form.field = 2;
        form
    }

    // --- pane helpers -----------------------------------------------------

    /// Whether at least one account has folders — the "first run" check that
    /// decides whether to sync before the first paint.
    pub fn has_any_folders(&self) -> bool {
        self.folders_by_account.iter().any(|v| !v.is_empty())
    }

    /// Whether the message list currently spans accounts (All Email, or an
    /// active search) — the list then shows an account column.
    pub fn spans_accounts(&self) -> bool {
        self.query.is_some() || matches!(self.selected_row(), Some(FolderRow::AllEmail))
    }

    /// Full email address for the list's account column.
    pub fn account_label(&self, account_id: &str) -> &str {
        self.app
            .account_configs()
            .iter()
            .find(|c| c.to_account().id == account_id)
            .map(|c| c.email.as_str())
            .unwrap_or("?")
    }
}

/// Sync every configured account on one worker; per-account failures do not
/// stop the rest.
fn sync_all_accounts(app: &App) -> WorkerMsg {
    let mut synced: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    for cfg in app.account_configs() {
        match sync_account(app, cfg) {
            Ok(summary) => synced.push(format!("{}: {summary}", cfg.alias())),
            Err(e) => failed.push(format!("{}: {e}", cfg.alias())),
        }
    }
    if synced.is_empty() && failed.is_empty() {
        return WorkerMsg::Failed("no accounts configured".to_string());
    }
    let mut text = synced.join("; ");
    if !failed.is_empty() {
        if !text.is_empty() {
            text.push_str(" — ");
        }
        text.push_str(&format!("failed: {}", failed.join("; ")));
    }
    if synced.is_empty() {
        WorkerMsg::Failed(text)
    } else {
        WorkerMsg::Synced(text)
    }
}

fn sync_account(app: &App, cfg: &AccountConfig) -> Result<String, String> {
    let result = sync_account_inner(app, cfg);
    app.record_sync_status(&cfg.to_account().id, result.as_ref().err().map(String::as_str));
    result
}

fn sync_account_inner(app: &App, cfg: &AccountConfig) -> Result<String, String> {
    let account = cfg.to_account();
    let provider = app.provider(cfg).map_err(|e| e.to_string())?;
    let report = app
        .sync()
        .sync_account(&*provider, &account.id, false)
        .map_err(|e| e.to_string())?;
    if !report.failures.is_empty() {
        return Err(report.failures.iter().map(|f| format!("{}: {}", f.folder_name, f.error)).collect::<Vec<_>>().join("; "));
    }
    Ok(format!(
        "{} new, {} flagged, {} removed",
        report.total_upserted(),
        report.total_flagged(),
        report.total_removed()
    ))
}

/// Append the configured signature under the body, on its own `-- ` block.
fn with_signature(body: &str, signature: &str) -> String {
    if signature.trim().is_empty() {
        body.to_string()
    } else {
        format!("{body}\n-- \n{signature}\n")
    }
}
