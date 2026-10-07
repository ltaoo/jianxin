//! Generic IMAP + SMTP provider.
//!
//! Covers 163, 126, QQ, aliyun, Tencent, Zoho, iCloud, Fastmail and any
//! self-hosted mailbox, plus Gmail/Outlook when they are used with an app
//! password. IMAP does not expose threads, labels or push, so the capability
//! set says so and the UI adapts instead of pretending otherwise.

use jianxin_config::{AccountConfig, FolderOverrides, SyncConfig, TlsMode};
use jianxin_domain::{
    Account, Address, AuthType, Capability, ComposeMessage, FlagUpdate, Folder, FolderKind,
    MailError, MailErrorCode, Message, MessagePage, ProviderCapabilities, ProviderId, Result,
    SendResult, SyncCursor,
};
use std::sync::Arc;
use std::time::Duration;

use crate::auth::{imap_quote, AuthProvider, AuthSecret, ConfigAuth};
use crate::imap::client::ImapClient;
use crate::imap::extensions;
use crate::imap::mapper::{folder_from_raw, message_from_raw};
use crate::imap::ImapEndpoint;
use crate::smtp::{self, SmtpEndpoint};
use crate::traits::{ListMessagesOptions, MailProvider, SearchQuery, SyncDelta};

/// Tuning knobs shared by all sync operations.
#[derive(Debug, Clone, Copy)]
pub struct SyncSettings {
    pub page_size: usize,
    pub max_messages_per_folder: usize,
    pub timeout: Duration,
}

impl Default for SyncSettings {
    fn default() -> Self {
        Self {
            page_size: 50,
            max_messages_per_folder: 500,
            timeout: Duration::from_secs(30),
        }
    }
}

impl From<&SyncConfig> for SyncSettings {
    fn from(config: &SyncConfig) -> Self {
        Self {
            page_size: config.page_size.max(1) as usize,
            max_messages_per_folder: config.max_messages_per_folder.max(1) as usize,
            timeout: Duration::from_secs(config.timeout_secs.max(1)),
        }
    }
}

pub struct GenericImapProvider {
    account: Account,
    imap: ImapEndpoint,
    smtp: SmtpEndpoint,
    auth: Arc<dyn AuthProvider>,
    folders: Option<FolderOverrides>,
    sync: SyncSettings,
    /// Store a copy in the Sent folder via IMAP APPEND after sending.
    save_sent: bool,
}

impl GenericImapProvider {
    pub fn new(
        account: Account,
        imap: ImapEndpoint,
        smtp: SmtpEndpoint,
        auth: Arc<dyn AuthProvider>,
        folders: Option<FolderOverrides>,
        sync: SyncSettings,
    ) -> Self {
        Self {
            account,
            imap,
            smtp,
            auth,
            folders,
            sync,
            save_sent: true,
        }
    }

    /// Build a provider straight from a JSON account entry.
    pub fn from_config(config: &AccountConfig, sync: &SyncConfig) -> Result<Self> {
        let account = config.to_account();
        let auth: Arc<dyn AuthProvider> = Arc::new(ConfigAuth::from_account(config));

        let timeout = Duration::from_secs(sync.timeout_secs.max(1));
        let imap_cfg = config.imap_server();
        let smtp_cfg = config.smtp_server();

        Ok(Self::new(
            account,
            ImapEndpoint::new(&imap_cfg.host, imap_cfg.port, imap_cfg.tls, timeout),
            SmtpEndpoint::new(&smtp_cfg.host, smtp_cfg.port, smtp_cfg.tls, timeout),
            auth,
            config.folders.clone(),
            SyncSettings::from(sync),
        ))
    }

    pub fn set_save_sent(&mut self, save: bool) {
        self.save_sent = save;
    }

    fn connect(&self) -> Result<ImapClient> {
        ImapClient::connect(&self.imap, self.auth.as_ref())
    }

    /// Resolve the default sender address for this account.
    pub fn default_from(&self) -> Address {
        match &self.account.display_name {
            Some(name) if !name.is_empty() => {
                Address::with_name(name.clone(), self.account.email.clone())
            }
            _ => Address::new(self.account.email.clone()),
        }
    }

    /// Domain folder list, sorted so Inbox comes first.
    fn sorted_folders(&self) -> Result<Vec<Folder>> {
        let mut client = self.connect()?;
        let raws = client.list_folders()?;
        let mut folders: Vec<Folder> = raws
            .iter()
            // `\Noselect` entries (e.g. Gmail's `[Gmail]` namespace root)
            // cannot be SELECTed; syncing them fails with "Unknown Mailbox".
            .filter(|raw| !raw.attributes.split_whitespace().any(|a| a == "\\Noselect"))
            .map(|raw| folder_from_raw(raw, &self.account.id, self.folders.as_ref()))
            .collect();

        folders.sort_by_key(|f| (rank(f.kind), f.name.to_lowercase()));
        Ok(folders)
    }

