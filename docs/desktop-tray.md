# 三端托盘与未读邮件面板

桌面入口是 `jianxin-desktop`，由 GPUI Kit 渲染同一套 Rust 面板，`tray-icon` 接入系统托盘。
与 CLI、TUI、Web 共用配置和 SQLite，默认启动现有 `jianxin daemon` 后台收信。

- 左键点击托盘切换面板；右键菜单可查看未读邮件、打开阅读器、刷新。
- 跨邮箱显示最新 30 封未读邮件，包含主题、本地时间和完整邮箱地址；邮件行不显示悬停提示，左键点击不跳转。
- 每 5 秒读取本地数据库，沿用现有邮件去重与删除过滤规则；刷新不会强制远端同步。
- 每封邮件支持右键菜单“在浏览器中打开”，定位对应邮箱、文件夹和邮件正文。
- 面板打开和刷新不修改已读状态。右键选择“在浏览器中打开”才启动已有 Web 阅读器，由阅读器按现有规则同步已读。
- Esc、关闭按钮或窗口失焦关闭面板，后台收信继续运行。
- 浮层右下角“退出”按钮先停止后台收信，再关闭桌面应用；停止失败时保留应用并提示重试。托盘右键菜单“退出应用”使用相同的完全退出逻辑。
- 浮层跟随系统浅色/暗色主题；启动、重新打开及系统外观变化时自动同步，按钮、提示和滚动条一并适配。
- macOS 菜单栏使用随系统深浅色自动适配的原生信封图标，并显示未读总数；三端托盘提示显示总数。
- 每个数据目录只允许一个托盘实例；重复启动会显示已有实例的面板。

## 构建与启动

使用 Rust 1.92+。纯 CLI 仍可单独构建，不需要 GPUI。

```sh
cargo build --locked -p jianxin-cli -p jianxin-desktop
target/debug/jianxin tray
target/debug/jianxin tray --show
target/debug/jianxin --config /绝对路径/config.json tray --no-daemon
```

`--no-daemon` 适合已有 watcher 或只查看离线缓存。没有配置时先用 `jianxin init` 配置邮箱。
独立桌面程序也支持 `--config`、`--show`、`--no-daemon`，以及 `--cli <路径>` 指定配套 CLI。
程序默认查找同目录的 CLI、macOS 应用 Resources 中的 CLI，最后查找 PATH。

### macOS

```sh
bash scripts/build-macos-app.sh
```

构建产物是 `target/macos/Jianxin.app`。手动打开后显示托盘和面板，原 Swift 包装程序继续负责系统通知授权与通知点击。
安装到用户 Applications 目录使用已有 `--install` 选项。浮层“退出”会停止 launchd 后台收信。

### Windows

在带 MSVC 工具链的 PowerShell 中执行：

```powershell
.\scripts\build-desktop.ps1
.\target\release\jianxin.exe tray
```

分发时把 `jianxin.exe` 和 `jianxin-desktop.exe` 放在同一目录。桌面程序不创建控制台窗口。

### Linux

Ubuntu/Debian 构建依赖：

```sh
sudo apt-get install build-essential pkg-config clang libclang-dev libssl-dev libfontconfig1-dev libfreetype6-dev libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libvulkan-dev libxcb1-dev
bash scripts/build-desktop.sh
target/release/jianxin tray
```

托盘采用 KSNI / StatusNotifierItem，通过会话 D-Bus 工作，不需要另开 GTK 事件循环。
桌面环境需要提供 StatusNotifier host；GNOME 通常需要相应托盘扩展。可选的桌面入口模板在
`platform/linux/jianxin-desktop.desktop`；安装时需要将二进制加入 PATH，并安装已有简信图标。

KSNI 不提供托盘图标矩形，因此 Linux 使用屏幕右上方的备用位置；Wayland 最终位置由 compositor 决定。
没有托盘 host 时，可运行 `jianxin tray --show` 查看面板，窗口关闭后需再次通过该命令打开。

## 验证

`cargo test -p jianxin-desktop` 覆盖跨邮箱未读列表、时间排序、分页上限、文件夹副本去重、只读行为和窗口边界定位。
`.github/workflows/desktop.yml` 在三个系统分别编译并运行非图形测试。

手动检查各系统的托盘点击、右键菜单、Esc / 失焦关闭、重复启动、缩放与多显示器定位，以及同步后列表更新。
点击邮件后检查阅读器是否定位到正确邮件和邮箱。本项目不自动调用或操作浏览器。

macOS 启动后会在数据目录写入 `desktop-tray-status.json`，记录进程、原生菜单栏项可见状态、图标和位置，用于排查托盘不显示；不包含邮件内容。
