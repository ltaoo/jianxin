# jianxin CLI 使用说明

`jianxin` 是一个多厂商终端邮件客户端。Phase 1 先提供 CLI：在 JSON 里配置邮箱 → 同步到本地 SQLite → 在命令行读信、发信，需要交互时进入 TUI。

## 概览

```text
jianxin [全局选项] <子命令> [子命令选项]
```

不带子命令时默认进入 TUI。

HTML 邮件可用 `jianxin open <邮件ID>` 在浏览器阅读，或用 `jianxin web` 启动本地 Web 端。详情见 [Web 阅读说明](web.md)。

```sh
jianxin --help           # 列出全部子命令
jianxin <子命令> --help   # 查看某个子命令的完整参数
```

## 全局选项

所有子命令都接受以下选项：

| 选项 | 说明 |
|------|------|
| `--config <PATH>` | 配置文件路径。默认 `$JIANXIN_CONFIG`，否则用户目录下的 `~/.config/jianxin/config.json` |
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
      "platform": "gmail",
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
  "platform": "aliyun",
  "auth": { "type": "app_password", "password": { "env": "FUNZM_NOREPLY_PWD" } },
  "imap": { "host": "imap.qiye.aliyun.com", "port": 993, "tls": "tls" },
  "smtp": { "host": "smtp.qiye.aliyun.com", "port": 465, "tls": "tls" }
}
```

## 命令参考

### `init`

写入示例配置文件。

```sh
jianxin init                # 写到平台默认路径
jianxin init --path ./config.json
jianxin init --force        # 覆盖已有文件
```

### `accounts`

列出已配置的账号。

```sh
jianxin accounts
```

### `doctor`

检查每个账号（或 `-a` 指定的那一个）的连通性和凭据。

```sh
jianxin doctor
jianxin doctor -a noreply@funzm.com
```

任一账号失败时以非零退出码结束。

### `sync`

把邮件增量拉取到本地数据库。

```sh
jianxin sync                  # 同步全部账号
jianxin sync -a gmail         # 只同步一个账号
jianxin sync --full           # 忽略游标，重新拉取
jianxin sync --folder inbox   # 只同步某个文件夹
```

### `watch`

常驻轮询新邮件。详见 [watch.md](watch.md)。

```sh
jianxin watch
jianxin watch -a gmail --interval 60
```

### `daemon`

在后台定时拉取邮件，并在发现新邮件时发送系统通知：

```sh
jianxin daemon                 # 启动，间隔使用配置文件中的值
jianxin daemon --interval 60   # 指定轮询间隔
jianxin daemon --status        # 查看运行状态
jianxin daemon --stop          # 停止后台进程
```

重复执行启动命令不会创建第二个进程。macOS 使用 LaunchAgent 托管，日志文件保存在应用数据目录。

### `folders`

列出文件夹。

```sh
jianxin folders
jianxin folders --refresh     # 远程重新列出，而非读本地副本
```

### `list`

列出某文件夹的邮件，默认 `inbox`。

```sh
jianxin list                    # inbox 前 20 封
jianxin list sent               # 指定文件夹
jianxin list -n 50              # 前 50 封
jianxin list --offset 20        # 跳过前 20 封
jianxin list --unread           # 只看未读
```

### `read`

显示一封邮件。`id` 支持唯一前缀。

```sh
jianxin read <id>
jianxin read <id> --html        # 输出 HTML 正文而非纯文本
jianxin read <id> --headers     # 打印原始头
```

### `threads`

列出会话（按最后活动时间倒序）。会话是本地归组的结果：按 `In-Reply-To`/`References` 祖先链，再退到服务端线程 key（Gmail `X-GM-THRID`、IMAP `THREAD=REFERENCES`）、Outlook `Thread-Index` 根，最后是归一化主题。

```sh
jianxin threads                 # 最近 20 个会话
jianxin threads -n 50           # 前 50 个
jianxin threads --offset 20     # 翻页
jianxin threads --unread        # 只看有未读的会话
jianxin threads --rebuild       # 丢弃现有归组，按存储的头全部重算
```

`--rebuild` 用来在归组规则改进后重算存量邮件；它只基于已存储的 `References`/主题等信息，不会重新拉取邮件。

### `thread`

显示一个会话的全部邮件，时间正序。`id` 可以是会话 id/前缀，也可以是会话内任意一封邮件的 id/前缀。打开即把会话内未读标为已读。

```sh
jianxin thread <thread-id>
jianxin thread <message-id>     # 用邮件定位它所在的会话
jianxin thread <id> --html      # 输出 HTML 正文
```

### `search`

搜索本地数据库。本地搜索是对主题、摘要、正文、发件人（邮箱/姓名）的子串匹配。

```sh
jianxin search "keyword"
jianxin search "keyword" -n 50
jianxin search "keyword" --server   # 改由服务端搜索（需要 provider 支持）
```

### `flags`

修改一封邮件的已读/星标标记。

```sh
jianxin flags <id> --read true
jianxin flags <id> --star true
jianxin flags <id> --read false --star false
```

### `delete`

删除一封邮件。

```sh
jianxin delete <id>
```

### `move`

把邮件移动到另一个文件夹（目标用角色名、远程名或 id 前缀）。

```sh
jianxin move <id> archive
```

### `send`

发送邮件。`--to` 必填（使用 `--reply-to` 时可省略，自动填回复目标）。

```sh
# 简单文本
jianxin send --to a@example.com --subject "hi" --body "hello"

