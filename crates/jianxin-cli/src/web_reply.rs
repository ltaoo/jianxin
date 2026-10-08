//! Manual replies use the existing durable outbox; model tools cannot send.
use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use jianxin_domain::{Address, ComposeAttachment, ComposeMessage, SendResult};
use std::io::Read;

const MAX_ATTACHMENTS: usize = 20;
const MAX_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;
const MAX_REQUEST: u64 = (MAX_ATTACHMENT_BYTES * 4 / 3 + 128 * 1024) as u64;

fn original(app: &App, route: &str) -> Result<(Message, jianxin_config::AccountConfig)> {
    let id = route
        .strip_prefix("api/reply/")
        .context("invalid reply endpoint")?;
    if id.is_empty() || id.len() > 64 || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        bail!("invalid message id");
    }
    let message = app.mail().get(id)?;
    let cfg = app
        .account_configs()
        .iter()
        .find(|cfg| cfg.to_account().id == message.account_id)
        .context("unknown account")?
        .clone();
    Ok((message, cfg))
}

fn recipients(message: &Message, cfg: &jianxin_config::AccountConfig) -> Vec<Address> {
    // Replies to our own sent mail go to its recipients.
    if message
        .from
        .iter()
        .any(|a| a.email.eq_ignore_ascii_case(&cfg.email))
    {
        return message
            .to
            .iter()
            .filter(|a| !a.email.eq_ignore_ascii_case(&cfg.email))
            .cloned()
            .collect();
    }
    if message.reply_to.is_empty() {
        message.from.clone()
    } else {
        message.reply_to.clone()
    }
}

pub(super) fn prepare(app: &App, route: &str) -> Result<Reply> {
    let (message, cfg) = original(app, route)?;
    let subject = message.subject.trim();
    let subject = if subject.to_ascii_lowercase().starts_with("re:") {
        subject.to_string()
    } else {
        format!("Re: {subject}")
    };
    Ok(Reply::json(
        200,
        serde_json::json!({"from":cfg.email,"to":recipients(&message, &cfg),"cc":[],"subject":subject}),
    ))
}

pub(super) fn post(app: &App, route: &str, body: &mut dyn Read) -> Reply {
    send(app, route, body, |cfg, compose| {
        Ok(app.compose().send(cfg, compose)?)
    })
}

