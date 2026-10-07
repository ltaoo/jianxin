//! Well-known IMAP/SMTP endpoints.
//!
//! The architecture document's priority list is Microsoft Graph + Gmail API +
//! generic IMAP. Graph and Gmail API are Phase 2 transports; every provider
//! listed here is reachable today through generic IMAP + SMTP (163, 126, QQ,
//! aliyun, Tencent, Zoho, self-hosted, …).

use jianxin_domain::ProviderId;

use crate::{ServerConfig, TlsMode};

#[derive(Debug, Clone)]
pub struct ProviderPreset {
    pub provider: ProviderId,
    pub imap: ServerConfig,
    pub smtp: ServerConfig,
}

macro_rules! preset {
    ($provider:expr, $imap:expr, $imap_port:expr, $imap_tls:expr,
     $smtp:expr, $smtp_port:expr, $smtp_tls:expr) => {
        ProviderPreset {
            provider: $provider,
            imap: ServerConfig {
                host: $imap.to_string(),
                port: $imap_port,
                tls: $imap_tls,
                username: None,
            },
            smtp: ServerConfig {
                host: $smtp.to_string(),
                port: $smtp_port,
                tls: $smtp_tls,
                username: None,
            },
        }
    };
}

/// Domain → endpoint table.
fn table() -> Vec<(&'static str, ProviderPreset)> {
    vec![
        // --- Google -------------------------------------------------------
        (
            "gmail.com",
            preset!(
                ProviderId::Google,
                "imap.gmail.com",
                993,
                TlsMode::Tls,
                "smtp.gmail.com",
                465,
                TlsMode::Tls
            ),
        ),
        (
            "googlemail.com",
            preset!(
                ProviderId::Google,
                "imap.gmail.com",
                993,
                TlsMode::Tls,
                "smtp.gmail.com",
                465,
                TlsMode::Tls
            ),
        ),
        // --- Microsoft ---------------------------------------------------
        (
            "outlook.com",
            preset!(
                ProviderId::Microsoft,
                "outlook.office365.com",
                993,
                TlsMode::Tls,
                "smtp-mail.outlook.com",
                587,
                TlsMode::StartTls
            ),
        ),
        (
            "hotmail.com",
            preset!(
                ProviderId::Microsoft,
                "outlook.office365.com",
                993,
                TlsMode::Tls,
                "smtp-mail.outlook.com",
                587,
                TlsMode::StartTls
            ),
        ),
        (
            "live.com",
            preset!(
                ProviderId::Microsoft,
                "outlook.office365.com",
                993,
                TlsMode::Tls,
                "smtp-mail.outlook.com",
                587,
                TlsMode::StartTls
            ),
        ),
        (
            "office365.com",
            preset!(
                ProviderId::Microsoft,
                "outlook.office365.com",
                993,
                TlsMode::Tls,
                "smtp-mail.outlook.com",
                587,
                TlsMode::StartTls
            ),
        ),
        // --- China --------------------------------------------------------
        (
            "qq.com",
            preset!(
                ProviderId::GenericImap,
                "imap.qq.com",
                993,
                TlsMode::Tls,
                "smtp.qq.com",
                465,
                TlsMode::Tls
            ),
        ),
        (
            "foxmail.com",
            preset!(
                ProviderId::GenericImap,
                "imap.qq.com",
                993,
                TlsMode::Tls,
                "smtp.qq.com",
                465,
                TlsMode::Tls
            ),
        ),
        (
            "163.com",
            preset!(
                ProviderId::GenericImap,
                "imap.163.com",
                993,
                TlsMode::Tls,
                "smtp.163.com",
                465,
                TlsMode::Tls
            ),
        ),
        (
            "126.com",
            preset!(
                ProviderId::GenericImap,
                "imap.126.com",
                993,
                TlsMode::Tls,
                "smtp.126.com",
                465,
                TlsMode::Tls
            ),
        ),
        (
            "yeah.net",
            preset!(
                ProviderId::GenericImap,
                "imap.yeah.net",
                993,
                TlsMode::Tls,
                "smtp.yeah.net",
                465,
                TlsMode::Tls
            ),
        ),
        (
            "aliyun.com",
            preset!(
                ProviderId::GenericImap,
                "imap.aliyun.com",
                993,
                TlsMode::Tls,
                "smtp.aliyun.com",
                465,
                TlsMode::Tls
            ),
        ),
        (
            "sina.com",
            preset!(
                ProviderId::GenericImap,
                "imap.sina.com",
                993,
                TlsMode::Tls,
                "smtp.sina.com",
                465,
                TlsMode::Tls
            ),
        ),
        (
            "139.com",
            preset!(
                ProviderId::GenericImap,
                "imap.139.com",
                993,
                TlsMode::Tls,
                "smtp.139.com",
                465,
                TlsMode::Tls
            ),
        ),
        // --- Others -------------------------------------------------------
        (
            "icloud.com",
            preset!(
                ProviderId::GenericImap,
                "imap.mail.me.com",
                993,
                TlsMode::Tls,
                "smtp.mail.me.com",
                587,
                TlsMode::StartTls
            ),
        ),
        (
            "me.com",
            preset!(
                ProviderId::GenericImap,
                "imap.mail.me.com",
                993,
                TlsMode::Tls,
                "smtp.mail.me.com",
                587,
                TlsMode::StartTls
            ),
        ),
        (
            "zoho.com",
            preset!(
                ProviderId::GenericImap,
                "imap.zoho.com",
                993,
                TlsMode::Tls,
                "smtp.zoho.com",
                465,
                TlsMode::Tls
            ),
        ),
        (
            "yahoo.com",
            preset!(
                ProviderId::GenericImap,
                "imap.mail.yahoo.com",
                993,
                TlsMode::Tls,
                "smtp.mail.yahoo.com",
                465,
                TlsMode::Tls
            ),
        ),
        (
            "fastmail.com",
            preset!(
                ProviderId::GenericImap,
                "imap.fastmail.com",
                993,
                TlsMode::Tls,
                "smtp.fastmail.com",
                465,
                TlsMode::Tls
            ),
        ),
    ]
}

/// Look up a preset by email domain.
///
/// Level 1 of the detection chain described in the architecture document
/// ("Domain Known"). Unknown domains fall through to `generic-imap` with an
/// `imap.<domain>` guess, which is level 5 (manual/assumed configuration).
pub fn detect(email: &str) -> Option<ProviderPreset> {
    let domain = email.rsplit('@').next()?.trim().to_ascii_lowercase();
    table()
        .into_iter()
        .find(|(d, _)| *d == domain)
        .map(|(_, p)| p)
}

/// Every known domain, for `jianxin accounts --presets`.
pub fn known_domains() -> Vec<&'static str> {
    table().into_iter().map(|(d, _)| d).collect()
}
