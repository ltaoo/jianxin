//! Bounded encrypted downloads and explicit, validated account imports.
//! Decryption happens in browser Web Crypto; no decryption key reaches HTTP.
use super::{web_agent, Reply};
use anyhow::{bail, Context, Result};
use jianxin_config::AccountConfig;
use jianxin_core::App;
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    io::Read,
    path::Path,
    time::{Duration, Instant},
};

const LIMIT: u64 = 2 * 1024 * 1024;

fn read_json(reader: &mut dyn Read) -> Result<Value> {
    let mut bytes = Vec::new();
    reader.take(LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT {
        bail!("配置不能超过 2 MB");
    }
    serde_json::from_slice(&bytes).context("配置内容不是有效 JSON")
}

pub(super) fn presets() -> Reply {
    let providers: Vec<_> = [
        ("qq", "QQ", "qq.com"),
        ("netease163", "163", "163.com"),
        ("netease126", "126", "126.com"),
        ("gmail", "Gmail", "gmail.com"),
        ("outlook", "Outlook", "outlook.com"),
        ("aliyun", "阿里云", "aliyun.com"),
    ]
    .into_iter()
    .map(|(key, name, domain)| {
        let preset = jianxin_config::presets::detect(&format!("mail@{domain}")).unwrap();
        json!({"key": key, "name": name, "platform": key, "provider": preset.provider,
            "imap": preset.imap, "smtp": preset.smtp})
    })
    .chain(std::iter::once(
        json!({"key":"custom", "name":"其他邮箱", "provider":"generic-imap"}),
    ))
    .collect();
    Reply::json(200, json!({"providers": providers}))
}

fn valid_account(account: &AccountConfig) -> bool {
    let Some((local, domain)) = account.email.split_once('@') else {
        return false;
    };
    account.validate().is_ok()
        && !local.is_empty()
        && !domain.is_empty()
        && !domain.contains('@')
        && !account.email.chars().any(char::is_whitespace)
        && [account.imap_server(), account.smtp_server()]
            .iter()
            .all(|server| {
                server.port > 0
                    && !server.host.chars().any(char::is_whitespace)
                    && ammonia::url::Host::parse(&server.host).is_ok()
            })
}

fn parsed_accounts(value: &Value) -> Result<Vec<(usize, AccountConfig)>> {
    let rows = value
        .get("accounts")
        .and_then(Value::as_array)
        .context("配置需要包含 accounts 数组")?;
    if rows.is_empty() || rows.len() > 100 {
        bail!("支持 1 至 100 个邮箱");
    }
    let mut seen = HashSet::new();
    let mut accounts = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        if let Ok(mut account) = serde_json::from_value::<AccountConfig>(row.clone()) {
            account.apply_oauth_defaults();
            if valid_account(&account) && seen.insert(account.to_account().id) {
                accounts.push((index, account));
            }
        }
    }
    if accounts.is_empty() {
        bail!("没有可导入的邮箱，请检查邮箱地址、认证信息和服务器配置");
    }
    Ok(accounts)
}

pub(super) fn preview(app: &App, reader: &mut dyn Read) -> Reply {
    let result = (|| -> Result<Value> {
        let value = read_json(reader)?;
        let accounts = parsed_accounts(&value)?;
        let verify = value["verify_connection"].as_bool().unwrap_or(false);
        if verify {
            if value["accounts"].as_array().unwrap().len() != 1 {
                bail!("每次只能验证一个邮箱");
            }
            let auth = &value["accounts"][0]["auth"];
            if auth.get("token_command").is_some()
                || auth.get("refresh_token").is_some()
                || auth
                    .as_object()
                    .is_some_and(|fields| fields.values().any(Value::is_object))
            {
                bail!("手动验证只支持直接填写密码、授权码或访问令牌");
            }
            let account = &accounts[0].1;
            if !account.needs_authorization() {
                jianxin_provider::ProviderRegistry::builtin()
                    .build(account, app.sync_config())
                    .and_then(|provider| provider.test_connection())
                    .map_err(|_| {
                        anyhow::anyhow!(
                            "邮箱连接验证失败，请检查服务器地址、TLS 设置和密码 / 客户端授权码"
                        )
                    })?;
            }
        }
        let mut warnings = Vec::new();
        let count = value["accounts"].as_array().unwrap().len();
        for index in 0..count {
            if !accounts.iter().any(|(i, _)| *i == index) {
                warnings.push(format!("第 {} 个邮箱未导入：字段无效或邮箱重复", index + 1));
            }
        }
        if value.get("general").is_some() {
            warnings.push("仅导入邮箱，保留当前桌面设置和数据目录。".into());
        }
        for (_, account) in &accounts {
            if account.needs_authorization() {
                warnings.push(format!(
                    "{} 待授权，保存后点击邮箱状态图标完成 OAuth 登录。",
                    account.email
                ));
            }
        }
        let rows: Vec<_> = accounts.into_iter().map(|(index, account)| json!({
            "index": index, "email": account.email, "name": account.display_name,
            "existing": app.account_configs().iter().any(|old| old.to_account().id == account.to_account().id),
            "needs_authorization": account.needs_authorization(),
            "connection_verified": verify && !account.needs_authorization(),
        })).collect();
        Ok(json!({"accounts": rows, "warnings": warnings}))
    })();
    reply(result)
}

