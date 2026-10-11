# 简信 · Jianxin

简信是支持多个邮箱和邮件服务商的邮件客户端，提供 CLI、TUI、本地 Web 阅读器、桌面托盘，以及独立的 iOS / Android 原生应用。目前通过 IMAP / SMTP 连接 Gmail、Outlook、QQ、163、阿里云和通用邮箱。

桌面各入口共用配置与本地 SQLite 数据库，可跨邮箱浏览、搜索和聚合会话；移动端直接连接邮箱服务器，支持本地缓存、草稿、附件和加密配置导入。

## 快速开始

需要 Rust 1.92+、Node.js 18+ 和所在平台的原生编译工具。Linux 桌面依赖见[桌面构建说明](docs/desktop-tray.md#linux)。

在仓库根目录安装 Web 构建依赖，再构建 CLI：

```sh
npm ci --prefix webpc
cargo build --locked --release -p jianxin-cli
target/release/jianxin init --path ./config.json
```

编辑生成的 `config.json`，填写自己的邮箱、服务器和登录凭据。密码或客户端授权码可以通过环境变量、文件或系统钥匙串引用；完整配置见 [CLI 使用说明](docs/cli.md#配置)，Outlook 登录见 [OAuth 说明](docs/outlook-oauth2.md)。

```sh
target/release/jianxin --config ./config.json doctor
target/release/jianxin --config ./config.json sync
target/release/jianxin --config ./config.json tui
```

启动本地 Web 阅读器后，手动打开命令输出的网址：

```sh
target/release/jianxin --config ./config.json web
```

Windows 使用 `target\release\jianxin.exe`。省略 `--config` 时，CLI 使用 `JIANXIN_CONFIG` 或默认的 `~/.config/jianxin/config.json`。

## 各端入口

| 入口 | 用途 | 说明 |
| --- | --- | --- |
| CLI / TUI | 收发邮件、同步、搜索与会话管理 | [CLI](docs/cli.md)、[TUI](docs/tui.md) |
| Web PC | 本地邮件阅读、回复、配置导入和邮件助手 | [使用说明](docs/web.md)、[构建与开发](webpc/README.md) |
| 桌面托盘 | 未读面板、后台收信和系统通知 | [桌面托盘](docs/desktop-tray.md)、[系统通知](docs/desktop-notifications.md) |
| Android / iOS | 独立原生客户端，直接连接邮箱 | [移动端构建与使用](mobile/README.md) |
| MCP | 向 Agent 提供邮件工具 | [Agent 接入](docs/agent-usage.md) |

## 目录

```text
crates/     Rust 邮件引擎、CLI、TUI、MCP 与桌面托盘
webpc/      PC Web 阅读器、组件、设计契约与构建脚本
android/    Android 原生应用与 Gradle 工程
ios/        iOS 原生应用与 Xcode 工程
mobile/     两端共用的设计契约、测试、工具与说明
docs/       使用说明与架构设计
platform/   桌面平台适配与打包资源
scripts/    桌面构建与资源生成脚本
assets/     应用图标与邮件服务商图形
```

架构设计见[多邮件厂商 Email Client 架构设计文档](docs/多邮件厂商%20Email%20Client%20架构设计文档.md)；新邮件监听与事件通道见 [watch](docs/watch.md)。

## 构建产物与本地文件

`target/`、`webpc/dist/`、`node_modules/`、Android / iOS 构建缓存与安装包、日志、运行状态文件、本地配置和签名资料均被 Git 忽略。Cargo 构建会自动生成 Web 资源并嵌入程序；手动运行 `node webpc/build.mjs` 可生成 `webpc/dist/`。

仓库保留锁文件、Gradle Wrapper、平台工程和源码资源，以便独立构建。移动端主题、图标等资源的生成命令见[移动端说明](mobile/README.md#目录与设计)。

## 非浏览器检查

```sh
cargo test --locked -p jianxin-cli -p jianxin-desktop
node --test webpc/tests/*.test.mjs
node mobile/tests/config_crypto.test.mjs
mobile/tools/test_swift.sh
```

Android 检查在 `android/` 中运行 `./gradlew :app:testDebugUnitTest :app:lintDebug`。各端交互与真实邮箱兼容性请按对应文档手动验证。
