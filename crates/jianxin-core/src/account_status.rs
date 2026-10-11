//! Last sync outcome, shared by the watcher, TUI and Web reader.
use jianxin_config::AppConfig;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountSyncStatus {
    Synced,
    Local,
    NotSynced,
    LoginFailed,
    #[serde(rename = "oauth_required")]
    OAuthRequired,
    MissingCredentials,
    SyncFailed,
}

impl AccountSyncStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Synced => "已同步",
            Self::Local => "已有本地邮件",
            Self::NotSynced => "未同步",
            Self::LoginFailed => "登录失败：更新客户端授权码",
            Self::OAuthRequired => "等待 OAuth 登录",
            Self::MissingCredentials => "缺少凭据：配置客户端授权码",
            Self::SyncFailed => "同步失败",
        }
    }

    pub fn tui_label(self) -> &'static str {
        match self {
            Self::NotSynced => "未同步：按 f 获取邮件",
            Self::SyncFailed => "同步失败：按 f 重试",
            other => other.label(),
        }
    }

    pub fn tone(self) -> &'static str {
        match self {
            Self::Synced => "success",
            Self::Local => "neutral",
            Self::NotSynced | Self::OAuthRequired => "warning",
            Self::LoginFailed | Self::MissingCredentials | Self::SyncFailed => "error",
        }
    }
}

pub fn read_account_sync_status(
    config: &AppConfig,
    account_id: &str,
    has_local_folders: bool,
) -> AccountSyncStatus {
    if config.accounts.iter().any(|account| account.to_account().id == account_id && account.needs_authorization()) {
        return AccountSyncStatus::OAuthRequired;
    }
    let path = config
        .data_dir()
        .join("account-status")
        .join(format!("{account_id}.txt"));
    // Only return known status text. Never expose an arbitrary error/credential
    // accidentally written into the status file.
    match std::fs::read_to_string(path).as_deref().map(str::trim) {
        Ok("已同步") => AccountSyncStatus::Synced,
        Ok("登录失败：更新客户端授权码") => AccountSyncStatus::LoginFailed,
        Ok("等待 OAuth 登录") => AccountSyncStatus::OAuthRequired,
        Ok("缺少凭据：配置客户端授权码") => AccountSyncStatus::MissingCredentials,
        Ok(_) => AccountSyncStatus::SyncFailed,
        Err(_) if has_local_folders => AccountSyncStatus::Local,
        Err(_) => AccountSyncStatus::NotSynced,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oauth_without_credentials_is_pending_immediately_before_any_sync() {
        assert_eq!(serde_json::to_value(AccountSyncStatus::OAuthRequired).unwrap(), "oauth_required");
        let directory = std::env::temp_dir().join(format!("jianxin-pending-status-{}", jianxin_domain::new_id()));
        let mut config: AppConfig = serde_json::from_str(r#"{"accounts":[{"email":"pending@outlook.com","auth":{"type":"oauth2"}}]}"#).unwrap();
        config.general.data_dir = Some(directory.clone());
        let id = config.accounts[0].to_account().id;
        assert_eq!(read_account_sync_status(&config, &id, false), AccountSyncStatus::OAuthRequired);
        crate::record_account_sync_status(&config, &id, None);
        assert_eq!(read_account_sync_status(&config, &id, true), AccountSyncStatus::OAuthRequired);
        let jianxin_config::AuthConfig::OAuth2 { access_token, .. } = &mut config.accounts[0].auth else { panic!("expected oauth2"); };
        *access_token = Some(jianxin_config::SecretRef::Plain("valid-token".into()));
        assert_eq!(read_account_sync_status(&config, &id, true), AccountSyncStatus::Synced);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn shares_watcher_outcomes_with_both_interfaces_without_exposing_raw_errors() {
        let directory =
            std::env::temp_dir().join(format!("jianxin-status-{}", jianxin_domain::new_id()));
        let mut config = AppConfig::default();
        config.general.data_dir = Some(directory.clone());
        assert_eq!(
            read_account_sync_status(&config, "test", false),
            AccountSyncStatus::NotSynced
        );
        assert_eq!(
            read_account_sync_status(&config, "test", true),
            AccountSyncStatus::Local
        );
        for (error, expected) in [
            (None, AccountSyncStatus::Synced),
            (
                Some("AUTHENTICATION_FAILED secret-password"),
                AccountSyncStatus::LoginFailed,
            ),
            (
                Some("AUTHENTICATION_REQUIRED oauth2"),
                AccountSyncStatus::OAuthRequired,
            ),
            (
                Some("AUTHENTICATION_REQUIRED"),
                AccountSyncStatus::MissingCredentials,
            ),
            (
                Some("network failure secret-password"),
                AccountSyncStatus::SyncFailed,
            ),
        ] {
            crate::record_account_sync_status(&config, "test", error);
            let status = read_account_sync_status(&config, "test", false);
            assert_eq!(status, expected);
            assert!(!status.label().contains("secret-password"));
        }
        std::fs::write(
            directory.join("account-status/test.txt"),
            "unexpected-secret-password",
        )
        .unwrap();
        assert_eq!(
            read_account_sync_status(&config, "test", false),
            AccountSyncStatus::SyncFailed
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
