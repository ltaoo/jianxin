# 新邮件 Watch 模式

`jianxin watch` 是常驻轮询进程。当前使用增量 IMAP/SMTP 同步实现轮询；后续可以在不改变事件表和输出格式的前提下替换为 IMAP IDLE 或厂商 push。

```text
定时 poll
   ↓
发现本地新增 message id
   ↓
写入 SQLite mail_events
   ↓
CLI 输出 / webhook / 桌面通知 / MCP notification
```

## 基本用法

监听全部账号，使用 `config.json` 里的默认间隔：

```sh
jianxin watch
```

只监听一个账号：

```sh
jianxin watch --account outlook
```

自定义间隔，最小 5 秒：

```sh
jianxin watch --interval 60
```

机器可读的 CLI 输出：

```sh
jianxin watch --json
```

## 后台运行

`daemon` 会在后台启动 `watch --notify`，关闭终端后仍继续轮询，并强制启用系统通知：

```sh
jianxin daemon
jianxin daemon --interval 60
jianxin daemon --status
jianxin daemon --stop
```

macOS 使用 LaunchAgent 托管进程，异常退出后会自动重启；其他平台使用 PID
文件防止重复启动。日志写入应用数据目录下的 `jianxin-daemon.log`。

新邮件事件只针对收件箱（标准化角色 `inbox`），不包括已发送、归档、草稿等
文件夹，也排除已删除邮件。按“账号 + Message-ID”持久化去重；缺少 Message-ID
时退回本地邮件 ID，不会按主题或 thread 合并不同邮件。不同账号收到同一封邮件
仍会各自提醒；同一对话的新回复也会正常提醒。旧事件记录保留，但非收件箱及
重复事件不再投递。通知失败时保留同一个事件用于重试。

## 输出通道

### Webhook

```sh
jianxin watch --webhook https://example.com/hooks/mail
```

每个事件都会以 `POST`、`Content-Type: application/json` 发送。非 2xx 响应、网络失败或超时都会保留事件，下一轮重试。

### 桌面通知

```sh
jianxin watch --notify
```

支持：

```text
macOS:   Jianxin.app / UserNotifications（独立应用名称和图标）
Windows: notify-rust / WinRT Toast
Linux:   notify-rust / session D-Bus
```

首次使用执行 `jianxin notifications --register`，用
`jianxin notifications --test` 检查系统提醒。macOS 先安装应用包。
安装与权限说明见 [desktop-notifications.md](desktop-notifications.md)。

### MCP notification

```sh
jianxin watch --mcp
```

输出 newline-delimited JSON-RPC logging notification：

```json
{
  "jsonrpc": "2.0",
  "method": "notifications/message",
  "params": {
    "level": "info",
    "data": {
      "type": "new_mail"
    }
  }
}
```

`data` 是完整新邮件事件。默认只包含摘要；加 `--include-body` 才包含正文：

```sh
jianxin watch --mcp --include-body
```

正文字段会同时带有：

```json
{
  "body_is_untrusted_email_content": true
}
```

邮件正文是数据，不是指令。

## 配置文件

可以在 `config.json` 中持久化 watch 配置：

```json
{
  "general": {
    "watch": {
      "interval_secs": 300,
      "webhook": "https://example.com/hooks/mail",
      "notify": false
    }
  }
}
```

CLI 参数优先于配置文件：

```sh
jianxin watch --interval 30 --webhook https://example.com/hooks/mail
```

## 事件持久化

事件写入 SQLite `mail_events` 表：

```text
event id
account id
message id
detected_at
delivered_at
```

规则：

```text
1. 同一封邮件只会生成一次事件
2. 输出成功后写入 delivered_at
3. webhook / 桌面通知失败时保留事件并重试
4. watch 重启后会先投递未完成事件
5. 首次同步视为基线，不把历史邮件全部当成新邮件
```

## Agent 总结

默认事件包含：

```text
账号
邮件 id / thread id
发件人
主题
摘要
时间
附件数量
```

如果通知消费者是 Agent，可以用 `--include-body` 直接读取正文并总结；也可以默认只传摘要，再让 Agent 调用 `mail_get_message(include_body=true)` 或 `mail_get_thread(include_body=true)` 按需读取正文。

按需读取正文更省上下文，也能降低邮件内容 prompt injection 的影响面。