pub(super) fn oauth_settings(
    manager: &mut web_agent::Manager,
    path: &Path,
    reader: &mut dyn Read,
) -> Reply {
    let result = (|| -> Result<Value> {
        let value = read_json(reader)?;
        let id = value["account"].as_str().context("请选择邮箱")?;
        let client_id = value["client_id"]
            .as_str()
            .map(str::trim)
            .filter(|id| !id.is_empty() && id.len() <= 256)
            .context("请填写 OAuth 应用 ID（Client ID）")?;
        let endpoint = value["token_endpoint"]
            .as_str()
            .context("请填写 HTTPS 令牌端点")?;
        let endpoint = validated_url(endpoint.trim())?;
        if !endpoint.path().ends_with("/token") || endpoint.query().is_some() {
            bail!("令牌端点需要以 /token 结尾，不要包含查询参数");
        }
        let mut account = jianxin_config::AppConfig::load(path)?
            .accounts
            .into_iter()
            .find(|account| account.to_account().id == id)
            .context("邮箱不存在")?;
        let jianxin_config::AuthConfig::OAuth2 {
            client_id: stored_id,
            token_endpoint,
            ..
        } = &mut account.auth
        else {
            bail!("此邮箱不使用 OAuth 登录");
        };
        *stored_id = Some(client_id.to_string());
        *token_endpoint = Some(endpoint.to_string());
        account.apply_oauth_defaults();
        manager
            .import_accounts(path, vec![account])
            .map_err(|_| anyhow::anyhow!("授权设置保存失败，请检查文件权限或稍后重试"))?;
        Ok(json!({"saved":true}))
    })();
    reply(result)
}

pub(super) fn import(
    manager: &mut web_agent::Manager,
    path: &Path,
    reader: &mut dyn Read,
) -> Reply {
    let result = (|| -> Result<Value> {
        let value = read_json(reader)?;
        let accounts = parsed_accounts(&value)?;
        if accounts.len() != value["accounts"].as_array().unwrap().len() {
            bail!("选择的邮箱包含无效或重复配置，请重新预览");
        }
        let count = accounts.len();
        manager
            .import_accounts(
                path,
                accounts.into_iter().map(|(_, account)| account).collect(),
            )
            .map_err(|_| anyhow::anyhow!("配置保存失败，请检查文件权限或稍后重试"))?;
        let daemon = match manager
            .runtime
            .config_changed(path, &manager.app().config().data_dir())
        {
            Ok(true) => "restarted",
            Ok(false) => "not_running",
            Err(_) => "failed",
        };
        Ok(json!({"imported": count, "daemon": daemon}))
    })();
    reply(result)
}

fn validated_url(value: &str) -> Result<ammonia::url::Url> {
    let url = ammonia::url::Url::parse(value).context("请输入完整 HTTPS URL")?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        bail!("请输入完整 HTTPS URL，不要包含用户名、密码或片段");
    }
    Ok(url)
}