fn send(
    app: &App,
    route: &str,
    body: &mut dyn Read,
    deliver: impl FnOnce(&jianxin_config::AccountConfig, &ComposeMessage) -> Result<SendResult>,
) -> Reply {
    let result = (|| -> Result<(jianxin_config::AccountConfig, ComposeMessage)> {
        let mut bytes = Vec::new();
        body.take(MAX_REQUEST + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_REQUEST {
            bail!("reply too large");
        }
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Input {
            subject: String,
            body: String,
            to: Vec<Address>,
            #[serde(default)]
            cc: Vec<Address>,
            #[serde(default)]
            attachments: Vec<Upload>,
        }
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Upload {
            filename: String,
            data: String,
        }
        let input: Input = serde_json::from_slice(&bytes)?;
        if input.subject.len() > 1000
            || input.subject.contains(['\r', '\n'])
            || input.body.len() > 64000
            || (input.body.trim().is_empty() && input.attachments.is_empty())
            || input.to.is_empty()
            || input.to.len() + input.cc.len() > 100
            || input.attachments.len() > MAX_ATTACHMENTS
        {
            bail!("invalid reply content");
        }
        let (message, cfg) = original(app, route)?;
        for address in input.to.iter().chain(&input.cc) {
            if address.email.len() > 320
                || address.email.contains(|c: char| c.is_control())
                || address
                    .name
                    .as_ref()
                    .is_some_and(|name| name.len() > 256 || name.contains(|c: char| c.is_control()))
            {
                bail!("invalid recipient");
            }
        }
        let mut attachments = Vec::new();
        let mut total_bytes = 0;
        for upload in input.attachments {
            if upload.filename.is_empty()
                || upload.filename.len() > 255
                || upload
                    .filename
                    .contains(|c: char| c.is_control() || c == '/' || c == '\\')
            {
                bail!("invalid attachment filename");
            }
            let data = STANDARD.decode(&upload.data)?;
            total_bytes += data.len();
            if total_bytes > MAX_ATTACHMENT_BYTES {
                bail!("attachments too large");
            }
            let mime_type = ComposeAttachment::guess_mime(&upload.filename);
            attachments.push(ComposeAttachment::new(upload.filename, mime_type, data));
        }
        let signature = app
            .config()
            .general
            .compose
            .signature
            .as_deref()
            .unwrap_or("");
        let text = if signature.trim().is_empty() {
            input.body
        } else {
            format!("{}\n-- \n{signature}\n", input.body)
        };
        let compose = ComposeMessage {
            from: Some(Address::with_name(
                cfg.display_name.as_deref().unwrap_or(&cfg.email),
                &cfg.email,
            )),
            to: input.to,
            cc: input.cc,
            attachments,
            subject: input.subject.trim().into(),
            text_body: Some(text),
            in_reply_to: message.message_id_header.clone(),
            references: jianxin_domain::thread::build_references(&message),
            ..Default::default()
        };
        jianxin_provider::smtp::build_message(&compose, compose.from.as_ref().unwrap())?;
        Ok((cfg, compose))
    })();
    let (cfg, compose) = match result {
        Ok(value) => value,
        Err(_) => {
            return Reply::json(
                400,
                serde_json::json!({"error":"无法发送：请检查收件人、抄送和附件。主题最多 1000 字节，正文最多 64000 字节，附件最多 20 个、合计 20 MB。"}),
            )
        }
    };
    match deliver(&cfg, &compose) {
        Ok(_) => Reply::json(200, serde_json::json!({"ok":true})),
        Err(_) => Reply::json(
            502,
            serde_json::json!({"error":"发送未确认成功，内容已保留。请先检查已发送邮件及 outbox 发件队列，再决定是否重试，避免重复发送。"}),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reply_uses_owning_account_reply_to_and_thread_headers_and_validates_before_send() {
        let (app, mut message) = super::super::tests::fixture();
        message.from = vec![Address::new("sender@example.com")];
        message.reply_to = vec![Address::new("reply@example.com")];
        message.message_id_header = Some("<parent@example.com>".into());
        message.references = vec!["<root@example.com>".into()];
        jianxin_store::message_repo::upsert_many(&app.db().lock(), &[message.clone()]).unwrap();
        let route = format!("api/reply/{}", message.id);
        let prepared = prepare(&app, &route).unwrap();
        let value: serde_json::Value = serde_json::from_str(&prepared.body).unwrap();
        assert_eq!(value["from"], "test@example.com");
        assert_eq!(value["to"][0]["email"], "reply@example.com");
        assert!(value["subject"].as_str().unwrap().starts_with("Re: "));
        let response = send(
            &app,
            &route,
            &mut br#"{"subject":"Re: Meeting","body":"Thanks","to":[{"email":"edited@example.com","name":"Edited"}],"cc":[{"email":"copy@example.com","name":null}],"attachments":[{"filename":"note.txt","data":"aGk="}]}"#.as_slice(),
            |cfg, compose| {
                assert_eq!(cfg.email, "test@example.com");
                assert_eq!(compose.from.as_ref().unwrap().email, cfg.email);
                assert_eq!(compose.to, [Address::with_name("Edited", "edited@example.com")]);
                assert_eq!(compose.cc, [Address::new("copy@example.com")]);
                assert_eq!(compose.attachments[0].data, b"hi");
                assert_eq!(compose.attachments[0].mime_type, "text/plain");
                assert_eq!(compose.in_reply_to, message.message_id_header);
                assert_eq!(
                    compose.references,
                    ["<root@example.com>", "<parent@example.com>"]
                );
                Ok(SendResult::default())
            },
        );
        assert_eq!(response.status, 200);
        let upload = serde_json::json!({"filename":"note.txt","data":"aGk="});
        let mut attachment_only = serde_json::json!({"to":[{"email":"ok@example.com"}],"body":"","subject":"Test","attachments":[upload.clone()]});
        assert_eq!(
            send(
                &app,
                &route,
                &mut attachment_only.to_string().as_bytes(),
                |_, compose| {
                    assert_eq!(compose.attachments.len(), 1);
                    Ok(SendResult::default())
                }
            )
            .status,
            200
        );
        attachment_only["attachments"] = serde_json::json!(vec![upload; MAX_ATTACHMENTS + 1]);
        assert_eq!(
            send(
                &app,
                &route,
                &mut attachment_only.to_string().as_bytes(),
                |_, _| panic!("too many attachments must not send")
            )
            .status,
            400
        );
        attachment_only["attachments"] = serde_json::json!([{"filename":"too-large.bin","data":STANDARD.encode(vec![0;MAX_ATTACHMENT_BYTES+1])}]);
        assert_eq!(
            send(
                &app,
                &route,
                &mut attachment_only.to_string().as_bytes(),
                |_, _| panic!("oversized decoded attachments must not send")
            )
            .status,
            400
        );
        for bad in [
            serde_json::json!({"to":[{"email":"bad"}],"body":"Hi","subject":"Test"}),
            serde_json::json!({"to":[],"body":"Hi","subject":"Test"}),
            serde_json::json!({"to":[{"email":"ok@example.com"}],"cc":[{"email":"bad\r\nBcc: victim@example.com"}],"body":"Hi","subject":"Test"}),
            serde_json::json!({"to":[{"email":"ok@example.com"}],"body":"Hi","subject":"Test","attachments":[{"filename":"../secret.txt","data":"aGk="}]}),
            serde_json::json!({"to":[{"email":"ok@example.com"}],"body":"Hi","subject":"Test","attachments":[{"filename":"note.txt","data":"not-base64!"}]}),
        ] {
            assert_eq!(
                send(
                    &app,
                    &route,
                    &mut bad.to_string().as_bytes(),
                    |_, _| panic!("invalid recipient or attachment must not send")
                )
                .status,
                400
            );
        }
        for body in [
            br#"{"subject":"Re: Test","body":" ","to":[{"email":"ok@example.com"}]}"#.as_slice(),
            br#"{"subject":"injected\r\nBcc: victim@example.com","body":"Hi","to":[{"email":"ok@example.com"}]}"#.as_slice(),
            b"bad JSON",
        ] {
            assert_eq!(
                send(&app, &route, &mut &*body, |_, _| panic!(
                    "invalid input must not send"
                ))
                .status,
                400
            );
        }
        assert_eq!(
            send(
                &app,
                &route,
                &mut vec![b'x'; MAX_REQUEST as usize + 1].as_slice(),
                |_, _| panic!("oversized request must not send")
            )
            .status,
            400
        );
        assert_eq!(
            send(
                &app,
                &route,
                &mut br#"{"subject":"Test","body":"Hi","to":[{"email":"recipient@example.com"}]}"#
                    .as_slice(),
                |_, _| bail!("private-secret")
            )
            .status,
            502
        );
        message.from = vec![Address::new("TEST@example.com")];
        message.to = vec![
            Address::new("test@example.com"),
            Address::new("other@example.com"),
        ];
        assert_eq!(
            recipients(&message, &app.account_configs()[0]),
            [Address::new("other@example.com")]
        );
    }
}
