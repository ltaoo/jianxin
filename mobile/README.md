# 简信原生移动客户端

独立的 iOS / Android 邮箱客户端，直接连接 IMAP 与 SMTP。iOS 使用 SwiftUI、Foundation、CFNetwork 和 Keychain；Android 使用 Kotlin、系统原生控件、SSLSocket 和 Android Keystore。移动应用不加载 Rust、WebView、跨端框架或 Jianxin 服务，也不需要运行桌面端。

| 能力 | 两端行为 |
| --- | --- |
| 密码管理器 | iOS Password AutoFill；Android 系统 Autofill 提示与手动唤起；验证成功后提交保存提示；兼容已启用的自动填充提供方 |
| 邮箱管理 | 多账号；QQ、163、126、Gmail、阿里云预设；自定义服务器、用户名、端口；连接验证与账号移除 |
| 安全连接 | IMAP / SMTP 隐式 TLS 或 STARTTLS；验证证书链与服务器名称；禁止明文登录 |
| 邮件列表 | 文件夹切换、每页 30 封、继续加载、未读筛选、服务器全文搜索、手动刷新 |
| 邮件阅读 | 完整正文转为可选择的原生文本；中文编码、MIME 多部分、HTML 转文本；读取后同步已读 |
| 回复与发信 | 回复、回复全部、转发、新邮件；收件人、抄送、密送；保留 Message-ID / In-Reply-To / References |
| 附件 | 读取、导出、添加、移除；支持 Base64、quoted-printable、RFC 2231 中文文件名 |
| 邮件管理 | 星标、已读/未读、移动；删除操作移到已删除文件夹，先确认 |
| 离线使用 | 即时显示已有列表及正文缓存；本地草稿自动保存；移除邮箱同时删除该邮箱缓存与草稿 |
| 发送记录 | 持久化发送中、已发送、失败、结果未知；失败手动重试；未知结果须再次确认 |

密送地址只进入 SMTP 信封，不写入邮件头。SMTP 的最终 `250` 响应才代表确认投递；写入 DATA 后连接断开会显示“结果未知”，不会自动重发。应用在发送中被结束，下次启动也会恢复为“结果未知”。保存已发送文件夹副本失败不会把已经投递的邮件变成可重试状态；保存副本前按 Message-ID 检查现有邮件。

IMAP 操作检查 UIDVALIDITY，避免邮箱重建后操作旧 UID。移动优先使用 MOVE；仅支持 UIDPLUS 的服务器使用 COPY + STORE + UID EXPUNGE，清理指定邮件。两者都不支持时显示错误，不执行全邮箱 EXPUNGE。

iOS 凭据仅存于设备 Keychain，缓存和草稿启用系统文件保护且排除备份。Android 凭据、缓存与草稿使用设备 Keystore 密钥和 AES-GCM 加密，放在应用私有的不备份目录。

## 构建与测试

### iOS

最低 iOS 17，使用 Xcode 16 或更新版本。直接打开 [Jianxin.xcodeproj](ios/Jianxin.xcodeproj)，选择 Jianxin scheme。在 Signing & Capabilities 中设置你的开发团队，再选择 iPhone 运行。仓库不包含个人开发者签名配置。

命令行无签名真机编译：

```sh
xcodebuild -project mobile/ios/Jianxin.xcodeproj -scheme Jianxin \
  -sdk iphoneos -configuration Release -destination 'generic/platform=iOS' \
  -derivedDataPath /tmp/jianxin-ios-release CODE_SIGNING_ALLOWED=NO build
```

离线协议测试与真实本地 TLS 拒绝测试（macOS）：

```sh
mobile/tools/test_swift.sh
python3 mobile/tools/test_tls.py
```

TLS 测试生成临时自签名证书，在本机启动测试服务器，确认隐式 TLS、STARTTLS 均进行 TLS 握手且拒绝不可信证书。测试不修改系统信任、不连接真实邮箱、不发送真实邮件。