    /// Render the message and store it in the remote Sent folder.
    ///
    /// Best-effort on purpose: Gmail and Office 365 already keep their own sent
    /// copy, and a failure here must not turn a successful send into an error.
    fn store_sent_copy(&self, message: &ComposeMessage, from: &Address) {
        if !self.save_sent {
            return;
        }
        let Ok(email) = smtp::build_message(message, from) else {
            return;
        };
        let raw = email.formatted();

        if let Ok(mut client) = self.connect() {
            if let Ok(folders) = self.sorted_folders() {
                if let Some(sent) = folders.iter().find(|f| f.kind == FolderKind::Sent) {
                    if let Err(err) = client.append(&sent.remote_id, &raw, true) {
                        tracing::warn!("cannot store sent copy: {err}");
                    }
                }
            }
        }
    }
}

fn rank(kind: FolderKind) -> u8 {
    match kind {
        FolderKind::Inbox => 0,
        FolderKind::Archive => 1,
        FolderKind::Drafts => 2,
        FolderKind::Sent => 3,
        FolderKind::Custom => 4,
        FolderKind::Spam => 5,
        FolderKind::Trash => 6,
    }
}

impl MailProvider for GenericImapProvider {
    fn id(&self) -> ProviderId {
        ProviderId::GenericImap
    }

    fn account_id(&self) -> &str {
        &self.account.id
    }

    fn account_email(&self) -> &str {
        &self.account.email
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::imap()
    }

    fn list_folders(&self) -> Result<Vec<Folder>> {
        self.sorted_folders()
    }

    fn list_messages(&self, folder: &Folder, opts: ListMessagesOptions) -> Result<MessagePage> {
        let mut client = self.connect()?;
        client.select(&folder.remote_id)?;

        let criteria = if opts.unread_only { "UNSEEN" } else { "ALL" };
        let mut uids = client.uid_search(criteria)?;
        // Highest UID first: UIDs are allocated in arrival order on every
        // server that follows RFC 3501.
        uids.sort_unstable();
        uids.reverse();

        let start = (opts.offset as usize).min(uids.len());
        let limit = if opts.limit == 0 {
            50
        } else {
            opts.limit as usize
        };
        let end = (start + limit).min(uids.len());
        let page = &uids[start..end];

        let set = page
            .iter()
            .map(|uid| uid.to_string())
            .collect::<Vec<_>>()
            .join(",");

        let raws = client.fetch_messages(&set)?;
        let messages = raws
            .iter()
            .map(|raw| message_from_raw(raw, &self.account.id, &folder.id, folder.kind))
            .collect::<Result<Vec<_>>>()?;

        Ok(MessagePage {
            has_more: end < uids.len(),
            messages,
            next_cursor: None,
        })
    }

    fn get_message(&self, folder: &Folder, remote_id: &str) -> Result<Message> {
        let mut client = self.connect()?;
        client.select(&folder.remote_id)?;
        let raws = client.fetch_messages(remote_id)?;
        let raw = raws.first().ok_or_else(|| {
            MailError::new(
                MailErrorCode::MessageNotFound,
                format!("no message with uid {remote_id} in {}", folder.remote_id),
            )
        })?;
        message_from_raw(raw, &self.account.id, &folder.id, folder.kind)
    }

    fn send_message(&self, message: &ComposeMessage) -> Result<SendResult> {
        self.require(Capability::Send)?;
        let from = message.from.clone().unwrap_or_else(|| self.default_from());

        let (secret, is_oauth2) = match self.auth.imap_secret()? {
            AuthSecret::Password(password) => (password, false),
            AuthSecret::OAuth2(token) => (token, true),
        };

        let result = smtp::send(
            &self.smtp,
            self.auth.username(),
            &secret,
            is_oauth2,
            message,
            &from,
        )?;

        self.store_sent_copy(message, &from);
        Ok(result)
    }

    fn create_draft(&self, folder: &Folder, message: &ComposeMessage) -> Result<Message> {
        self.require(Capability::Drafts)?;
        let from = message.from.clone().unwrap_or_else(|| self.default_from());
        let email = smtp::build_message(message, &from)?;
        let raw = email.formatted();

        let mut client = self.connect()?;
        client.append(&folder.remote_id, &raw, false)?;
        drop(client);

        // APPEND does not report the new UID, so re-read the newest message.
        let page = self.list_messages(folder, ListMessagesOptions::new(1))?;
        page.messages.first().cloned().ok_or_else(|| {
            MailError::new(
                MailErrorCode::MessageNotFound,
                "draft was appended but could not be read back",
            )
        })
    }

