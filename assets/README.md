# 应用 Logo

主源文件是 `jianxin-icon.svg`，当前采用用户指定的「来信探头」。PNG、ICO、移动端图标和 Web SVG 都由同一个脚本生成，运行时不依赖外部设计目录。

开发时需要 Python 3.9+、`rsvg-convert` 和 ImageMagick 7。macOS 可运行 `brew install librsvg imagemagick`；其他平台安装对应工具并放入 PATH。普通应用构建使用已生成并提交的资源，不需要这些工具。

在仓库根目录一键替换全部 Logo：

```sh
python3 scripts/generate-icons.py --source /path/to/new-logo.svg
```

给托盘使用另一个透明背景的纯色 SVG：

```sh
python3 scripts/generate-icons.py --tray-source /path/to/monochrome-logo.svg
```

两种参数可以一起传。支持 `--macos-source`、`--windows-source`、`--notification-source`、`--android-source`、`--ios-source`、`--web-source`；例如只让 Web 使用另一张图：

```sh
python3 scripts/generate-icons.py --web-source /path/to/web-logo.svg
```

覆盖源文件会复制到 `assets/logo-overrides/<场景>.svg`，以后换主 Logo 或不带参数重新生成时仍保留。恢复某个场景跟随主 Logo：

```sh
python3 scripts/generate-icons.py --reset web
python3 scripts/generate-icons.py --reset tray --reset android
```

源文件需为包含 viewBox 的自包含 SVG。脚本在写入前完成全部渲染；错误不会覆盖原资源。主源文件会复制进仓库。旧入口 `mobile/tools/generate_icons.py` 和 `scripts/generate-windows-icon.ps1` 调用同一脚本，不再各自绘制旧 Logo。

| 场景 | 输出 / 消费位置 |
| --- | --- |
| 共用彩色图标 | `jianxin-icon.png`，1024 px |
| macOS Dock / Finder / .app / 系统通知 | `jianxin-macos.png`，1024 px，透明外边距 100 px、圆角图块 824 px；打包时生成 `Jianxin.icns` |
| Windows .exe | `jianxin.ico`，16 / 24 / 32 / 48 / 64 / 128 / 256 px；Cargo 自动嵌入 |
| macOS 托盘 | `jianxin-tray-template.png`，36 px；AppKit 按 18 pt 显示并自动适配明暗颜色 |
| Windows / Linux 托盘 | `jianxin-tray.png`，32 px；保持托盘 SVG 的颜色，需自行选择适合系统主题的颜色 |
| Windows / Linux 通知 | `jianxin-notification.png`，256 px；注册通知身份时写入系统图标位置 |
| Android 启动 / 最近任务 / 应用信息 | 自适应图标，432 px 前景，主体位于安全区域；背景取 Android SVG 左上角颜色；主题单色图标共用托盘源 |
| iOS | 不透明 1024 px AppIcon，系统应用圆角 |
| Web / 桌面内置阅读器 | `jianxin-web.svg` 用于应用内 Logo 和 SVG favicon；另导出 `jianxin-favicon.ico` 供外部静态站点使用 |

macOS 通知角标由系统使用 `.app` 图标，跟随 `macos` 场景；无法通过 `--notification-source` 单独替换该角标。当前保留一个来自同一图案的透明单色托盘源，换主图时可同时提供新的 `--tray-source`。

生成资源不会修改已安装或正在运行的应用。替换后重新构建各端并安装新版本；macOS 执行 `bash scripts/build-macos-app.sh`（加 `--install` 安装），Windows 执行 `scripts/build-desktop.ps1`，Android / iOS 构建方式见 [移动端说明](../mobile/README.md)。系统或浏览器缓存图标时，退出旧版本后重新启动或刷新页面。

验证与回归检查：

```sh
python3 scripts/generate-icons.py --check
python3 scripts/test-generate-icons.py
```
