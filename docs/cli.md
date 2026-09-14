# eeeemail CLI 使用说明

`eeeemail` 是一个多厂商终端邮件客户端。Phase 1 先提供 CLI：在 JSON 里配置邮箱 → 同步到本地 SQLite → 在命令行读信、发信，需要交互时进入 TUI。

## 概览

```text
eeeemail [全局选项] <子命令> [子命令选项]
```

不带子命令时默认进入 TUI。

```sh
eeeemail --help           # 列出全部子命令
eeeemail <子命令> --help   # 查看某个子命令的完整参数
```

## 全局选项

所有子命令都接受以下选项：

| 选项 | 说明 |
|------|------|
| `--config <PATH>` | 配置文件路径。默认 `$EEEEMAIL_CONFIG`，否则平台配置目录下的 `config.json` |
| `-a, --account <ACCOUNT>` | 选择邮箱：`id` 别名、完整邮箱地址，或 `id` 前缀 |
| `--json` | 输出机器可读 JSON |
| `-v, --verbose...` | 详细日志，可重复累加 |
| `-h, --help` | 帮助 |
| `-V, --version` | 版本 |

程序启动时会读取当前目录下的 `.env`（`dotenvy`），因此 `SecretRef` 里的 `env` 密码可以放在那里；进程已有环境变量始终优先。

## 配置

账号和登录态都写在 `config.json`（写入时权限设为 owner-only）。最小示例：

```json
{
  "accounts": [
    {
      "id": "gmail",
      "email": "you@gmail.com",
      "auth": { "type": "app_password", "password": { "env": "GMAIL_PWD" } }
    }
  ]
}
```

密钥用 `SecretRef` 引用，不落明文：`{ "env": "NAME" }`、`{ "keychain": "service" }`、`{ "command": "..." }`、`{ "file": "path" }`，或调试用的裸字符串。

IMAP/SMTP 端点解析顺序：显式 `imap`/`smtp` 字段 → 域名预设（`gmail.com`、`qq.com`、`aliyun.com` 等）→ `imap.<domain>` 兜底猜测。企业邮箱（如阿里云企业邮箱 `qiye.aliyun.com`）需要显式写端点：

```json
{
  "id": "funzmnoreply",
  "email": "noreply@funzm.com",
  "provider": "generic-imap",
  "auth": { "type": "app_password", "password": { "env": "FUNZM_NOREPLY_PWD" } },
  "imap": { "host": "imap.qiye.aliyun.com", "port": 993, "tls": "tls" },
  "smtp": { "host": "smtp.qiye.aliyun.com", "port": 465, "tls": "tls" }
}
```

## 命令参考

### `init`

写入示例配置文件。

```sh
eeeemail init                # 写到平台默认路径
eeeemail init --path ./config.json
eeeemail init --force        # 覆盖已有文件
```

### `accounts`

列出已配置的账号。

```sh
eeeemail accounts
```

### `doctor`

检查每个账号（或 `-a` 指定的那一个）的连通性和凭据。

```sh
eeeemail doctor
eeeemail doctor -a noreply@funzm.com
```

任一账号失败时以非零退出码结束。

### `sync`

把邮件增量拉取到本地数据库。

```sh
eeeemail sync                  # 同步全部账号
eeeemail sync -a gmail         # 只同步一个账号
eeeemail sync --full           # 忽略游标，重新拉取
eeeemail sync --folder inbox   # 只同步某个文件夹
```

### `watch`

常驻轮询新邮件。详见 [watch.md](watch.md)。

```sh
eeeemail watch
eeeemail watch -a gmail --interval 60
```

### `folders`

列出文件夹。

```sh
eeeemail folders
eeeemail folders --refresh     # 远程重新列出，而非读本地副本
```

### `list`

列出某文件夹的邮件，默认 `inbox`。

```sh
eeeemail list                    # inbox 前 20 封
eeeemail list sent               # 指定文件夹
eeeemail list -n 50              # 前 50 封
eeeemail list --offset 20        # 跳过前 20 封
eeeemail list --unread           # 只看未读
```

### `read`

显示一封邮件。`id` 支持唯一前缀。

```sh
eeeemail read <id>
eeeemail read <id> --html        # 输出 HTML 正文而非纯文本
eeeemail read <id> --headers     # 打印原始头
```

### `search`

搜索本地数据库。本地搜索是对主题、摘要、正文、发件人（邮箱/姓名）的子串匹配。

```sh
eeeemail search "keyword"
eeeemail search "keyword" -n 50
eeeemail search "keyword" --server   # 改由服务端搜索（需要 provider 支持）
```

### `flags`

修改一封邮件的已读/星标标记。

```sh
eeeemail flags <id> --read true
eeeemail flags <id> --star true
eeeemail flags <id> --read false --star false
```

### `delete`

删除一封邮件。

```sh
eeeemail delete <id>
```

### `move`

把邮件移动到另一个文件夹（目标用角色名、远程名或 id 前缀）。

```sh
eeeemail move <id> archive
```

### `send`

发送邮件。`--to` 必填。

```sh
# 简单文本
eeeemail send --to a@example.com --subject "hi" --body "hello"

# 省略 --body 时从 stdin 读取正文
echo "hello" | eeeemail send --to a@example.com --subject "hi"

# 从文件读正文
eeeemail send --to a@example.com --body-file message.txt

# 多个收件人 / 抄送 / 密送
eeeemail send --to a@example.com --to b@example.com --cc c@example.com

# 附件（可多次）
eeeemail send --to a@example.com --subject "hi" --attach report.pdf

# 存草稿而非发送
eeeemail send --to a@example.com --subject "hi" --body "hello" --draft
```

正文来源优先级：`--body` → `--body-file` → `--editor`（或 stdin 是 TTY 时调 `$EDITOR`）→ stdin。

### `outbox`

查看或清空发件队列。

```sh
eeeemail outbox                 # 查看待发/失败任务
eeeemail outbox --flush         # 重试投递队列中/失败的任务
eeeemail outbox --retry         # 把失败任务移回待发
```

### `login`

完成 OAuth2 device 登录并保存 refresh token。详见 [outlook-oauth2.md](outlook-oauth2.md)。

```sh
eeeemail login -a outlook
```

### `agent`

在 stdio 上运行 read-first MCP 工具服务。详见 [agent-usage.md](agent-usage.md)。

```sh
eeeemail agent
```

### `tui`

打开终端 UI（无子命令时的默认行为）。

```sh
eeeemail tui
```

## 相关文档

- [watch.md](watch.md) — 新邮件监听与事件通道
- [agent-usage.md](agent-usage.md) — MCP agent 服务
- [outlook-oauth2.md](outlook-oauth2.md) — Outlook OAuth2 登录
