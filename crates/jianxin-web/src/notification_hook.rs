//! Authenticated loopback hook; authorization and concurrency are enforced by Web.
use std::io::Read;

use serde::Deserialize;
use serde_json::{json, Value};
use subtle::ConstantTimeEq;

use super::Reply;

const MAX_BODY: usize = 32 * 1024;

pub(super) fn authorize(
    request: &tiny_http::Request,
    expected: Option<&str>,
    host: &str,
) -> Option<Reply> {
    let header = |name| {
        request
            .headers()
            .iter()
            .find(|header| header.field.equiv(name))
            .map(|header| header.value.as_str())
    };
    if header("Host") != Some(host)
        || header("Origin").is_some_and(|origin| origin != format!("http://{host}"))
    {
        return Some(Reply::json(403, json!({"error": "请求来源无效"})));
    }
    let count = request
        .headers()
        .iter()
        .filter(|header| header.field.equiv("Authorization"))
        .count();
    if count != 1 || !token_authorized(expected, header("Authorization")) {
        return Some(Reply::json(
            401,
            json!({"error": "需要有效的 Authorization: Bearer token；请检查 general.api.token 配置"}),
        ));
    }
    None
}

fn token_authorized(expected: Option<&str>, authorization: Option<&str>) -> bool {
    let Some(expected) = expected.filter(|token| !token.is_empty()) else {
        return false;
    };
    let Some((scheme, token)) = authorization.and_then(|value| value.split_once(' ')) else {
        return false;
    };
    scheme.eq_ignore_ascii_case("Bearer") && bool::from(token.as_bytes().ct_eq(expected.as_bytes()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Notification {
    title: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    subtitle: Option<String>,
    #[serde(default)]
    id: Option<String>,
}

pub(super) fn handle(request: &mut tiny_http::Request, runtime: &dyn crate::RuntimeHooks) -> Reply {
    let content_type = request
        .headers()
        .iter()
        .find(|header| header.field.equiv("Content-Type"))
        .map(|header| header.value.as_str().to_owned());
    let mut reader = request.as_reader();
    deliver(&mut reader, content_type.as_deref(), |payload| {
        runtime.notify(payload)
    })
}

fn deliver(
    reader: &mut impl Read,
    content_type: Option<&str>,
    send: impl FnOnce(&Value) -> anyhow::Result<()>,
) -> Reply {
    if !content_type.is_some_and(|value| {
        value
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .eq_ignore_ascii_case("application/json")
    }) {
        return Reply::json(415, json!({"error": "请使用 application/json"}));
    }
    let mut bytes = Vec::new();
    if reader
        .take((MAX_BODY + 1) as u64)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return Reply::json(400, json!({"error": "无法读取请求内容"}));
    }
    if bytes.len() > MAX_BODY {
        return Reply::json(413, json!({"error": "请求内容超过 32 KiB"}));
    }
    let Ok(input) = serde_json::from_slice::<Notification>(&bytes) else {
        return Reply::json(
            400,
            json!({"error": "JSON 参数无效；支持 title、body、subtitle、id"}),
        );
    };
    let title = input.title.trim();
    let subtitle = input.subtitle.as_deref().unwrap_or("简信").trim();
    let id = input
        .id
        .unwrap_or_else(|| format!("jianxin-hook-{}", uuid::Uuid::new_v4()));
    let valid_line = |value: &str, max: usize| {
        value.chars().count() <= max && !value.chars().any(char::is_control)
    };
    if title.is_empty()
        || !valid_line(title, 120)
        || !valid_line(subtitle, 120)
        || id.trim().is_empty()
        || !valid_line(&id, 128)
        || input.body.chars().count() > 4096
        || input
            .body
            .chars()
            .any(|ch| ch.is_control() && ch != '\n' && ch != '\t')
    {
        return Reply::json(
            400,
            json!({"error": "title 必须为 1–120 字；subtitle 最多 120 字，body 最多 4096 字，id 为 1–128 字；不支持控制字符"}),
        );
    }
    let payload =
        json!({"id": id, "title": title, "subtitle": subtitle, "body": input.body, "thread": ""});
    match send(&payload) {
        Ok(()) => Reply::json(202, json!({"accepted": true, "id": id})),
        Err(error) => {
            eprintln!("notification hook delivery failed: {error:#}");
            Reply::json(
                503,
                json!({"accepted": false, "error": "系统通知投递失败；请检查 Jianxin.app 是否安装及系统通知权限"}),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_token_is_required_and_absent_or_empty_configuration_denies_access() {
        assert!(token_authorized(
            Some("secret-token"),
            Some("Bearer secret-token")
        ));
        assert!(token_authorized(
            Some("secret-token"),
            Some("bearer secret-token")
        ));
        for header in [
            None,
            Some(""),
            Some("secret-token"),
            Some("Basic secret-token"),
            Some("Bearer wrong-token"),
            Some("Bearer secret-token "),
            Some("Bearer  secret-token"),
        ] {
            assert!(!token_authorized(Some("secret-token"), header));
        }
        for config in [None, Some("")] {
            assert!(!token_authorized(config, Some("Bearer secret-token")));
            assert!(!token_authorized(config, Some("Bearer ")));
        }
    }

    fn request(value: Value, send: impl FnOnce(&Value) -> anyhow::Result<()>) -> Reply {
        deliver(
            &mut value.to_string().as_bytes(),
            Some("application/json; charset=utf-8"),
            send,
        )
    }

    #[test]
    fn valid_hook_uses_native_payload_and_returns_generated_or_supplied_id() {
        let reply = request(
            json!({"title":" 外部任务完成 ","body":"第一行\n第二行","subtitle":"自动化","id":"external-job-1"}),
            |payload| {
                assert_eq!(payload["title"], "外部任务完成");
                assert_eq!(payload["body"], "第一行\n第二行");
                assert_eq!(payload["subtitle"], "自动化");
                assert_eq!(payload["thread"], "");
                Ok(())
            },
        );
        assert_eq!(reply.status, 202);
        assert_eq!(
            serde_json::from_str::<Value>(&reply.body).unwrap()["id"],
            "external-job-1"
        );
        let reply = request(json!({"title":"任务完成"}), |payload| {
            assert_eq!(payload["subtitle"], "简信");
            assert!(payload["id"].as_str().unwrap().starts_with("jianxin-hook-"));
            Ok(())
        });
        assert_eq!(reply.status, 202);
    }

    #[test]
    fn invalid_payloads_never_invoke_system_notifications() {
        for value in [
            json!({}),
            json!({"title":" "}),
            json!({"title":123}),
            json!({"title":"hello","url":"https://example.com"}),
            json!({"title":"a".repeat(121)}),
            json!({"title":"hello","body":"a".repeat(4097)}),
            json!({"title":"hello","id":""}),
            json!({"title":"hello","subtitle":"bad\nsubtitle"}),
            json!({"title":"hello","body":"bad\u{0000}body"}),
        ] {
            assert_eq!(
                request(value, |_| panic!("invalid request must not notify")).status,
                400
            );
        }
        assert_eq!(
            deliver(&mut b"{}".as_slice(), Some("text/plain"), |_| panic!()).status,
            415
        );
        assert_eq!(
            deliver(&mut b"{".as_slice(), Some("application/json"), |_| panic!()).status,
            400
        );
        assert_eq!(
            deliver(
                &mut vec![b' '; MAX_BODY + 1].as_slice(),
                Some("application/json"),
                |_| panic!()
            )
            .status,
            413
        );
    }

    #[test]
    fn delivery_failure_is_not_reported_as_success_or_leaked_to_client() {
        let reply = request(json!({"title":"任务完成"}), |_| {
            anyhow::bail!("private system error")
        });
        assert_eq!(reply.status, 503);
        assert!(!reply.body.contains("private system error"));
        assert_eq!(
            serde_json::from_str::<Value>(&reply.body).unwrap()["accepted"],
            false
        );
    }
}
