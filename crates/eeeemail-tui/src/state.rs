//! UI state for the terminal client.
//!
//! The state owns no business logic — every action goes through
//! [`eeeemail_core`] services. What it *does* own is the one thing the UI
//! cannot do without: which account, folder and message the user is looking at,
//! and how to keep painting while the network is busy.
//!
//! Network work runs on a worker thread and reports back over a channel, so a
//! slow IMAP server never freezes the render loop.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;

use eeeemail_config::AccountConfig;
use eeeemail_core::App;
use eeeemail_domain::{Account, Address, ComposeMessage, Folder, Message};

/// Which pane receives the arrow keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Folders,
    List,
    Preview,
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
    /// Sync finished; the payload is a human summary.
    Synced(String),
    /// Mail went out.
    Sent(String),
    /// Both of the above can fail.
    Failed(String),
}

/// The compose form. Fields are plain strings — parsing happens at send time so
/// a half-typed address is not an error until the user asks us to send.
#[derive(Debug, Clone, Default)]
pub struct ComposeForm {
    pub to: String,
    pub subject: String,
    pub body: String,
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

pub struct State {
    pub app: Arc<App>,
    pub cfg: AccountConfig,
    pub account: Account,
    pub account_idx: usize,

    pub folders: Vec<Folder>,
    pub folder_idx: usize,
    pub messages: Vec<Message>,
    pub msg_idx: usize,

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
}

impl State {
    pub fn new(app: Arc<App>) -> anyhow::Result<Self> {
        let cfg = app
            .account_configs()
            .first()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no accounts configured — add one to config.json"))?;

        let (tx, rx) = mpsc::channel();
        let mut state = Self {
            app,
            account: cfg.to_account(),
            account_idx: 0,
            cfg,
            folders: Vec::new(),
            folder_idx: 0,
            messages: Vec::new(),
            msg_idx: 0,
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
        };
        state.reload_folders();
        state.reload_messages();
        Ok(state)
    }

    // --- navigation -------------------------------------------------------

    pub fn selected_folder(&self) -> Option<&Folder> {
        self.folders.get(self.folder_idx)
    }

    pub fn selected_message(&self) -> Option<&Message> {
        self.messages.get(self.msg_idx)
    }

    pub fn move_folder(&mut self, delta: isize) {
        if self.folders.is_empty() {
            return;
        }
        let n = self.folders.len() as isize;
        let i = (self.folder_idx as isize + delta).clamp(0, n - 1) as usize;
        if i != self.folder_idx {
            self.folder_idx = i;
            self.reload_messages();
        }
    }

    pub fn move_message(&mut self, delta: isize) {
        if self.messages.is_empty() {
            return;
        }
        let n = self.messages.len() as isize;
        self.msg_idx = (self.msg_idx as isize + delta).clamp(0, n - 1) as usize;
        self.scroll = 0;
    }

    pub fn jump(&mut self, to_end: bool) {
        if self.focus == Focus::Folders {
            self.folder_idx = if to_end {
                self.folders.len().saturating_sub(1)
            } else {
                0
            };
            self.reload_messages();
        } else {
            self.msg_idx = if to_end {
                self.messages.len().saturating_sub(1)
            } else {
                0
            };
            self.scroll = 0;
        }
    }

    /// Switch to another configured account.
    pub fn cycle_account(&mut self, delta: isize) -> bool {
        let n = self.app.account_configs().len();
        if n < 2 {
            return false;
        }
        self.account_idx = (self.account_idx as isize + delta).rem_euclid(n as isize) as usize;
        self.cfg = self.app.account_configs()[self.account_idx].clone();
        self.account = self.cfg.to_account();
        self.folder_idx = 0;
        self.msg_idx = 0;
        self.query = None;
        self.reload_folders();
        self.reload_messages();
        true
    }

    // --- data -------------------------------------------------------------

    pub fn reload_folders(&mut self) {
        match self.app.mail().folders(&self.account.id) {
            Ok(folders) => {
                self.folders = folders;
                if self.folder_idx >= self.folders.len() {
                    self.folder_idx = self.folders.len().saturating_sub(1);
                }
            }
            Err(e) => self.status = Some((format!("cannot read folders: {e}"), true)),
        }
    }