# 省略 --body 时从 stdin 读取正文
echo "hello" | jianxin send --to a@example.com --subject "hi"

# 从文件读正文
jianxin send --to a@example.com --body-file message.txt

# 多个收件人 / 抄送 / 密送
jianxin send --to a@example.com --to b@example.com --cc c@example.com

# 附件（可多次）
jianxin send --to a@example.com --subject "hi" --attach report.pdf

# 存草稿而非发送
jianxin send --to a@example.com --subject "hi" --body "hello" --draft

# 回复：自动填 To（Reply-To 优先）、Re: 主题、In-Reply-To/References 头
jianxin send --reply-to <id> --body "here you go"
```

正文来源优先级：`--body` → `--body-file` → `--editor`（或 stdin 是 TTY 时调 `$EDITOR`）→ stdin。

回复会携带 `In-Reply-To`/`References` 线程头，保证回复在各家客户端都归入同一会话。

### `outbox`

查看或清空发件队列。

```sh
jianxin outbox                 # 查看待发/失败任务
jianxin outbox --flush         # 重试投递队列中/失败的任务
jianxin outbox --retry         # 把失败任务移回待发
```

### `login`

完成 OAuth2 device 登录并保存 refresh token。详见 [outlook-oauth2.md](outlook-oauth2.md)。

```sh
jianxin login -a outlook
```

### `agent`

在 stdio 上运行 read-first MCP 工具服务。详见 [agent-usage.md](agent-usage.md)。

```sh
jianxin agent
```

### `tui`

打开终端 UI（无子命令时的默认行为）：左侧文件夹、右上的邮件/会话列表、右下的正文预览。按 `f` 同步、`c` 撰写、`t` 在平铺列表与会话视图间切换；完整快捷键见 [tui.md](tui.md)。

```sh
jianxin tui
```

## 相关文档

- [watch.md](watch.md) — 新邮件监听与事件通道
- [tui.md](tui.md) — 终端 UI 布局与快捷键
- [agent-usage.md](agent-usage.md) — MCP agent 服务
- [outlook-oauth2.md](outlook-oauth2.md) — Outlook OAuth2 登录

## 桌面托盘

构建配套 `jianxin-desktop` 后，运行 `jianxin tray` 启动三端托盘；
`jianxin tray --show` 立即显示最新未读邮件面板，`--no-daemon` 只查看现有本地数据。
托盘显示多个邮箱的邮件标题、时间和邮箱，每 5 秒更新，打开面板不改变已读状态。
构建说明与平台差异见 [桌面托盘](desktop-tray.md)。