### Android

最低 Android 8 / API 26，compile/target SDK 35。需要 JDK 17、Android SDK Platform 35 与 Build Tools 35，以及已配置的 `ANDROID_HOME`。首次运行 Gradle Wrapper 会下载构建依赖；应用运行时只依赖系统与编译后的 Kotlin 标准库，Release 使用 R8 和资源裁剪。

```sh
cd mobile/android
./gradlew :app:assembleDebug :app:testDebugUnitTest :app:lintDebug :app:assembleRelease
```

输出：

- `app/build/outputs/apk/debug/app-debug.apk`：可安装调试包。
- `app/build/outputs/apk/release/app-release-unsigned.apk`：已裁剪的无签名 Release 包。
- `../artifacts/jianxin-android-test.apk`：本次工作生成的已裁剪测试包，使用开发测试签名；不作为正式发布签名。

## 本次验证结果

- iOS 真机 arm64 Release 编译成功，未签名 `.app` 目录约 2.0 MB；Swift 离线协议测试及真实本地 TLS / STARTTLS 拒绝证书测试通过。
- Android Debug / Release 编译成功，24 项协议、加密导入与下载测试通过，Lint 无问题。签名并裁剪后的测试 APK 为 103,871 字节，约 101 KiB；APK v2/v3 签名和 zipalign 验证通过。
- 上述大小不含用户邮件与附件缓存，iOS `.app` 目录大小与 Android 压缩 APK 大小的口径不同。

## 使用与手动验证

1. 在“邮箱”中添加账号。QQ、163 等先在邮箱设置中开启 IMAP/SMTP 并生成客户端授权码；Gmail 使用支持的应用专用密码或有效 OAuth 访问令牌。
2. 用预设填写服务器，或输入自己的服务器配置，点击“验证并保存”。默认 IMAP 993 / SMTP 465 使用隐式 TLS；需要 143 / 587 时开启相应 STARTTLS。
3. 验证真实邮箱的文件夹、搜索、分页和正文；回复一封自己的测试邮件，再检查回复所属对话、已发送记录和附件。
4. 打开一封邮件后切到离线，再打开缓存邮件；编辑草稿、退出并重进，确认草稿保留。
5. 测试错误授权码、发信拒绝与断网。对“结果未知”的记录先查看已发送文件夹，再决定是否确认重发。

未自动操作浏览器或真机界面；真实服务商兼容性和实际交互需要手动验证。

## 密码管理器接入

iOS 在系统设置中启用密码自动填充和你使用的密码管理器，然后在邮箱设置的密码字段点击键盘“密码”选择条目。Android 在系统设置中选择自动填充服务，然后在邮箱设置点击“从密码管理器填入”。自定义登录用户名不为空时，自动填充使用该用户名字段；否则使用邮箱地址字段。

客户端授权码需要先由邮箱服务商生成，再保存为密码管理器条目的密码。选择条目后，应用仍使用既有 Keychain / Keystore 方式保存连接所需凭据；不会读取整个密码库。OAuth 令牌可以保存为条目中的密码并填入，但密码管理器不负责邮箱的 OAuth 授权或令牌刷新。

当前未配置服务商域名的 Associated Domains / Digital Asset Links，因此不承诺自动匹配 QQ、Gmail 等网站的所有现有密码；可以在密码管理器的选择界面手动查找条目。不同系统与提供方的实际选择界面需在真机验证。

