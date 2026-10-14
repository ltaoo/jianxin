# 桌面托盘与本地 Web 服务

桌面入口是 `jianxin-desktop`，启动后只有托盘，并在后台运行本地 Web 服务和收信线程。不提供原生桌面邮件应用；邮箱配置、邮件阅读和回复均在默认浏览器的 Web 工作区中完成。
分发包不包含独立 CLI / TUI 程序，也不依赖 GPUI Kit、GPU 渲染器或托盘浮层。
邮件引擎、配置和 SQLite 仍与独立构建的 CLI 共用，Web 后端位于 `crates/jianxin-web`。

## 托盘菜单

「邮箱」二级菜单列出所有已配置邮箱，显示最近一次同步完成时间和当前状态，点击邮箱打开对应邮件列表。尚无同步记录的邮箱会明确标注。阅读器侧栏的「同步日志」提供按邮箱筛选的历史记录，详见 [同步日志](web.md#同步日志)。

- 左键或右键点击托盘展开系统原生菜单。Linux 通过 StatusNotifier 的 `IsMenu` 属性请求托盘 host 显示菜单。
- 跨邮箱显示最近 **10 封未读邮件标题**，按邮件时间从新到旧排列；沿用数据库中的副本去重与删除过滤。
- 每条显示“标题 · YYYY-MM-DD HH:mm”，使用本地时区，优先取发送时间，缺失时取接收时间；时间缺失或无效时显示“时间未知”。空标题显示“（无主题）”，换行合并为空格，标题超过 80 字截断为省略号，时间完整保留。邮件总未读数不受 10 条上限限制。
- 点击标题在默认浏览器中打开 Web 阅读器，定位对应邮箱、文件夹和邮件正文。同名标题按邮件 ID 分别路由。
- 展开或刷新菜单不修改已读状态；进入阅读器后，沿用 Web 阅读器的已读同步规则。
- 菜单提供“打开邮件阅读器”“立即收信 / 刷新菜单”和“退出应用”。无未读邮件、首次加载及错误均有状态提示。
- 每 5 秒读取本地未读缓存；手动刷新也触发内置收信。默认按配置的 `watch.interval_secs` 后台同步。
- 原生菜单随系统主题显示；macOS 菜单栏使用原生信封图标和未读计数。
- Web 左下角「更多 → 配置」可切换「托盘显示未读数」，默认开启。设置保存在 `general.tray_show_unread_count`，macOS 菜单栏数字在 5 秒内更新，重启后保留；托盘菜单中的未读邮件列表和总数保持可用。
- 退出结束托盘、Web 服务和本进程收信。关闭浏览器标签页不会停止收信；阅读器保持一个服务，重复打开复用地址。
- Web 服务默认监听 `http://127.0.0.1:65250/`。端口被占用时在 `65250–65265` 中选择可用端口，并保存到数据目录的 `desktop-web-port`；重启优先复用已保存端口，即使默认端口重新空闲也不会切回。所有候选端口均被占用时会提示错误。
- 退出不以停止收信成功为条件。点击退出即开始清理；网络、数据库或托盘析构阻塞时，2 秒内强制结束本进程。macOS 的 Cmd-Q / 系统退出直接通知它拥有的桌面进程，同样有 2 秒强制退出兜底；应用包运行中被移动或删除也不影响退出。
- 每个数据目录只运行一个桌面实例；重复启动默认不打开阅读器，`--show` 和通知点击会转发给现有实例。

## 构建与启动

需要 Rust 1.92+、Node.js 18.20+ 和平台原生编译工具。首次运行：

```sh
npm ci --prefix webpc
cargo build --locked --release -p jianxin-desktop
target/release/jianxin-desktop
```

Cargo 自动构建并内嵌 Web 资源。桌面程序支持 `--config <路径>`、`--show`（在浏览器中打开阅读器）和 `--no-daemon`；
后者关闭本实例的收信，仅查看本地缓存。通知路由使用内部的 `--account`、`--open-message` 和 `--notification-open` 参数。
桌面程序不提供 `sync`、`watch`、`agent`、`tui` 等 CLI 子命令。

配置优先使用 `--config`、`JIANXIN_CONFIG`、桌面可执行文件同目录已有的 `config.json`，最后使用默认配置路径。
没有配置时创建空配置并显示空托盘，不要求先运行 CLI。
在菜单中打开阅读器后，可以导入邮箱配置并完成 OAuth 登录；配置更改会自动加载到托盘和内置收信线程。
原来的原生“配置加密”窗口已移除，现有 Web 密文导入功能保留。

### macOS

```sh
bash scripts/build-macos-app.sh
```

产物是 `target/macos/Jianxin.app`，只包含桌面可执行文件、图标和小型 Swift 通知包装器。
手动打开后只显示托盘并启动本地 Web 服务，不自动打开浏览器。再次打开 App 会在浏览器中打开 Web 阅读器。
系统通知点击直接转发邮件目标到桌面程序；冷启动时也可定位原邮件，无需独立 CLI 或 Web 子进程。
通知包装器随它启动的桌面程序退出。

`bash scripts/build-macos-app.sh --install` 可安装到用户 Applications 目录。
首次正常启动会申请系统通知权限。旧版相同配置的 launchd 收信服务会停止，原 plist 移至数据目录的 `legacy-mail-daemon-*.plist`，以后由桌面进程收信。

### Windows

在 MSVC 工具链的 PowerShell 中执行：

```powershell
.\scripts\build-desktop.ps1
.\target\release\jianxin-desktop.exe
```

只分发 `jianxin-desktop.exe`，构建脚本生成仅包含此文件的 `target/release/jianxin-windows.zip`。
双击后只显示托盘并启动本地 Web 服务，不创建邮件窗口或控制台窗口。点击托盘中的「打开邮件阅读器」在浏览器中管理邮箱和邮件。启动错误会显示系统对话框并写入 `%TEMP%/jianxin-startup-error.log`。
系统 Toast 通过用户级 `jianxin://` 协议转发给同一桌面程序，通知中心的历史通知也可定位邮件。

### Linux

Ubuntu/Debian 构建依赖：

```sh
sudo apt-get install build-essential pkg-config libssl-dev
bash scripts/build-desktop.sh
target/release/jianxin-desktop
```

托盘使用 KSNI / StatusNotifierItem，会话 D-Bus 和系统托盘 host 负责显示菜单；不需要 GTK、Wayland、Vulkan 或 GPUI 的图形开发库。
GNOME 通常需要托盘扩展。桌面入口模板是 `platform/linux/jianxin-desktop.desktop`。

## 验证

```sh
cargo test --locked -p jianxin-web -p jianxin-desktop -p jianxin-cli
node --test crates/jianxin-web/src/web-reader.test.cjs
npm test --prefix webpc
```

测试覆盖跨邮箱排序、10 条上限、副本去重、只读菜单、同名标题路由、深链接、单实例转发、通知目标、嵌入式 HTTP 服务启动/退出及现有 Web 功能。
自动化测试不启动或操作浏览器；用户点击托盘菜单时，程序会调用系统默认浏览器打开阅读器。

手动验证：点击托盘菜单、标题打开对应邮件、无未读时的空状态、同步后的菜单更新、系统主题、重复启动、通知点击，以及退出后不再收信且 Web 地址停止响应。
macOS / Windows 在数据目录写入 `desktop-tray-status.json` 供排查托盘可见性。
仅验证 HTTP 时，可设置 `JIANXIN_DESKTOP_HEADLESS=1` 并使用临时空邮箱配置；程序输出回环地址，不初始化托盘或打开浏览器。
