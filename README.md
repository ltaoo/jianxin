<div align="center" data-n="readme-header">
  <img src="assets/jianxin-macos.png" alt="简信 Logo" width="128" height="128" data-n="readme-logo" />
  <h1 data-n="readme-title">简信 · Jianxin</h1>
  <p data-n="readme-tagline"><strong>多个邮箱，一处阅读。</strong></p>
  <p data-n="readme-description">支持桌面、终端与原生移动端的邮件客户端，通过 IMAP / SMTP 连接你的邮箱。</p>
  <p data-n="readme-badges">
    <a href="https://github.com/ltaoo/jianxin/actions/workflows/desktop.yml" data-n="readme-ci-link"><img src="https://github.com/ltaoo/jianxin/actions/workflows/desktop.yml/badge.svg" alt="Desktop checks" data-n="readme-ci-badge" /></a>
    <a href="Cargo.toml" data-n="readme-rust-link"><img src="https://img.shields.io/badge/Rust-1.92%2B-orange?style=flat-square&amp;logo=rust" alt="Rust 1.92+" data-n="readme-rust-badge" /></a>
    <a href="#支持平台" data-n="readme-platform-link"><img src="https://img.shields.io/badge/platforms-macOS%20%7C%20Windows%20%7C%20Linux%20%7C%20Android%20%7C%20iOS-2563EB?style=flat-square" alt="macOS / Windows / Linux / Android / iOS" data-n="readme-platform-badge" /></a>
  </p>
  <p data-n="readme-navigation">
    <a href="#主要功能" data-n="readme-features-link">主要功能</a> ·
    <a href="#快速开始" data-n="readme-start-link">快速开始</a> ·
    <a href="#文档" data-n="readme-docs-link">文档</a> ·
    <a href="#开发与贡献" data-n="readme-contributing-link">参与贡献</a> ·
    <a href="https://github.com/ltaoo/jianxin/issues" data-n="readme-issues-link">反馈问题</a>
  </p>
</div>

---

## 主要功能

- **统一阅读** — 集中浏览多个邮箱，按账号和文件夹切换，筛选未读、搜索邮件、收藏重要内容。
- **对话聚合** — 将同一会话中的来往邮件放在一起，过滤同一邮箱不同文件夹中的邮件副本。
- **桌面常驻** — 从原生托盘查看最近 10 封未读邮件，后台收信、接收系统通知，点击后定位对应邮件。
- **本地阅读** — 桌面、CLI、TUI 与 Web 共用本机 SQLite 数据库；Web 服务仅监听 `127.0.0.1`。
- **原生移动端** — Android 与 iOS 直接连接邮箱服务器，支持本地缓存、草稿、附件和加密配置导入。
- **邮件助手** — Web 内置助手支持检索、阅读与草稿提议；独立 MCP Server 为外部 Agent 提供受限的邮件工具。

通过 IMAP / SMTP 连接 Gmail、Outlook、QQ、163、126、阿里云等服务，也支持自定义服务器。服务商可能要求先开启 IMAP / SMTP，并使用客户端授权码、应用专用密码或 OAuth 登录；Outlook 配置见 [OAuth 指南](docs/outlook-oauth2.md)。

## 支持平台

| 入口 | 平台 | 使用方式 |
| --- | --- | --- |
| 桌面应用 | macOS / Windows / Linux | 原生托盘、后台收信、系统通知与内置 Web 阅读器 |
| Web 阅读器 | 本机浏览器 | 邮箱侧栏、对话列表、正文阅读、回复、同步日志与邮件助手 |
| CLI / TUI | macOS / Windows / Linux | 命令行收发与同步、交互式终端阅读 |
| 原生移动应用 | Android 8+ / iOS 17+ | 独立连接邮箱，直接在设备上收发邮件 |
| MCP Server | 支持 MCP 的 Agent 客户端 | 通过 stdio 接入本地邮件工具 |

桌面分发包包含独立的 `jianxin-desktop` 程序；CLI / TUI 可按需另行构建。移动应用独立运行，无需启动桌面端。

