use anyhow::Result;
use jianxin_core::App;

pub const LIST_LIMIT: u32 = 10;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MailRow {
    pub id: String,
    pub subject: String,
    pub timestamp: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub total: u64,
    pub show_unread_count: bool,
    pub rows: Vec<MailRow>,
    pub error: Option<String>,
    pub loaded: bool,
    pub mailboxes: Vec<MailboxRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MailboxRow {
    pub id: String,
    pub email: String,
    pub last_sync_at: Option<i64>,
    pub status: String,
}

pub fn load_snapshot(app: &App) -> Result<Snapshot> {
    let total = app.mail().count_all_unread()?;
    let rows = app
        .mail()
        .list_all(LIST_LIMIT, 0, true)?
        .into_iter()
        .map(|message| MailRow {
            timestamp: message.sent_at.or(message.received_at),
            id: message.id,
            subject: if message.subject.trim().is_empty() {
                "（无主题）".into()
            } else {
                message.subject
            },
        })
        .collect();
    Ok(Snapshot {
        total,
        show_unread_count: app.config().general.tray_show_unread_count,
        rows,
        loaded: true,
        error: None,
        mailboxes: app
            .account_configs()
            .iter()
            .map(|cfg| {
                let account = cfg.to_account();
                let last = jianxin_store::sync_log_repo::latest(&app.db().lock(), &account.id)?;
                let status = jianxin_core::read_account_sync_status(
                    app.config(),
                    &account.id,
                    !app.mail().folders(&account.id)?.is_empty(),
                );
                Ok(MailboxRow {
                    id: account.id,
                    email: account.email,
                    last_sync_at: last.map(|entry| entry.completed_at),
                    status: status.label().into(),
                })
            })
            .collect::<Result<Vec<_>>>()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use jianxin_config::AppConfig;
    use jianxin_domain::{Folder, FolderKind, Message};
    use jianxin_store::{folder_repo, message_repo, Database};

    #[test]
    fn merged_unread_is_latest_bounded_deduplicated_and_read_only() {
        let config: AppConfig = serde_json::from_value(serde_json::json!({"accounts": [
            {"email":"a@example.com","auth":{"type":"password","password":"unused"}},
            {"email":"b@example.com","auth":{"type":"password","password":"unused"}}
        ]}))
        .unwrap();
        let app = App::with_database(config, Database::open_in_memory().unwrap()).unwrap();
        let accounts = app.accounts().unwrap();
        for (index, account) in accounts.iter().enumerate() {
            let mut folder = Folder::new(&account.id, "INBOX", "INBOX", FolderKind::Inbox);
            folder.id = format!("folder-{index}");
            folder_repo::upsert_many(&app.db().lock(), std::slice::from_ref(&folder)).unwrap();
            for i in 0..20 {
                let message = Message {
                    id: format!("{index}-{i}"),
                    account_id: account.id.clone(),
                    folder_id: folder.id.clone(),
                    remote_id: i.to_string(),
                    subject: format!("Mail {index}-{i}"),
                    sent_at: Some(1000 + i * 2 + index as i64),
                    received_at: Some(2000 + i * 2 + index as i64),
                    ..Message::default()
                };
                message_repo::upsert_many(&app.db().lock(), std::slice::from_ref(&message))
                    .unwrap();
            }
            let mut read = app.mail().get(&format!("{index}-0")).unwrap();
            read.id = format!("read-{index}");
            read.remote_id = "read".into();
            read.is_read = true;
            message_repo::upsert_many(&app.db().lock(), std::slice::from_ref(&read)).unwrap();
            let mut deleted = read.clone();
            deleted.id = format!("deleted-{index}");
            deleted.remote_id = "deleted".into();
            deleted.is_read = false;
            deleted.is_deleted = true;
            message_repo::upsert_many(&app.db().lock(), std::slice::from_ref(&deleted)).unwrap();
        }
        let original = app.mail().get("0-19").unwrap();
        let mut original = original;
        original.message_id_header = Some("<duplicate>".into());
        message_repo::upsert_many(&app.db().lock(), std::slice::from_ref(&original)).unwrap();
        let archive = Folder::new(&accounts[0].id, "Archive", "Archive", FolderKind::Archive);
        folder_repo::upsert_many(&app.db().lock(), std::slice::from_ref(&archive)).unwrap();
        let mut copy = original;
        copy.id = "copy".into();
        copy.folder_id = archive.id;
        message_repo::upsert_many(&app.db().lock(), std::slice::from_ref(&copy)).unwrap();
        let snapshot = load_snapshot(&app).unwrap();
        assert_eq!(snapshot.total, 40);
        assert_eq!(snapshot.rows.len(), LIST_LIMIT as usize);
        assert_eq!(snapshot.rows[0].id, "1-19");
        assert_eq!(snapshot.rows[0].timestamp, Some(1039));
        assert!(!snapshot.rows.iter().any(|row| row.id == "copy"));
        assert_eq!(app.mail().count_all_unread().unwrap(), 40);
        let mut latest = app.mail().get("1-19").unwrap();
        latest.sent_at = None;
        latest.received_at = Some(1039);
        message_repo::upsert_many(&app.db().lock(), &[latest]).unwrap();
        assert_eq!(load_snapshot(&app).unwrap().rows[0].timestamp, Some(1039));
    }
}