    pub fn reload_messages(&mut self) {
        let limit = self.app.sync_config().page_size.clamp(20, 500);
        let result = if let Some(q) = self.query.clone() {
            self.app.mail().search_local(&self.account.id, &q, limit)
        } else {
            match self.selected_folder() {
                Some(folder) => self
                    .app
                    .mail()
                    .list(&folder.id, limit, 0, false)
                    .map(|mut v| {
                        // Newest first is what every mail client does.
                        v.sort_by(|a, b| b.sort_ts().cmp(&a.sort_ts()));
                        v
                    }),
                None => Ok(Vec::new()),
            }
        };

        match result {
            Ok(messages) => {
                self.messages = messages;
                self.msg_idx = self.msg_idx.min(self.messages.len().saturating_sub(1));
                self.scroll = 0;
            }
            Err(e) => self.status = Some((format!("cannot read messages: {e}"), true)),
        }
    }

    // --- actions ----------------------------------------------------------

    /// Mark the highlighted message read (or unread).
    pub fn toggle_read(&mut self) {
        let Some(message) = self.selected_message().cloned() else {
            return;
        };
        let target = !message.is_read;
        match self.app.mail().mark_read(&self.cfg, &message.id, target) {
            Ok(()) => {
                if let Some(m) = self.messages.get_mut(self.msg_idx) {
                    m.is_read = target;
                }
                self.status = Some((if target { "marked read" } else { "marked unread" }.to_string(), false));
            }
            Err(e) => self.status = Some((e.to_string(), true)),
        }
    }

    pub fn toggle_star(&mut self) {
        let Some(message) = self.selected_message().cloned() else {
            return;
        };
        let target = !message.is_starred;
        match self.app.mail().star(&self.cfg, &message.id, target) {
            Ok(()) => {
                if let Some(m) = self.messages.get_mut(self.msg_idx) {
                    m.is_starred = target;
                }
            }
            Err(e) => self.status = Some((e.to_string(), true)),
        }
    }

    pub fn delete_selected(&mut self) {
        let Some(message) = self.selected_message().cloned() else {
            return;
        };
        match self.app.mail().delete(&self.cfg, &message.id) {
            Ok(()) => {
                self.messages.remove(self.msg_idx);
                if self.msg_idx >= self.messages.len() {
                    self.msg_idx = self.messages.len().saturating_sub(1);
                }
                self.status = Some(("deleted".to_string(), false));
                self.reload_folders();
            }
            Err(e) => self.status = Some((e.to_string(), true)),
        }
    }

    /// Open the highlighted message: mark it read and move focus to the reader.
    pub fn open_message(&mut self) {
        let Some(message) = self.selected_message().cloned() else {
            return;
        };
        self.focus = Focus::Preview;
        self.scroll = 0;
        if !message.is_read {
            if self.app.mail().mark_read(&self.cfg, &message.id, true).is_ok() {
                if let Some(m) = self.messages.get_mut(self.msg_idx) {
                    m.is_read = true;
                }
            }
        }
    }

    pub fn set_query(&mut self, q: String) {
        if q.trim().is_empty() {
            self.query = None;
        } else {
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
        let cfg = self.cfg.clone();
        let tx = self.tx.clone();
        thread::spawn(move || {
            let outcome = match sync_account(&app, &cfg) {
                Ok(summary) => WorkerMsg::Synced(summary),
                Err(e) => WorkerMsg::Failed(e),
            };
            let _ = tx.send(outcome);
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
            self.account.display_name.as_deref().unwrap_or(&self.account.email),
            &self.account.email,
        ));
        for address in eeeemail_domain::parse_address_list(&form.to) {
            compose.to.push(address);
        }
        compose.subject = form.subject.trim().to_string();
        compose.text_body = Some(with_signature(&form.body, &signature));

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
        while let Ok(msg) = self.rx.try_recv() {
            self.busy = false;
            match msg {
                WorkerMsg::Synced(summary) => {
                    self.status = Some((format!("synced — {summary}"), false));
                    self.reload_folders();
                    self.reload_messages();
                }
                WorkerMsg::Sent(id) => {
                    self.status = Some((format!("sent ({id})"), false));
                    self.overlay = Overlay::None;
                    self.reload_folders();
                    self.reload_messages();
                }
                WorkerMsg::Failed(e) => {
                    self.status = Some((e.clone(), true));
                    if let Overlay::Compose(f) = &mut self.overlay {
                        f.sending = false;
                        f.error = Some(e);
                    }
                }
            }
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

        // Prefer Reply-To, else the sender; never reply to yourself.
        let targets = if !message.reply_to.is_empty() {
            message.reply_to.clone()
        } else {
            message.from.clone()
        };
        form.to = targets
            .iter()
            .filter(|a| a.email != self.account.email)
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
}

fn sync_account(app: &App, cfg: &AccountConfig) -> Result<String, String> {
    let account = cfg.to_account();
    let provider = app.provider(cfg).map_err(|e| e.to_string())?;
    let report = app
        .sync()
        .sync_account(&*provider, &account.id, false)
        .map_err(|e| e.to_string())?;
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
