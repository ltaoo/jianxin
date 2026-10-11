# 三端托盘与未读邮件面板

桌面入口是 `jianxin-desktop`，由 GPUI Kit 渲染同一套 Rust 面板，`tray-icon` 接入系统托盘。
与 CLI、TUI、Web 共用配置和 SQLite，默认启动现有 `jianxin daemon` 后台收信。

- 直接运行托盘程序默认只显示托盘图标，不打开浮层；左键点击托盘切换面板，右键菜单提供打开阅读器、刷新、配置加密和退出。
- 跨邮箱显示最新 30 封未读邮件，包含主题、本地时间和完整邮箱地址；邮件行不显示悬停提示，左键点击不跳转。
- 每 5 秒读取本地数据库，沿用现有邮件去重与删除过滤规则；刷新不会强制远端同步。
- 每封邮件支持右键菜单“在浏览器中打开”，定位对应邮箱、文件夹和邮件正文。
- 面板打开和刷新不修改已读状态。右键选择“在浏览器中打开”才启动已有 Web 阅读器，由阅读器按现有规则同步已读。
- Esc、关闭按钮或窗口失焦关闭面板，后台收信继续运行。
- 浮层右下角“退出”按钮先停止后台收信，再关闭桌面应用；停止失败时保留应用并提示重试。托盘右键菜单“退出应用”使用相同的完全退出逻辑。
- 浮层跟随系统浅色/暗色主题；启动、重新打开及系统外观变化时自动同步，按钮、提示和滚动条一并适配。
- macOS 菜单栏使用随系统深浅色自动适配的原生信封图标，并显示未读总数；三端托盘提示显示总数。
- 每个数据目录只允许一个托盘实例；重复启动保持当前面板状态，显式传入 `--show` 才打开已有实例的面板。
- 打开阅读器由独立线程处理，不等待后台收信启动或未读列表刷新；重复打开会复用仍在运行的 Web 服务。服务空闲 30 分钟退出后，下次打开自动重新启动。Windows 直接调用默认浏览器接口打开地址；启动失败时在面板展示错误。

## 构建与启动

使用 Rust 1.92+。纯 CLI 仍可单独构建，不需要 GPUI。

```sh
cargo build --locked -p jianxin-cli -p jianxin-desktop
target/debug/jianxin tray
target/debug/jianxin tray --show
target/debug/jianxin --config /绝对路径/config.json tray --no-daemon
```

`--no-daemon` 适合已有 watcher 或只查看离线缓存。没有配置时会创建空配置并正常显示托盘；没有邮箱时不启动收信，也不弹出配置提醒。
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
双击任一 exe 均可启动托盘，默认不打开浮层。在已有终端运行无参数的 `jianxin` 仍进入 TUI。
配置路径优先使用 `--config`、`JIANXIN_CONFIG`，其次使用桌面 exe 同目录已有的 `config.json`，最后使用 `~/.config/jianxin/config.json`。
配置不存在时自动新建空配置；配置损坏等启动错误会显示 Windows 对话框，并写入 `%TEMP%/jianxin-startup-error.log`。
两个 exe 均内嵌简信 Logo；运行时托盘使用预先缩小的 32px 图标，原始高分辨率 Logo 不打入桌面程序。
Release 构建采用体积优化、Thin LTO、单代码生成单元，移除符号和调试信息，并使用 `panic = "abort"`。
发生 Rust panic 时进程直接退出；普通邮件、配置和网络错误仍沿用现有错误处理。
构建脚本同时生成 `target/release/jianxin-windows.zip`，只包含两个 exe，无需附带源代码、调试文件或原始图标。

### Web 导入邮箱配置

阅读器左侧提供“导入邮箱配置”按钮。
支持本地密文文件和 HTTPS URL，最大 2 MB。选择与安卓端相同的 AES-256-GCM、AES-128-GCM 或旧版 JSON，输入解密密钥，再点击“读取并解密”。
兼容 `mobile/tools/config_crypto.mjs` 生成的文件。密钥只用于当前浏览器的 Web Crypto 解密，不会发给服务器或保存到浏览器存储。
预览后勾选要保存的邮箱：添加新邮箱，更新同身份已有邮箱，保留未选账号和当前 `general` 设置。无效或重复账号在预览中提示。
OAuth 邮箱即使没有密码、访问令牌或刷新令牌也可导入，预览标记“待授权”。保存后立即显示“等待 OAuth 登录”，点击邮箱状态图标手动授权。
Outlook 自动采用与安卓相同的 Microsoft 令牌端点和 IMAP/SMTP 权限。缺少 Client ID 等授权应用信息时，可在邮箱状态面板填写并保存；保存设置不会自动开始登录。
配置保存后 Web 邮箱列表和运行中的托盘自动更新；已有收信后台会重启加载新配置。初次添加邮箱后，正常运行的托盘会启动收信。
密钥错误、下载失败或文件损坏不会修改当前配置。本地文件导入不联网；URL 由本机服务下载，支持 HTTPS 重定向，不允许降级到 HTTP。

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

## 配置加密

在托盘右键菜单或未读邮件浮层点击“配置加密”，打开原生 GPUI Kit 窗口。
页面自动显示桌面端当前使用的 `config.json` 路径，包括启动时通过 `--config` 指定的文件。
输入加密密钥后点击“生成密文”，也可在密钥输入框按 Enter。每次生成都会重新读取文件，
在后台完成加密；密钥不会保存到磁盘。窗口失焦仍然保留，重复打开会激活同一窗口，关闭后再次打开会清空表单。

生成成功后可以查看、选择和复制完整的 Base64 密文。将密文保存为文本文件并上传到 CDN，
手机通过“邮箱 → 从 URL 导入加密配置”填写 HTTPS 地址、选择 **AES-256-GCM**，并输入相同密钥。
格式与 `mobile/tools/config_crypto.mjs` 默认输出一致，桌面工具无需 Node.js。
手机仅导入字符串账号凭据，不能读取电脑上的 `env`、`file`、`keychain` 等引用；这些配置需要先按
[mobile 导入说明](../mobile/README.md#从-cdn-导入加密-configjson)调整。

页面仅显示配置路径和密文。修改密钥或重新生成会清除上一次结果，生成失败时显示错误并可重试。

## 验证

`cargo test -p jianxin-desktop` 覆盖跨邮箱未读列表、时间排序、分页上限、文件夹副本去重、只读行为、窗口边界定位，以及 mobile 加密向量互通、错误密钥/密文篡改、配置校验、随机 nonce 和当前配置重读。
`.github/workflows/desktop.yml` 在三个系统分别编译并运行非图形测试。

手动检查各系统的托盘点击、右键菜单、Esc / 失焦关闭、重复启动、缩放与多显示器定位，以及同步后列表更新。
点击邮件后检查阅读器是否定位到正确邮件和邮箱。本项目不自动调用或操作浏览器。
配置加密需手动检查两个入口、路径、密钥遮蔽、Enter/按钮生成、复制、错误重试、关闭/重新打开和浅色/暗色主题；复制密文后可在手机中手动验证导入。

macOS 启动后会在数据目录写入 `desktop-tray-status.json`，记录进程、原生菜单栏项可见状态、图标和位置，用于排查托盘不显示；不包含邮件内容。
Windows 也写入同名文件，记录进程、托盘创建状态、浮层状态和图标位置。Windows 可能把新图标放入任务栏的隐藏图标区域。
