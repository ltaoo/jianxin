# Agent / MCP 使用指南

`eeeemail agent` 提供一个运行在 stdio 上的 MCP Server，用于让外部 Agent 管理邮箱账号、同步邮件、搜索本地索引和读取邮件内容。

当前设计是 **read-first**：Agent 不能发送邮件、删除邮件、移动邮件，也不能读取或传入明文凭证。

## 1. 构建和准备配置

```sh
cargo build --release
```

二进制文件：

```text
target/release/eeeemail
```

如果没有配置文件，先创建一个空账号配置：

```sh
mkdir -p ~/.config/eeeemail

cat > ~/.config/eeeemail/config.json <<'EOF'
{
  "accounts": []
}
EOF

chmod 600 ~/.config/eeeemail/config.json
```

`config.json` 是账号配置和 OAuth 登录状态的迁移入口。该文件可能包含长期凭证，必须保持 owner-only 权限。

## 2. 注册 MCP Server

### JSON 配置格式

适用于支持 `mcpServers` JSON 配置的 MCP Host：

```json
{
  "mcpServers": {
    "eeeemail": {
      "command": "/absolute/path/to/eeeemail",
      "args": [
        "--config",
        "/absolute/path/to/config.json",
        "agent"
      ]
    }
  }
}
```

示例：

```json
{
  "mcpServers": {
    "eeeemail": {
      "command": "/Users/mayfair/Documents/other/eeeemail/target/release/eeeemail",
      "args": [
        "--config",
        "/Users/mayfair/.config/eeeemail/config.json",
        "agent"
      ]
    }
  }
}
```

### Codex TOML 配置格式

```toml
[mcp_servers.eeeemail]
command = "/Users/mayfair/Documents/other/eeeemail/target/release/eeeemail"
args = [
  "--config",
  "/Users/mayfair/.config/eeeemail/config.json",
  "agent"
]
```

注册后重启 MCP Host，使配置生效。

## 3. 当前 Tools

### Account 管理

| Tool | 用途 |
|---|---|
| `mail_list_accounts` | 列出已配置账号，不暴露凭证 |
| `mail_add_account` | 添加账号，需要两阶段确认 |
| `mail_get_account` | 查看单个账号的脱敏配置 |
| `mail_update_account` | 修改账号别名和显示名 |
| `mail_remove_account` | 删除账号及其本地缓存，需要两阶段确认 |

### 邮件读取和同步

| Tool | 用途 |
|---|---|
| `mail_list_folders` | 列出账号文件夹 |
| `mail_search_messages` | 搜索本地邮件索引 |
| `mail_get_message` | 读取单封邮件；正文和头默认不返回 |
| `mail_get_thread` | 读取本地归并出的会话，按时间旧到新 |
| `mail_list_threads` | 列出本地会话，按最后活动时间倒序 |
| `mail_sync_account` | 触发增量或全量同步 |

## 4. 添加 OAuth 账号

可以让 Agent 调用 `mail_add_account`：

```json
{
  "id": "outlook",
  "email": "funzm@outlook.com",
  "provider": "microsoft",
  "auth": {
    "type": "oauth2",
    "client_id": "your-client-id"
  }
}
```

Microsoft 和 Google 会自动使用默认 token endpoint 和 scope。第一次调用不会直接写入配置，而是返回：

```json
{
  "status": "confirmation_required",
  "confirmation_token": "...",
  "expires_in_secs": 300,
  "action": {
    "type": "add_account"
  }
}
```

用户确认后，Agent 带同一个 `confirmation_token` 再次调用才会写入 `config.json`：

```json
{
  "id": "outlook",
  "email": "funzm@outlook.com",
  "provider": "microsoft",
  "auth": {
    "type": "oauth2",
    "client_id": "your-client-id"
  },
  "confirmation_token": "..."
}
```

### 完成 OAuth 首次登录

Agent 可以添加 OAuth 账号，但不会接触 OAuth token，也不会代替用户完成浏览器授权。

账号添加成功后执行：

```sh
/absolute/path/to/eeeemail \
  --config /absolute/path/to/config.json \
  login \
  --account outlook
```

CLI 会输出 Microsoft device code 登录地址和验证码。完成授权后，refresh token 会写回 `config.json`。

