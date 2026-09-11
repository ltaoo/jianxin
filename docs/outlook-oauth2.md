# Outlook.com OAuth2 收发信验证与接入说明

本文记录 `funzm@outlook.com` consumer 邮箱的可用 OAuth2 方案，以及端到端 IMAP/SMTP 验证结果。

## 结论

Outlook.com / Microsoft consumer 邮箱应使用：

```text
Delegated OAuth 2.0
  + Device Code Flow 或 Authorization Code + PKCE
  + IMAP/SMTP
  + SASL XOAUTH2
```

不要使用 Azure App 的 `client_credentials` / app-only 模式访问个人 Outlook.com 邮箱。

已验证可用的客户端配置：

```text
Client ID: 24f078c3-3cad-4b6f-8c6e-0c91e0990af7
Authority: https://login.microsoftonline.com/consumers
Flow: Device Code Flow
```

不要在文档、配置或聊天中保存 client secret。

## Azure App 配置

### Supported account types

在 Azure Portal：

```text
Microsoft Entra ID
  → App registrations
  → 应用
  → Authentication
  → Supported account types
```

选择以下任一项：

```text
Accounts in any organizational directory and personal Microsoft accounts
```

或：

```text
Personal Microsoft accounts only
```

如果 token 请求返回：

```text
AADSTS700016: Application ... was not found
```

通常表示应用未开放给 personal Microsoft accounts。

### Public client

对本机 GUI/TUI 客户端，在 Authentication 页面开启：

```text
Allow public client flows = Yes
```

Device Code Flow 不需要 redirect URI。

### API permissions

添加 Delegated permissions，不要选 Application permissions：

```text
APIs my organization uses
  → Office 365 Exchange Online
  → Delegated permissions
  → IMAP.AccessAsUser.All
  → SMTP.Send
```

个人账号通常在登录时用户 consent，不需要管理员 consent。

以下权限适用于组织 Exchange Online 邮箱的 app-only 模式，不适用于 Outlook.com：

```text
IMAP.AccessAsApp
POP.AccessAsApp
SMTP.SendAsApp
```

## OAuth scopes

IMAP 收信和 SMTP 发信使用：

```text
offline_access
https://outlook.office.com/IMAP.AccessAsUser.All
https://outlook.office.com/SMTP.Send
```

如需 POP，额外添加：

```text
https://outlook.office.com/POP.AccessAsUser.All
```

## Token endpoints

仅 consumer 账号：

```text
https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode
https://login.microsoftonline.com/consumers/oauth2/v2.0/token
```

同时支持组织账号和个人账号：

```text
https://login.microsoftonline.com/common/oauth2/v2.0/devicecode
https://login.microsoftonline.com/common/oauth2/v2.0/token
```

## Device Code Flow

### 1. 发起授权

```sh
curl -sS -X POST \
  https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode \
  -H 'Content-Type: application/x-www-form-urlencoded' \
  --data-urlencode 'client_id=<CLIENT_ID>' \
  --data-urlencode 'scope=offline_access https://outlook.office.com/IMAP.AccessAsUser.All https://outlook.office.com/SMTP.Send'
```

响应包含：

```text
user_code
device_code
verification_uri
expires_in
interval
```

提示用户打开：

```text
https://www.microsoft.com/link
```

输入 `user_code`，用 Outlook.com 账号登录并授权。

### 2. 轮询 token endpoint

```sh
curl -sS -X POST \
  https://login.microsoftonline.com/consumers/oauth2/v2.0/token \
  -H 'Content-Type: application/x-www-form-urlencoded' \
  --data-urlencode 'grant_type=urn:ietf:params:oauth:grant-type:device_code' \
  --data-urlencode 'client_id=<CLIENT_ID>' \
  --data-urlencode 'device_code=<DEVICE_CODE>' \
  --data-urlencode 'scope=offline_access https://outlook.office.com/IMAP.AccessAsUser.All https://outlook.office.com/SMTP.Send'
```

成功响应包含：

```text
access_token
refresh_token
token_type
expires_in
scope
```

`refresh_token` 可以写入 `config.json`，便于把一个 JSON 文件迁移到其他设备；该文件在 Unix 上会设置为 `0600`。如果不想让 token 进入明文配置，也可改用 `file` / `env` / `command` / `keychain` 形式的 `SecretRef`。

## IMAP / SMTP 设置

### 本项目 token 配置

`config.json` 可直接配置 refresh token，provider 会在连接前自动刷新并缓存 access token：

```json
{
  "email": "funzm@outlook.com",
  "auth": {
    "type": "oauth2",
    "client_id": "24f078c3-3cad-4b6f-8c6e-0c91e0990af7",
    "token_endpoint": "https://login.microsoftonline.com/consumers/oauth2/v2.0/token"
  }
}
```

首次 `eeeemail login` 成功后，CLI 会把返回的 `refresh_token` 以字符串形式写回这个账号的 JSON 节点。access token 过期前 60 秒会主动刷新；如果服务商轮转 refresh token，新 token 也会写回 `config.json`。