    fn mark_read(&self, folder: &Folder, remote_id: &str, read: bool) -> Result<()> {
        let uid = parse_uid(remote_id)?;
        let mut client = self.connect()?;
        client.select(&folder.remote_id)?;
        client.set_read(uid, read)
    }

    fn set_flag(&self, folder: &Folder, remote_id: &str, starred: bool) -> Result<()> {
        let uid = parse_uid(remote_id)?;
        let mut client = self.connect()?;
        client.select(&folder.remote_id)?;
        client.set_flagged(uid, starred)
    }

    fn move_message(&self, folder: &Folder, remote_id: &str, target: &Folder) -> Result<()> {
        let uid = parse_uid(remote_id)?;
        let mut client = self.connect()?;
        client.select(&folder.remote_id)?;
        // No MOVE on many servers, so COPY + \Deleted + EXPUNGE.
        client.copy(uid, &target.remote_id)?;
        client.delete(uid)
    }

    fn delete_message(&self, folder: &Folder, remote_id: &str) -> Result<()> {
        let uid = parse_uid(remote_id)?;
        let mut client = self.connect()?;
        client.select(&folder.remote_id)?;
        client.delete(uid)
    }

    fn search(&self, query: &SearchQuery) -> Result<MessagePage> {
        self.require(Capability::ServerSearch)?;

        let mut criteria: Vec<String> = Vec::new();
        if let Some(text) = &query.text {
            criteria.push(format!("TEXT {}", imap_quote(text)));
        }
        if let Some(subject) = &query.subject {
            criteria.push(format!("SUBJECT {}", imap_quote(subject)));
        }
        if let Some(from) = &query.from {
            criteria.push(format!("FROM {}", imap_quote(from)));
        }
        if let Some(to) = &query.to {
            criteria.push(format!("TO {}", imap_quote(to)));
        }
        if query.unread_only {
            criteria.push("UNSEEN".to_string());
        }
        if criteria.is_empty() {
            criteria.push("ALL".to_string());
        }
        let criteria = criteria.join(" ");

        let limit = if query.limit == 0 {
            50
        } else {
            query.limit as usize
        };

        // With an explicit folder we can use one connection; otherwise walk the
        // syncable folders of the account.
        let folders: Vec<Folder> = match &query.folder_id {
            Some(id) => self
                .sorted_folders()?
                .into_iter()
                .filter(|f| f.id == *id)
                .collect(),
            None => self
                .sorted_folders()?
                .into_iter()
                .filter(|f| f.kind.sync_by_default())
                .collect(),
        };

        let mut out = MessagePage::default();
        let mut client = self.connect()?;
        for folder in folders {
            if out.messages.len() >= limit {
                out.has_more = true;
                break;
            }
            if client.select(&folder.remote_id).is_err() {
                continue;
            }
            let mut uids = match client.uid_search(&criteria) {
                Ok(uids) => uids,
                Err(_) => continue,
            };
            uids.sort_unstable();
            uids.reverse();
            uids.truncate(limit - out.messages.len());
            if uids.is_empty() {
                continue;
            }
            let set = uids
                .iter()
                .map(|uid| uid.to_string())
                .collect::<Vec<_>>()
                .join(",");
            if let Ok(raws) = client.fetch_messages(&set) {
                for raw in raws {
                    if let Ok(message) =
                        message_from_raw(&raw, &self.account.id, &folder.id, folder.kind)
                    {
                        out.messages.push(message);
                    }
                }
            }
        }

        out.messages.sort_by_key(|m| std::cmp::Reverse(m.sort_ts()));
        Ok(out)
    }