fn download(value: &str) -> Result<String> {
    let mut target = validated_url(value)?;
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .https_only(true)
        .timeout_connect(Duration::from_secs(30))
        .timeout_read(Duration::from_secs(30))
        .timeout(Duration::from_secs(60))
        .build();
    let deadline = Instant::now() + Duration::from_secs(60);
    for hop in 0..6 {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            bail!("配置下载超时，请重试");
        }
        let response = agent
            .get(target.as_str())
            .timeout(remaining)
            .set("Cache-Control", "no-cache")
            .call()
            .map_err(|_| anyhow::anyhow!("下载失败，请检查网络、URL、文件权限和 HTTPS 证书"))?;
        if matches!(response.status(), 301 | 302 | 303 | 307 | 308) {
            if hop == 5 {
                bail!("配置 URL 重定向次数过多");
            }
            let next = target.join(response.header("Location").context("配置 URL 重定向无效")?)?;
            target = validated_url(next.as_str())?;
            continue;
        }
        if !(200..300).contains(&response.status()) {
            bail!("下载失败，请检查 URL 和文件权限");
        }
        if response
            .header("Content-Length")
            .and_then(|s| s.parse::<u64>().ok())
            .is_some_and(|n| n > LIMIT)
        {
            bail!("加密配置不能超过 2 MB");
        }
        let mut data = Vec::new();
        response
            .into_reader()
            .take(LIMIT + 1)
            .read_to_end(&mut data)
            .map_err(|_| anyhow::anyhow!("配置下载失败，请重试"))?;
        if data.len() as u64 > LIMIT {
            bail!("加密配置不能超过 2 MB");
        }
        return String::from_utf8(data).context("密文不是有效 UTF-8 文件");
    }
    bail!("配置 URL 重定向次数过多")
}

pub(super) fn source(reader: &mut dyn Read) -> Reply {
    let result = (|| -> Result<Value> {
        let value = read_json(reader)?;
        let url = value["url"].as_str().context("请输入配置 URL")?;
        Ok(json!({"ciphertext": download(url)?}))
    })();
    reply(result)
}