## CLI 使用

首次登录：

```sh
eeeemail login --account funzm@outlook.com
```

CLI 会显示 Microsoft device code 登录地址和验证码，并把 refresh token 写入 `config.json`。文件在 Unix 上会设置为 `0600`；迁移设备时只需要复制这个 JSON 文件。

收信：

```sh
eeeemail sync --account funzm@outlook.com
eeeemail list --account funzm@outlook.com inbox
eeeemail read --account funzm@outlook.com <message-id>
```

发信：

```sh
eeeemail send \
  --account funzm@outlook.com \
  --to funzm@outlook.com \
  --subject 'CLI OAuth test' \
  --body 'Sent by eeeemail CLI'
```

`sync`、`send`、`doctor` 等需要连网的命令会在建立 IMAP/SMTP 连接前刷新 access token，不需要单独执行 refresh 命令。

Outlook.com 官方设置：

| 协议 | Host | Port | TLS | 认证 |
|---|---|---:|---|---|
| IMAP | `outlook.office365.com` | 993 | SSL/TLS | OAuth2 / XOAUTH2 |
| POP | `outlook.office365.com` | 995 | SSL/TLS | OAuth2 / XOAUTH2 |
| SMTP | `smtp-mail.outlook.com` | 587 | STARTTLS | OAuth2 / XOAUTH2 |

用户名是完整邮箱地址。

Outlook.com 可能默认关闭 IMAP/POP。用户需要在 Outlook Web 设置中开启：

```text
Settings
  → Mail
  → Forwarding and IMAP
  → Let devices and apps use IMAP
```

## XOAUTH2 格式

原始 SASL 字符串：

```text
user=<EMAIL>\x01auth=Bearer <ACCESS_TOKEN>\x01\x01
```

协议传输时需要 base64。

IMAP：

```text
AUTHENTICATE XOAUTH2 <BASE64>
```

SMTP：

```text
AUTH XOAUTH2 <BASE64>
```

### Python 注意事项

Python `imaplib.authenticate()` 和 `smtplib.auth()` 会对 callback 返回值再次 base64 编码。因此 callback 应返回原始字符串，而不是预编码后的 base64：

```python
raw_sasl = f"user={email}\x01auth=Bearer {access_token}\x01\x01"
```

如果提前 base64，会被编码两次，Outlook IMAP 会返回：

```text
BAD Command Argument Error. 12
```

## Microsoft Graph 备选方案

Graph API 也可访问个人 Outlook 邮箱，但必须使用 delegated OAuth：

```text
offline_access
https://graph.microsoft.com/Mail.ReadWrite
https://graph.microsoft.com/Mail.Send
```

常用 API：

```text
GET  https://graph.microsoft.com/v1.0/me/messages
POST https://graph.microsoft.com/v1.0/me/sendMail
```

注意 token audience 不能混用：

```text
Graph token      → Graph API
Outlook token    → IMAP/SMTP
```

当前项目已有通用 IMAP/SMTP 层，因此最短实现路径是 delegated OAuth + IMAP/SMTP + XOAUTH2。

## 2026-09-10 端到端验证结果

使用上述 Client ID 和 Device Code Flow 获取 token：

```text
token_type: Bearer
expires_in: 3599
scope:
  https://outlook.office.com/IMAP.AccessAsUser.All
  https://outlook.office.com/SMTP.Send
```

### IMAP

```text
IMAP authenticate: OK
IMAP select INBOX: OK
INBOX message count: 8
```

### SMTP

测试邮件：

```text
Subject: eeeemail OAuth test e80cb318
From: funzm@outlook.com
To: funzm@outlook.com
Body: This is an end-to-end IMAP/SMTP OAuth2 test for eeeemail.
```

结果：

```text
SMTP XOAUTH2 send: OK
```

随后通过 IMAP 在 INBOX 中成功搜索并读取同一封邮件：

```text
IMAP after-send fetch: OK
Subject: eeeemail OAuth test e80cb318
From: funzm@outlook.com
Body: This is an end-to-end IMAP/SMTP OAuth2 test for eeeemail.
```

因此确认：

```text
Outlook.com consumer 邮箱 delegated OAuth2 + IMAP/SMTP + XOAUTH2 可用
```

## 参考资料

- [Authenticate an IMAP, POP or SMTP connection using OAuth](https://learn.microsoft.com/en-us/exchange/client-developer/legacy-protocols/how-to-authenticate-an-imap-pop-smtp-application-by-using-oauth)
- [POP, IMAP, and SMTP settings for Outlook.com](https://support.microsoft.com/en-us/office/pop-imap-and-smtp-settings-for-outlook-com-d088b986-291d-42b8-9564-9c414e2aa040)
- [Microsoft Graph Outlook mail API overview](https://learn.microsoft.com/en-us/graph/api/resources/mail-api-overview?view=graph-rest-1.0)
