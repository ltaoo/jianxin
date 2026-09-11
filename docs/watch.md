# 新邮件 Watch 模式

`eeeemail watch` 是常驻轮询进程。当前使用增量 IMAP/SMTP 同步实现轮询；后续可以在不改变事件表和输出格式的前提下替换为 IMAP IDLE 或厂商 push。

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
eeeemail watch
```

只监听一个账号：

```sh
eeeemail watch --account outlook
```

自定义间隔，最小 5 秒：

```sh
eeeemail watch --interval 60
```

机器可读的 CLI 输出：

```sh
eeeemail watch --json
```

## 输出通道

### Webhook

```sh
eeeemail watch --webhook https://example.com/hooks/mail
```

每个事件都会以 `POST`、`Content-Type: application/json` 发送。非 2xx 响应、网络失败或超时都会保留事件，下一轮重试。

### 桌面通知

```sh
eeeemail watch --notify
```

支持：

```text
macOS:  osascript
Linux:  notify-send
```

### MCP notification

```sh
eeeemail watch --mcp
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
eeeemail watch --mcp --include-body
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
eeeemail watch --interval 30 --webhook https://example.com/hooks/mail
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