## 快速开始

### 下载发布版

前往 [GitHub Releases](https://github.com/ltaoo/jianxin/releases) 下载对应平台的桌面包：macOS 提供 Apple Silicon / Intel 两个版本，Windows 和 Linux 提供 x64 版本。macOS 解压后打开 `Jianxin.app`；Windows 解压后运行 `jianxin-desktop.exe`；Linux 解压后运行 `./jianxin-desktop`。

发布由版本标签触发，下面的步骤适用于从源码构建。

### 准备环境

桌面与 CLI 构建需要 **Rust 1.92+**、**Node.js 18+** 和平台原生编译工具：macOS 使用 Xcode Command Line Tools，Windows 使用 MSVC 工具链，Linux 依赖见[构建说明](docs/desktop-tray.md#linux)。

```sh
git clone https://github.com/ltaoo/jianxin.git
cd jianxin
npm ci --prefix webpc
```

Cargo 会自动构建并嵌入 Web 资源，无需单独启动前端开发服务。

### 运行桌面应用

<details open data-n="readme-macos-build">
<summary data-n="readme-macos-summary"><strong>macOS</strong></summary>

```sh
bash scripts/build-macos-app.sh
open target/macos/Jianxin.app
```

加上 `--install` 可安装到用户的 Applications 目录。首次启动会申请系统通知权限。

</details>

<details data-n="readme-windows-build">
<summary data-n="readme-windows-summary"><strong>Windows · PowerShell</strong></summary>

在已配置 MSVC 工具链的 PowerShell 中执行：

```powershell
.\scripts\build-desktop.ps1
.\target\release\jianxin-desktop.exe
```

产物为 `target/release/jianxin-desktop.exe` 和 `target/release/jianxin-windows.zip`。

</details>

<details data-n="readme-linux-build">
<summary data-n="readme-linux-summary"><strong>Linux · Ubuntu / Debian</strong></summary>

```sh
sudo apt-get install build-essential pkg-config libssl-dev
bash scripts/build-desktop.sh
target/release/jianxin-desktop
```

托盘需要会话 D-Bus 与 StatusNotifier host；GNOME 通常需要托盘扩展。

</details>

启动后点击托盘中的「打开邮件阅读器」，再点击「添加邮箱」，可导入加密配置或选择服务商手动添加。没有配置时会创建空配置并显示空托盘；详细步骤见 [添加邮箱](docs/web.md#添加邮箱)，配置字段见 [配置指南](docs/cli.md#配置)。

桌面程序支持 `--config <路径>` 指定配置、`--show` 打开阅读器。配置变更会自动加载到托盘与后台收信线程，详细行为见[桌面使用说明](docs/desktop-tray.md)。

### 使用 CLI / TUI

```sh
cargo build --locked --release -p jianxin-cli
target/release/jianxin init --path ./config.json
```

编辑生成的 `config.json`，填写邮箱、服务器和登录凭据，再检查连接并同步邮件：

```sh
target/release/jianxin --config ./config.json doctor
target/release/jianxin --config ./config.json sync
target/release/jianxin --config ./config.json tui
```

也可以启动独立的本地 Web 阅读器，手动打开命令输出的网址：

```sh
target/release/jianxin --config ./config.json web
```

Windows 将上述可执行文件路径替换为 `target\release\jianxin.exe`。省略 `--config` 时，CLI 使用 `JIANXIN_CONFIG`，否则读取 `~/.config/jianxin/config.json`。密码和授权码可通过环境变量、文件或系统钥匙串引用，详见 [CLI 文档](docs/cli.md)。

### 使用移动应用

- **Android**：需要 JDK 17、Android SDK Platform 35 和 Build Tools 35，在 `android/` 中使用 Gradle Wrapper 构建。
- **iOS**：使用 Xcode 16+ 打开 `ios/Jianxin.xcodeproj`，选择 Jianxin scheme 并配置自己的开发团队。

安装、签名、测试和配置导入步骤见[移动端指南](mobile/README.md)。

## 文档

| 你想做什么 | 从这里开始 |
| --- | --- |
| 配置邮箱、同步或在命令行收发邮件 | [CLI 指南](docs/cli.md) · [Outlook OAuth](docs/outlook-oauth2.md) |
| 在终端中浏览账号、文件夹与会话 | [TUI 指南](docs/tui.md) |
| 阅读邮件、回复、管理收藏或使用邮件助手 | [Web 使用指南](docs/web.md) |
| 构建桌面应用、配置托盘与系统通知 | [桌面托盘](docs/desktop-tray.md) · [通知指南](docs/desktop-notifications.md) |
| 安装和开发 Android / iOS 应用 | [移动端指南](mobile/README.md) |
| 监听新邮件、连接 Webhook 或事件通道 | [watch 指南](docs/watch.md) |
| 给外部 Agent 接入邮件工具 | [Agent / MCP 指南](docs/agent-usage.md) |
| 修改 Web 界面、组件与设计规范 | [Web 开发指南](webpc/README.md) · [jxui 组件](webpc/packages/jxui/README.md) |
| 了解邮件引擎与提供商架构 | [架构设计](docs/多邮件厂商%20Email%20Client%20架构设计文档.md) |
| 替换所有 Logo，或为托盘指定单独图标 | [统一图标脚本](assets/README.md) |

启用邮件助手后，相关邮件内容会发送给配置的模型服务；打开面板本身不会调用模型。Web 助手不提供发送工具，草稿操作需确认；外部 MCP 的能力边界见 [Agent 文档](docs/agent-usage.md)。

## 开发与贡献

欢迎通过 [Issues](https://github.com/ltaoo/jianxin/issues) 反馈问题或提出建议，也欢迎提交 [Pull Request](https://github.com/ltaoo/jianxin/pulls)。报告问题时请提供平台、版本、复现步骤及脱敏日志；提交变更时说明行为变化与验证方式。

### 发布新版本

将待发布的变更提交并推送后，在对应提交上创建 `v主版本.次版本.修订版本` 标签：

```sh
git tag v0.1.0
git push origin v0.1.0
```

[Release workflow](.github/workflows/release.yml) 会运行测试，构建四个桌面包，在全部成功后创建对应的 GitHub Release、生成发布说明并上传附件。附件名包含标签、平台与架构；重跑同一标签会更新同名附件。普通分支 push 不触发发布。

发布使用自动提供的 `GITHUB_TOKEN`，仅发布任务申请 `contents: write`，无需配置额外 token。macOS 包使用现有脚本的临时签名，未做 Apple 公证；首次运行可能需要在系统设置中允许打开。Android / iOS 的签名发布暂未加入此 workflow。

### 本地检查

在仓库根目录执行：

```sh
cargo test --locked -p jianxin-web -p jianxin-cli -p jianxin-desktop
node --test crates/jianxin-web/src/web-reader.test.cjs
npm test --prefix webpc
node mobile/tests/config_crypto.test.mjs
```

macOS 可运行 `mobile/tools/test_swift.sh` 检查移动端协议逻辑；Android 在 `android/` 中运行 `./gradlew :app:testDebugUnitTest :app:lintDebug`。托盘、通知与真实邮箱兼容性还需在对应平台手动验证。

### 项目结构

```text
crates/     Rust 邮件引擎、CLI、TUI、MCP、桌面与 Web 后端
webpc/      Web 阅读器、组件与设计契约
android/    Android 原生应用与 Gradle 工程
ios/        iOS 原生应用与 Xcode 工程
mobile/     移动端共用工具、测试与说明
docs/       使用指南与架构设计
platform/   桌面平台适配与打包资源
scripts/    桌面构建与资源生成脚本
assets/     应用 Logo 与邮件服务商图形
```

锁文件、Gradle Wrapper、平台工程和源资源保留在仓库中。`target/`、`webpc/dist/`、依赖目录、移动端构建缓存与安装包、日志、本地配置和签名资料均被 Git 忽略；个人凭据和签名资料应留在本机。
