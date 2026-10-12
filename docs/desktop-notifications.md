# Desktop notifications

The desktop program polls mail in-process and emits native **local** notifications.
Standalone CLI users can separately run `jianxin daemon`. It does not
use APNs, FCM, or a cloud push relay, and never uploads mail to a notification
service. Failed notification submissions remain pending for retry. System
acceptance is not proof a banner appeared: Focus / Do Not Disturb and desktop
settings may suppress it.

```sh
jianxin notifications --register
jianxin notifications --status
jianxin notifications --test
```

These optional commands belong to the separately built CLI, which is not included in desktop packages. They do not require mail configuration. Default is `--status`. Desktop users grant permission by opening Jianxin.app.

## HTTP notification hook

外部本机程序可以通过运行中的 Jianxin Web 服务发送系统通知。
需要长期接收调用时，关闭服务的空闲退出：

```sh
jianxin web --port 8787 --idle-seconds 0
```

在配置文件中添加（合并到已有 `general`，保留邮箱等其他配置）：

```json
{
  "general": {
    "api": {
      "token": "YOUR_RANDOM_TOKEN"
    }
  }
}
```

`token` 使用不含空格的 ASCII 字符串，也支持已有 SecretRef 格式，
例如 `{"env":"JIANXIN_API_TOKEN"}`。未配置或空 token 时接口拒绝所有调用。
修改配置后重启 Web 服务生效。API token 不会出现在 Web 页面/API 响应或配置 Debug 日志中。

推送接口固定为 `POST http://127.0.0.1:8787/api/notifications`，
在 `Authorization` 请求头传入配置的 token：

```sh
curl --fail-with-body -X POST 'http://127.0.0.1:8787/api/notifications' \
  -H 'Content-Type: application/json' \
  -H "Authorization: Bearer $JIANXIN_API_TOKEN" \
  --data '{"title":"任务完成","subtitle":"外部程序","body":"报表已生成","id":"report-job-1"}'
```

`JIANXIN_API_TOKEN` 是调用方环境变量，值应与服务端配置的 token 一致。

| 字段 | 说明 |
| --- | --- |
| `title` | 必填，非空，最多 120 字 |
| `body` | 可选，默认空，最多 4096 字，允许换行和制表符 |
| `subtitle` | 可选，默认「简信」，最多 120 字 |
| `id` | 可选，非空，最多 128 字；省略时生成唯一 ID |

成功返回 HTTP **202**，JSON `{"accepted":true,"id":"report-job-1"}`。
这表示原生系统通知服务接受了消息；横幅是否展示仍由系统权限、专注模式等决定。
macOS 中同一 ID 会使用同一通知标识，可用唯一 ID 区分各次任务。
错误码：400 参数无效，401 token 缺少/错误或未配置，403 来源校验失败，405 方法错误，413 请求超过 32 KiB，
415 非 JSON，429 并发请求过多，503 原生投递失败。

接口只监听 `127.0.0.1`，必须使用 `Authorization: Bearer <token>`。
不接受 URL 查询参数中的 token，不需要 `X-Jianxin-Request`。
浏览器跨来源请求不会被允许。推送路径不随 Web 阅读页的随机路径变化；
旧的 `/<随机令牌>/api/notifications` 已停用。
桌面程序已内置持续运行的 Web 服务，端口由系统自动分配，地址写入桌面启动输出 / App 日志；把示例中的 8787 替换为该端口。仅启动独立 CLI 的收信 daemon 不会提供此 HTTP 接口。
macOS 需已安装 Jianxin.app 并已允许系统通知。
点击通知直接打开 Web 邮件阅读器，跳过临时窗口；此接口不支持执行命令或自定义跳转地址。

## macOS

```sh
bash scripts/build-macos-app.sh --install
```

Requires Xcode command line tools, Rust 1.92+ for the native desktop, and Node.js 18+ for the embedded Web client. Run `npm ci --prefix webpc` before the first build. The AppKit tray and Swift notification helper target macOS 11+. The build script
produces a signed (ad-hoc, for local use) `~/Applications/Jianxin.app`, creates
all icon sizes, and registers `com.jianxin.mail` with LaunchServices. Existing
bundles are moved to the printed temporary build/backup directory, not deleted.
Distribution to other machines needs Developer ID signing / notarization.
The app uses a separate `macos-app` Cargo profile with size optimization and
thin LTO, plus Swift size optimization. It bundles the ICNS icon only; the
original PNG stays in the repository. Normal CLI release builds are unchanged.

Choose **Allow** when macOS asks, or enable 简信 in System Settings →
Notifications. A native AppKit/UserNotifications helper runs inside the signed
bundle; this avoids anonymous AppleScript notifications and dependence on
deprecated notification APIs. The desktop first uses its current signed bundle, then the user-installed
bundle or `/Applications/Jianxin.app`. `JIANXIN_MACOS_APP` can select a build.

Opening Jianxin.app manually starts the native Rust tray and its embedded Web server,
without a Dock icon or a separate CLI executable. Clicking the tray opens a native menu
with the latest 10 unread mail titles. Clicking a title opens that exact mail in the Web
reader; closing the browser keeps the tray running. Exiting from the tray stops the Web
server and in-process polling. System notification clicks route to the same running
instance, or cold-start it with the requested mail target. The Swift wrapper exits when
its desktop process exits. See [desktop tray](desktop-tray.md) for build and manual checks.
New-mail notifications carry the mailbox, local message ID and source config alongside
subject, sender and thread identity; the full message body stays out of notifications.
Clicking selects the matching mailbox and folder, highlights the conversation and
expands that exact message in the Web reader. Older notifications without routing
metadata may still open the inbox. No browser is opened by registration/status/test commands.

## Windows

Uses native WinRT Toast notifications (Windows 10/11). Registration
creates `HKCU\Software\Classes\AppUserModelId\com.jianxin.mail` with the 简信
display name and icon; it does not need administrator privileges. Sends also
ensure this registration exists, instead of borrowing PowerShell's identity.
Registration also creates the user-local `jianxin://` protocol handler. New-mail
toasts use [Windows protocol activation](https://learn.microsoft.com/windows/apps/design/shell/tiles-and-notifications/toast-schema)
and store their targets under the local application data `jianxin/notification-targets`
directory, so clicking from Notification Center works after the sender exits.
Allow notifications in Windows Settings. Windows runtime behavior must be
verified on Windows; a macOS test does not validate it.

## Linux

Uses `notify-rust` / session D-Bus / freedesktop notifications. No `notify-send`
installation is needed. `--register` installs a user-local `.desktop` entry
and PNG icon; it does not add an autostart entry. Requires a graphical desktop,
a notification service, and access to that user's session bus. A headless
server or system service outside the GUI session cannot show desktop banners.
Clicking the banner or “查看邮件” action opens the exact message. The notification
service must support actions, and the watcher must still be running to handle them.
Linux runtime behavior must be verified on a Linux desktop.

The third-party backend: https://github.com/hoodie/notify-rust
The icon source and generation prompt are in `assets/`.