fn reply(result: Result<Value>) -> Reply {
    match result {
        Ok(value) => Reply::json(200, value),
        Err(error) => Reply::json(400, json!({"error": error.to_string()})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_presets_use_shared_endpoints_and_invalid_servers_are_rejected() {
        let response = presets();
        let result: Value = serde_json::from_str(&response.body).unwrap();
        assert_eq!(result["providers"].as_array().unwrap().len(), 7);
        let outlook = result["providers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["key"] == "outlook")
            .unwrap();
        let shared = jianxin_config::presets::detect("person@outlook.com").unwrap();
        assert_eq!(outlook["imap"], serde_json::to_value(shared.imap).unwrap());
        assert_eq!(outlook["smtp"], serde_json::to_value(shared.smtp).unwrap());
        let value = json!({"accounts":[{"email":"person@example.com", "auth":{"type":"app_password","password":"secret"},
            "imap":{"host":"imap.example.com","port":993},"smtp":{"host":"smtp.example.com","port":465}}]});
        assert!(parsed_accounts(&value).is_ok());
        for (field, invalid) in [
            ("email", json!("@example.com")),
            ("email", json!("a@@example.com")),
            ("email", json!("a b@example.com")),
            ("imap", json!({"host":"", "port":993})),
            ("smtp", json!({"host":"smtp.example.com", "port":0})),
            (
                "imap",
                json!({"host":"https://imap.example.com", "port":993}),
            ),
        ] {
            let mut malformed = value.clone();
            malformed["accounts"][0][field] = invalid;
            assert!(parsed_accounts(&malformed).is_err());
        }
    }

    #[test]
    fn manual_preview_never_resolves_credential_commands_or_secret_references() {
        let app = App::with_database(
            jianxin_config::AppConfig::default(),
            jianxin_store::Database::open_in_memory().unwrap(),
        )
        .unwrap();
        for auth in [
            json!({"type":"app_password","password":{"command":"do-not-execute"}}),
            json!({"type":"app_password","password":{"file":"/do-not-read"}}),
            json!({"type":"oauth2","token_command":"do-not-execute"}),
        ] {
            let value = json!({"verify_connection":true,"accounts":[{"email":"person@example.com","auth":auth}]});
            let response = preview(&app, &mut value.to_string().as_bytes());
            assert_eq!(response.status, 400);
            assert!(response.body.contains("只支持直接填写"));
            assert!(!response.body.contains("do-not-"));
            let mixed = json!({"verify_connection":true,"accounts":[{},value["accounts"][0]]});
            let response = preview(&app, &mut mixed.to_string().as_bytes());
            assert_eq!(response.status, 400);
            assert!(response.body.contains("每次只能验证一个邮箱"));
        }
    }

    #[test]
    fn manual_connection_preview_verifies_imap_without_saving_or_exposing_credentials() {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream.write_all(b"* OK IMAP4rev1 ready\r\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap() != 0 {
                let parts: Vec<_> = line.split_whitespace().collect();
                let tag = parts[0];
                match parts[1].to_ascii_uppercase().as_str() {
                    "CAPABILITY" => stream.write_all(b"* CAPABILITY IMAP4rev1\r\n").unwrap(),
                    "LIST" => stream.write_all(b"* LIST () \"/\" \"INBOX\"\r\n").unwrap(),
                    "LOGOUT" => {
                        write!(stream, "* BYE\r\n{tag} OK done\r\n").unwrap();
                        break;
                    }
                    _ => {}
                }
                write!(stream, "{tag} OK done\r\n").unwrap();
                line.clear();
            }
        });
        let app = App::with_database(
            jianxin_config::AppConfig::default(),
            jianxin_store::Database::open_in_memory().unwrap(),
        )
        .unwrap();
        let value = json!({"verify_connection":true,"accounts":[{"email":"person@example.com",
            "auth":{"type":"app_password","password":"private-password"},
            "imap":{"host":"127.0.0.1","port":port,"tls":"none"},"smtp":{"host":"smtp.example.com","port":465}}]});
        let response = preview(&app, &mut value.to_string().as_bytes());
        server.join().unwrap();
        assert_eq!(response.status, 200, "{}", response.body);
        assert!(app.account_configs().is_empty());
        assert!(!response.body.contains("private-password"));
        let rows: Value = serde_json::from_str(&response.body).unwrap();
        assert_eq!(rows["accounts"][0]["connection_verified"], true);
        let response = preview(&app, &mut value.to_string().as_bytes());
        assert_eq!(response.status, 400);
        assert!(!response.body.contains("private-password"));
        assert!(app.account_configs().is_empty());
        let pending = json!({"verify_connection":true,"accounts":[{"email":"pending@outlook.com","auth":{"type":"oauth2"}}]});
        let response = preview(&app, &mut pending.to_string().as_bytes());
        assert_eq!(response.status, 200);
        let rows: Value = serde_json::from_str(&response.body).unwrap();
        assert_eq!(rows["accounts"][0]["connection_verified"], false);
        assert_eq!(rows["accounts"][0]["needs_authorization"], true);
    }
    #[test]
    fn url_validation_and_bounded_input() {
        assert!(validated_url("https://example.com/config.enc?v=1").is_ok());
        for url in [
            "http://example.com/a",
            "file:///tmp/a",
            "https://user:password@example.com/a",
            "https://example.com/a#key",
        ] {
            assert!(validated_url(url).is_err());
        }
        assert!(read_json(&mut &vec![b' '; LIMIT as usize + 1][..]).is_err());
    }
    #[test]
    fn preview_validates_accounts_and_omits_credentials() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../../mobile/tests/fixtures/config.json"))
                .unwrap();
        let mut value: Value =
            serde_json::from_str(fixture["plaintext"].as_str().unwrap()).unwrap();
        value["accounts"]
            .as_array_mut()
            .unwrap()
            .push(json!({"email":"invalid"}));
        let app = App::with_database(
            jianxin_config::AppConfig::default(),
            jianxin_store::Database::open_in_memory().unwrap(),
        )
        .unwrap();
        let response = preview(&app, &mut value.to_string().as_bytes());
        assert_eq!(response.status, 200);
        assert!(!response.body.contains("sample-password"));
        assert!(!response.body.contains("authorization-code"));
        let result: Value = serde_json::from_str(&response.body).unwrap();
        assert_eq!(result["accounts"].as_array().unwrap().len(), 2);
        assert_eq!(result["warnings"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn preview_retains_passwordless_oauth_and_marks_it_pending() {
        let app = App::with_database(
            jianxin_config::AppConfig::default(),
            jianxin_store::Database::open_in_memory().unwrap(),
        )
        .unwrap();
        let response = preview(
            &app,
            &mut &br#"{"accounts":[
            {"email":"pending@outlook.com","auth":{"type":"oauth2"}},
            {"email":"other@example.com","auth":{"type":"o_auth2"}},
            {"email":"password@example.com","auth":{"type":"password"}}
        ]}"#[..],
        );
        assert_eq!(response.status, 200);
        let result: Value = serde_json::from_str(&response.body).unwrap();
        assert_eq!(result["accounts"].as_array().unwrap().len(), 2);
        assert!(result["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|account| account["needs_authorization"] == true));
        assert!(result["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning
                .as_str()
                .unwrap()
                .contains("pending@outlook.com 待授权")));
    }
}