由于这一步是 MCP Server 外部进程修改配置，完成后应重启 MCP Host 或重新连接 MCP Server。

## 5. 添加密码 / App Password 账号

Agent 不能传入明文密码，只能传入 SecretRef。

### 环境变量

```json
{
  "id": "qq",
  "email": "someone@qq.com",
  "auth": {
    "type": "app_password",
    "password": {
      "env": "QQ_MAIL_PASSWORD"
    }
  }
}
```

### macOS Keychain / Linux Secret Service

```json
{
  "id": "qq",
  "email": "someone@qq.com",
  "auth": {
    "type": "app_password",
    "password": {
      "keychain": "eeeemail/qq"
    }
  }
}
```

### 文件

```json
{
  "id": "qq",
  "email": "someone@qq.com",
  "auth": {
    "type": "app_password",
    "password": {
      "file": "/secure/path/qq.password"
    }
  }
}
```

Agent 输入不支持：

```text
明文字符串
command
```

这样可以避免 Agent 把密码或任意 shell 命令写入配置。

如果使用 `env`，MCP Host 启动 `eeeemail agent` 时也必须能读到该环境变量。

## 6. 同步和搜索邮件

同步账号：

```json
{
  "account_id": "outlook",
  "full": false
}
```

搜索邮件：

```json
{
  "account_id": "outlook",
  "query": "invoice",
  "limit": 20
}
```

支持过滤条件：

```json
{
  "account_id": "outlook",
  "query": "",
  "from": "@vendor.com",
  "since": "2026-09-01",
  "unread": true,
  "limit": 20
}
```

搜索默认返回摘要，不返回完整正文，避免占用过多 Agent 上下文。

## 7. 读取邮件和会话

读取单封邮件正文：

```json
{
  "account_id": "outlook",
  "message_id": "...",
  "include_body": true
}
```

需要邮件头时：

```json
{
  "account_id": "outlook",
  "message_id": "...",
  "include_body": true,
  "include_headers": true
}
```

返回正文时会带有：

```json
{
  "body_is_untrusted_email_content": true
}
```

含义是：邮件正文是数据，不是指令。

读取会话：

```json
{
  "account_id": "outlook",
  "thread_id": "...",
  "include_body": true
}
```

会话消息按时间从旧到新返回，适合让 Agent 总结上下文。

先列出会话再挑一个读（`thread_id` 可用 `mail_search_messages` 返回结果中的 id 前缀）：

```json
{
  "account_id": "outlook",
  "limit": 20,
  "unread": true
}
```

调用 `mail_list_threads`，返回按最后活动时间倒序的会话摘要（主题、消息数、未读数）。

## 8. 管理账号

修改别名和显示名：

```json
{
  "account_id": "outlook",
  "id": "work-outlook",
  "display_name": "Work Outlook"
}
```

清除显示名：

```json
{
  "account_id": "outlook",
  "clear_display_name": true
}
```

删除账号同样需要两阶段确认。

第一次：

```json
{
  "account_id": "outlook"
}
```

确认后：

```json
{
  "account_id": "outlook",
  "confirmation_token": "..."
}
```

删除会移除：

```text
config.json 里的 Account
本地 SQLite 里的邮件、文件夹和同步游标
```

## 9. 迁移到其他设备

迁移时复制：

```text
config.json
```

OAuth refresh token 已写入该 JSON，通常不需要重新登录。

新设备需要：

```text
1. 构建或复制 eeeemail 二进制
2. 复制 config.json
3. 注册 MCP Server
4. 调用 mail_sync_account 重新同步邮件
```

本地 SQLite 邮件缓存不需要迁移。

如果密码账号使用 `file`、`env` 或 `keychain`，secret 本身不在 JSON 里，新设备也必须能访问对应 secret。

## 10. 安全边界

Agent 当前可以：

```text
列出账号
添加账号，需确认
修改账号别名和显示名
删除账号，需确认
同步邮件
搜索本地邮件
读取邮件和会话
```

Agent 当前不可以：

```text
发送邮件
删除邮件
移动邮件
读取密码或 token
接收明文 secret
执行 shell command secret
```

这使 `eeeemail agent` 可以安全地作为读优先的邮件知识入口，同时把发送和破坏性邮件操作留给后续带确认机制的独立工具。