    fn sync(&self, folder: &Folder, cursor: Option<&SyncCursor>) -> Result<SyncDelta> {
        self.require(Capability::DeltaSync)?;

        let mut client = self.connect()?;
        let status = client.select(&folder.remote_id)?;
        let uid_validity = status.uid_validity.unwrap_or(0);

        let (last_validity, last_max_uid) = match cursor {
            Some(cursor) => parse_cursor(&cursor.cursor),
            None => (0, 0),
        };

        let all_uids = client.uid_search("ALL")?;
        let max_uid = all_uids.last().copied().unwrap_or(0);

        // A UIDVALIDITY change means the server invalidated every UID, so the
        // local copy is worthless and must be rebuilt from scratch.
        let full = last_validity != uid_validity;
        let mut delta = SyncDelta::new(&folder.id, &folder.name);
        delta.present_remote_ids = all_uids.iter().map(|uid| uid.to_string()).collect();

        let new_uids: Vec<u32> = if full {
            let take = all_uids.len().min(self.sync.max_messages_per_folder);
            all_uids[all_uids.len() - take..].to_vec()
        } else {
            all_uids
                .iter()
                .copied()
                .filter(|uid| *uid > last_max_uid)
                .collect()
        };

        for chunk in new_uids.chunks(self.sync.page_size.max(1)) {
            let set = chunk
                .iter()
                .map(|uid| uid.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let raws = client.fetch_messages(&set)?;
            for raw in raws {
                let message = message_from_raw(&raw, &self.account.id, &folder.id, folder.kind)?;
                delta.new_messages.push(message);
            }
        }

        // Server-assigned thread keys (Gmail `X-GM-THRID`, RFC 5256 `THREAD`).
        // The imap crate cannot parse those responses on its own session, so
        // they run on a throwaway raw connection. A failure only costs us a
        // hint — local header threading still applies.
        if !new_uids.is_empty() {
            let want_thread = client.has_capability("THREAD=REFERENCES");
            let want_thrid = client.has_capability("X-GM-EXT-1");
            if want_thread || want_thrid {
                match extensions::fetch_thread_hints(
                    &self.imap,
                    self.auth.as_ref(),
                    &folder.remote_id,
                    &new_uids,
                    uid_validity,
                    want_thread,
                    want_thrid,
                ) {
                    Ok(hints) => {
                        for message in &mut delta.new_messages {
                            if let Ok(uid) = message.remote_id.parse::<u32>() {
                                if let Some(key) = hints.get(&uid) {
                                    message.server_thread_key = Some(key.clone());
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(folder = %folder.remote_id, error = %e, "thread extension hints unavailable")
                    }
                }
            }
        }

        // Flag-only changes: cheap FLAGS fetch over everything we already have.
        if !full && last_max_uid > 0 {
            let known: Vec<u32> = all_uids
                .iter()
                .copied()
                .filter(|uid| *uid <= last_max_uid)
                .collect();
            for chunk in known.chunks((self.sync.page_size.max(1)) * 10) {
                let set = chunk
                    .iter()
                    .map(|uid| uid.to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                let updates = client.fetch_flags(&set)?;
                for (uid, flags) in updates {
                    delta.flag_updates.push(FlagUpdate {
                        remote_id: uid.to_string(),
                        is_read: flags.iter().any(|f| f.eq_ignore_ascii_case("\\Seen")),
                        is_starred: flags.iter().any(|f| f.eq_ignore_ascii_case("\\Flagged")),
                    });
                }
            }
        }

        delta.cursor = Some(SyncCursor {
            provider: self.id(),
            account_id: self.account.id.clone(),
            folder_id: folder.id.clone(),
            cursor_type: "uid".to_string(),
            cursor: format!("{uid_validity}:{max_uid}"),
        });

        Ok(delta)
    }

    fn test_connection(&self) -> Result<String> {
        let mut client = self.connect()?;
        let folders = client.list_folders()?;
        let auth = match self.auth.kind() {
            AuthType::Password => "password",
            AuthType::AppPassword => "app password",
            AuthType::OAuth2 => "oauth2/xoauth2",
        };
        Ok(format!(
            "connected to {}:{} via {} (auth={auth}), {} folders",
            self.imap.host,
            self.imap.port,
            tls_name(&self.imap.tls),
            folders.len()
        ))
    }
}

fn tls_name(mode: &TlsMode) -> &'static str {
    match mode {
        TlsMode::Tls => "implicit tls",
        TlsMode::StartTls => "starttls",
        TlsMode::None => "plaintext",
    }
}

fn parse_uid(remote_id: &str) -> Result<u32> {
    remote_id.trim().parse::<u32>().map_err(|_| {
        MailError::new(
            MailErrorCode::MessageNotFound,
            format!("`{remote_id}` is not a valid IMAP uid"),
        )
    })
}

fn parse_cursor(cursor: &str) -> (u32, u32) {
    match cursor.split_once(':') {
        Some((validity, max_uid)) => (
            validity.trim().parse().unwrap_or(0),
            max_uid.trim().parse().unwrap_or(0),
        ),
        None => (0, cursor.trim().parse().unwrap_or(0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_cursor() {
        assert_eq!(parse_cursor("42:900"), (42, 900));
    }

    #[test]
    fn parses_legacy_cursor() {
        assert_eq!(parse_cursor("900"), (0, 900));
    }

    #[test]
    fn rejects_non_numeric_uid() {
        assert!(parse_uid("AAMkAGI2").is_err());
        assert_eq!(parse_uid("17").unwrap(), 17);
    }
}