实现依据：[Apple Password AutoFill](https://developer.apple.com/documentation/security/enabling-password-autofill-on-a-text-input-view)、[Android Autofill](https://developer.android.com/identity/autofill/autofill-optimize)。

## 当前边界

- 支持密码、客户端授权码和已有 OAuth 访问令牌的 XOAUTH2 登录；尚未实现 OAuth 授权流程与自动刷新令牌，令牌过期需手动更新。
- 同步在应用前台手动触发；尚未实现后台推送、新邮件通知与 IMAP IDLE。
- 草稿保存在手机本地，不同步到服务器草稿箱；服务器草稿文件夹中的邮件可阅读。
- 正文显示原生文本，不还原 HTML 布局或加载外部图片。附件从完整邮件中解析，不做单个 MIME 部分的延迟下载。
- 单封邮件上限 25 MB；添加单个附件上限 15 MB，附件合计上限 18 MB。服务器的限制可能更小。
- Android 手机自带的邮箱数据无法自动导入；可手动添加账号或导入加密 config.json。

## 目录与设计

`ios/Jianxin` 是独立 Swift 实现；`android/app` 是独立 Kotlin 实现。两端协议代码独立，测试均覆盖 MIME、回复头、密送、UIDVALIDITY、UIDPLUS 安全移动和 SMTP 接受/拒绝/未知结果。

视觉沿用现有 `frontend/.design` 的 `primary`、`canvas`、`text`、`muted`、`fill` 与 spacing tokens，通过 `tools/generate_theme.py` 生成平台常量；[.design](.design) 声明原生 Mailbox、Composer 和 DeliveryQueue。可选重新生成命令：

```sh
python3 mobile/tools/generate_theme.py
python3 mobile/tools/generate_icons.py
python3 mobile/tools/generate_xcode_project.py
```

这些脚本只在开发时使用；安装后的应用不依赖 Python、前端或仓库路径。

协议参考：[IMAP RFC 9051](https://www.rfc-editor.org/rfc/rfc9051.html)、[SMTP RFC 5321](https://www.rfc-editor.org/rfc/rfc5321.html)、[邮件格式 RFC 5322](https://www.rfc-editor.org/rfc/rfc5322.html)、[MIME RFC 2045](https://www.rfc-editor.org/rfc/rfc2045.html)、[参数编码 RFC 2231](https://www.rfc-editor.org/rfc/rfc2231.html)。

## 从 CDN 导入加密 config.json

两端均可在“邮箱 → 从 URL 导入加密配置”填写 HTTPS URL、选择加密方式并输入解密密码，下载后在本机解密，再预览、选择并保存账号。不会删除配置中未出现的现有账号。同一邮箱地址、IMAP 主机和 IMAP 登录用户名匹配时更新账号，并保留本地账号 ID、邮件缓存和草稿；收发邮件时再验证连接。更换收信服务器或登录用户名会新增账号。

先在电脑上使用配套加密工具（仅需 Node.js 18+，无 npm 依赖）：

```sh
node mobile/tools/config_crypto.mjs encrypt \
  /Users/litao/.config/jianxin/config.json \
  /private/tmp/jianxin-config.enc.txt
```

默认使用 AES-256-GCM，生成的文件只有一段 Base64 密文串，没有 JSON 字段或算法标识。工具会要求输入并确认密码，输入不回显；不会从命令行参数读取密码。将生成的 `/private/tmp/jianxin-config.enc.txt` 上传到 CDN，然后把 HTTPS 地址填入手机，选择 AES-256-GCM 并输入密码。使用 `--cipher aes-128-gcm` 可改为 AES-128-GCM，手机必须选择相同方式。更新同一个输出文件时在命令末尾加 `--force`，再上传覆盖 CDN 文件。应用每次主动导入都会重新下载；如果 CDN 仍提供旧文件，请刷新 CDN 缓存或在 URL 加上版本查询参数，例如 `?v=20261011`。

自动化发布可通过 `JIANXIN_CONFIG_PASSWORD` 环境变量提供密码，应从发布系统的 Secret 配置读取。发布工具会解密自检后写入密文，并以 0600 权限新建输出文件；请只上传密文文件。密码用于抵抗 CDN 密文的离线猜测，应使用长且独立的密码。

解密后使用现有桌面端的 JSON 格式，最小示例如下（全部为示例数据）：

```json
{
  "accounts": [
    {
      "id": "personal",
      "email": "demo@qq.com",
      "auth": { "type": "app_password", "password": "example-authorization-code" }
    }
  ]
}
```

- 导入 `email`、`display_name`（缺省显示 `id`）、`auth.username`、`password/app_password` 的字符串密码或 `oauth2/o_auth2` 的字符串 `access_token`。
- 未指定 `imap` / `smtp` 时按桌面端相同的服务商表补齐，未知域名推测 `imap.<domain>` / `smtp.<domain>`。显式服务器支持 `host`、`port`、`tls`（`tls` / `starttls` / `start_tls`）和独立 `username`；支持 `folders` 的文件夹角色覆盖。
- 不执行 `command` / `token_command`，不读取电脑的 `env`、`file`、`keychain` 凭据引用。需先在待加密的配置中填入字符串凭据；不支持的账号在预览中提示并排除，其他账号仍可导入。
- `general` 的 API token、Agent、桌面路径、同步策略等不应用到手机。OAuth 只导入已有访问令牌，不导入自动刷新流程；明文邮箱连接不导入。
- URL 可包含 CDN 签名查询参数；不允许 URL 用户名、密码或片段。只接受 HTTPS，始终校验系统证书，重定向最多 5 次且不得降级到 HTTP；HTTP 错误和超过 2 MB 的响应会终止导入。配置支持 1 至 100 个账号。
- URL 和所选加密方式保存在既有受保护的本地存储中，解密密码每次输入，不上传、不保存。明文配置不写入文件；选中的账号仍存入 iOS Keychain / Android Keystore 加密存储。

新格式为 `Base64(nonce || ciphertext || tag)`，其中 nonce 为每次加密新生成的 12 字节随机值，认证标签为 16 字节。文件不含随机 salt、算法名称、版本、KDF 名称或迭代次数字段；手机完全按所选算法解析，不自动推断或尝试其他算法。Base64 是传输编码，解码后的 nonce 和 tag 属于解密所需的非秘密参数。

密码仍按 UTF-8 编码，使用 PBKDF2-HMAC-SHA256（600000 次）派生密钥。AES-256-GCM 为 32 字节密钥，AES-128-GCM 为 16 字节密钥。派生的固定应用上下文为 `jianxin-config:password-key:v2:<cipher>`（作为 PBKDF2 的固定 salt 参数），GCM AAD 为 `jianxin-config:v2:pbkdf2-sha256:600000:<cipher>`，其中 `<cipher>` 为 `aes-256-gcm` 或 `aes-128-gcm`。解密时使用相同固定上下文，不生成新 salt 或 nonce。

这满足文件无 salt 字段、手机只选算法并输入密码的操作需求。固定上下文不会提供每文件随机 salt 对批量预计算的隔离，同一算法下相同密码派生同一密钥；随机 nonce 确保多次加密产生不同密文。[RFC 8018](https://www.rfc-editor.org/rfc/rfc8018.html) 中的 salt 本身无需保密，它在密码派生中也不用于登录比对。

之前的 JSON 文件仍可在手机手动选择“旧版 JSON（兼容）”导入，保持既有用户文件可读；新的发布工具只生成纯 Base64 文件。更新后的客户端需重新安装；旧版客户端无法解析新格式。

非浏览器验证：

```sh
node mobile/tests/config_crypto.test.mjs
mobile/tools/test_swift.sh
python3 mobile/tools/test_tls.py
```

跨端测试使用合成配置和中文/emoji 密码，覆盖发布工具 → Swift / Kotlin 两种算法的解密互通、错误密码、错误算法、nonce/密文/标签修改、格式校验、账号合并与旧凭据兼容、下载状态、响应大小及重定向限制。没有下载你的 CDN 或向真实邮箱发信；两端的导入界面与 CDN 更新需在设备上手动验证。
