# 简信原生移动客户端

当前应用版本为 `1.0.0`；Android 在首页右上角“设置 → 关于简信”显示构建配置中的版本号。

独立的 iOS / Android 邮箱客户端，直接连接 IMAP 与 SMTP。iOS 使用 SwiftUI、Foundation、CFNetwork 和 Keychain；Android 使用 Kotlin、Material 3 原生控件、SSLSocket 和 Android Keystore。移动应用不加载 Rust、WebView、跨端框架或 Jianxin 服务，也不需要运行桌面端。

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

最低 iOS 17，使用 Xcode 16 或更新版本。直接打开 [Jianxin.xcodeproj](../ios/Jianxin.xcodeproj)，选择 Jianxin scheme。在 Signing & Capabilities 中设置你的开发团队，再选择 iPhone 运行。仓库不包含个人开发者签名配置。

命令行无签名真机编译：

```sh
xcodebuild -project ios/Jianxin.xcodeproj -scheme Jianxin \
  -sdk iphoneos -configuration Release -destination 'generic/platform=iOS' \
  -derivedDataPath /tmp/jianxin-ios-release CODE_SIGNING_ALLOWED=NO build
```

离线协议测试与真实本地 TLS 拒绝测试（macOS）：

```sh
scripts/test_swift.sh
python3 scripts/test_tls.py
```

TLS 测试生成临时自签名证书，在本机启动测试服务器，确认隐式 TLS、STARTTLS 均进行 TLS 握手且拒绝不可信证书。测试不修改系统信任、不连接真实邮箱、不发送真实邮件。

### Android

最低 Android 8 / API 26，compile/target SDK 35。需要 JDK 17、Android SDK Platform 35 与 Build Tools 35，以及已配置的 `ANDROID_HOME`。首次运行 Gradle Wrapper 会下载构建依赖；Android UI 使用 AppCompat、DrawerLayout 和 Material Components，Release 使用 R8 和资源裁剪。

```sh
cd android
./gradlew :app:assembleDebug :app:testDebugUnitTest :app:lintDebug :app:assembleRelease
```

输出：

- `app/build/outputs/apk/debug/app-debug.apk`：可安装调试包。
- `app/build/outputs/apk/release/app-release.apk`：配置发布签名后生成的可安装 Release 包，启用代码与资源裁剪。未配置签名时输出 `app-release-unsigned.apk`，不能直接安装。

本机发布签名配置位于 `android/signing/release.properties`，包含 `storeFile`（相对 `android/` 的密钥库路径）、`storePassword`、`keyAlias` 和 `keyPassword`；密钥库位于 `android/signing/release.jks`。整个 `signing/` 目录已被 Git 忽略，请单独备份并保留密钥，后续版本必须使用同一签名并提高 `versionCode` 才能覆盖升级。分享时仅发送 APK。发布包使用独立签名，不能直接覆盖使用调试签名安装的版本。

验证发布包：

```sh
./gradlew :app:testReleaseUnitTest :app:lintRelease :app:assembleRelease
```

## 此前验证结果

- Android 1.0 签名 Release 构建成功，APK 约 1.82 MiB；70 项 Release JVM 单元测试通过，Release Lint 为 0 错误、49 个警告。APK v2 签名与 ZIP 对齐校验通过，最低 Android 8；分享文件为 `artifacts/jianxin-1.0-release.apk`。此次未覆盖安装手机上的调试版本。
- iOS 真机 arm64 Release 编译成功，未签名 `.app` 目录约 2.0 MB；Swift 离线协议测试及真实本地 TLS / STARTTLS 拒绝证书测试通过。
- Android Debug 构建成功；70 项 JVM 单元测试及 8 项 Android 真机运行检查通过，Lint 为 0 错误、49 个警告。测试覆盖加密文件导入、配置下载、OAuth、IMAP、MIME、SMTP、统一收件箱、提供商元数据、会话分组、回复头、日期回退、时间格式、读取取消、加密存储并发及无额外请求的文件夹计数。更新 UI 后的 APK 包含 Material Components，大小以构建产物为准。
- 应用大小不含用户邮件与附件缓存，iOS `.app` 目录大小与 Android 压缩 APK 大小的口径不同。

## 使用与手动验证

1. 在“邮箱”中添加账号。QQ、163 等先在邮箱设置中开启 IMAP/SMTP 并生成客户端授权码；Gmail 使用支持的应用专用密码或有效 OAuth 访问令牌。
2. 用预设填写服务器，或输入自己的服务器配置，点击“验证并保存”。默认 IMAP 993 / SMTP 465 使用隐式 TLS；需要 143 / 587 时开启相应 STARTTLS。
3. 验证真实邮箱的文件夹、搜索、分页和正文；回复一封自己的测试邮件，再检查回复所属对话、已发送记录和附件。
4. 打开一封邮件后切到离线，再打开缓存邮件；编辑草稿、退出并重进，确认草稿保留。
5. 测试错误授权码、发信拒绝与断网。对“结果未知”的记录先查看已发送文件夹，再决定是否确认重发。

真实服务商兼容性、网页授权和发送邮件应使用自己的测试账号验证；离线协议测试不会发送真实邮件。

## Android 邮件导航与会话

首页为所有邮箱的统一收件箱，按日期排序；每个邮箱独立分页，一处连接失败仍保留其他邮箱的邮件与缓存。列表显示来源邮箱、提供商 logo、未读状态和会话中的已加载邮件数。

左侧抽屉默认显示每个邮箱完整的文件夹列表，包括 Inbox、已发送、草稿、归档、垃圾邮件、已删除和自定义文件夹，无需点击展开箭头。点击邮箱标题进入其收件箱，点击文件夹查看对应邮件。文件夹通过 IMAP LIST 获取并缓存，支持单独刷新。待授权的 Outlook 邮箱点击后进入授权登录。同步中也可以切换邮箱或打开邮件，旧页面的同步结果不会覆盖新页面。

每个邮箱标题行右侧提供“更多”图标，“刷新文件夹”收在其下拉菜单中。待授权或同步失败时图标显示提示颜色，菜单展示邮箱状态，可点击查看完整错误；配置 Microsoft OAuth 的邮箱提供授权登录或重新授权，手动令牌邮箱提供更新访问令牌，密码邮箱出错时提供更新密码 / 授权码，并保留编辑邮箱入口。待授权时暂停文件夹刷新；成功刷新或授权后清除对应邮箱的旧错误。

服务器声明支持 `LIST-STATUS`（或 IMAP4rev2）时，在现有的 LIST 命令附加 `RETURN (STATUS (MESSAGES))`，同一次请求获取各文件夹总邮件数；不支持时继续使用普通 LIST，不发送单独的 STATUS 请求。数字显示最近一次刷新返回的值，未返回数量的文件夹不显示数字，不用已加载的分页数量估算总数。该能力见 [RFC 9051 的 LIST 命令](https://www.rfc-editor.org/rfc/rfc9051.html#section-6.3.9)。

从阅读页返回时直接恢复原列表和滚动位置，不重新读取磁盘或重建抽屉。正文缓存按展开的邮件在后台读取；已缓存正文直接复用，同一会话的查询结果复用 2 分钟，阅读页“刷新”会重新读取。离开页面时取消排队任务并关闭其 IMAP 连接，后台结果不会覆盖用户的新页面。缓存列表与会话索引只保存邮件头，完整正文和附件单独保存；加密存储按记录同步，缓存写入不占用界面线程。SMTP 投递等写操作不随阅读任务取消。

点击列表打开邮件会话，按时间排列各封邮件，默认展开最新一封；点击其他邮件头展开其完整正文和附件。会话加载会补查该邮箱各文件夹，包含已发送回复，按 Message-ID 去除跨文件夹副本。聚合参考 Web 端：Gmail X-GM-THRID、IMAP THREAD=REFERENCES、Message-ID / References / In-Reply-To、Outlook Thread-Index 根以及规范化主题；不同邮箱隔离，空主题不会相互合并。列表中的数量是已加载数量，打开会话后更新为查询结果数量。

阅读页底部保留当前邮件的“回复”和“回复全部”图标；顶部提供刷新和“更多”下拉菜单，转发、星标、标记已读、移动与删除收在菜单内。附件显示为可点击的标签。回复保留所属邮箱、In-Reply-To 和 References，优先使用 Reply-To，回复全部去重并排除自己的地址。编辑页自动保存本地草稿，点击发送才提交 SMTP。

邮箱管理、账号编辑、密文导入与预览、OAuth 授权、时间设置、草稿和发送记录均使用 MD3 分组卡片与菜单。导入和服务商预设采用可展开下拉字段；主要表单操作固定在底部。写信页通过顶部图标添加附件或发送，抄送/密送按需展开，附件标签支持移除，“保存并关闭”和“丢弃草稿”位于更多菜单。

首页顶部提示可点击右侧关闭图标收起。关闭不影响邮箱同步；下一次主动刷新重新显示最新结果。底部“写邮件”右侧的 Plus 图标直接进入添加邮箱的服务商选择页。

首页右上角菜单中的“设置”可配置时间格式。默认 24 小时制；列表当天显示 `HH:mm`，其他日期显示 `MM-dd HH:mm`，阅读页显示 `yyyy-MM-dd HH:mm`。可切换 12 小时制或填写自定义格式，如 `yyyy-MM-dd HH:mm:ss`；设置保存在手机本地，按设备时区显示。“管理邮箱”和“设置”仅保留在首页右上角菜单，抽屉用于邮箱、文件夹及本地草稿/发件记录导航。

邮件日期优先读取邮件头 `Date`，缺失或无效时使用 IMAP `INTERNALDATE`，不使用同步时间。两个日期都不可用时显示“时间未知”。旧缓存中的日期会重新获取，已有正文和附件保留。比如缺少 `Date` 的邮件，其服务器日期 `25-Sep-2026 09:27:50 +0800` 会按设备时区显示为正确时间。

“管理邮箱 → 添加邮箱”先选择 QQ、163、126、Gmail、Outlook、阿里云或其他邮箱，再填写邮箱地址、可选名称及密码/客户端授权码。服务商预填的服务器、端口、TLS、独立登录用户名和 OAuth 应用配置收在“高级设置”；其他邮箱默认展开。Outlook 可不填密码或令牌，保存后进行授权登录。

Android 的 MD3 导航与多邮箱交互参考 [Thunderbird for Android](https://github.com/thunderbird/thunderbird-android) 的统一收件箱与邮箱抽屉模式。提供商映射沿用 `webpc/packages/jxui/core.js` 的 `platform` 优先、`provider` 回退规则；图形由 `assets/mail-providers/svg-exact` 生成原生 VectorDrawable，在抽屉、邮件列表与阅读页显示。旧版本没有保存这两个字段的账号按邮箱域名回退，自定义服务商显示通用邮件图标。

## 密码管理器接入

iOS 在系统设置中启用密码自动填充和你使用的密码管理器，然后在邮箱设置的密码字段点击键盘“密码”选择条目。Android 在系统设置中选择自动填充服务，然后在邮箱设置点击“从密码管理器填入”。自定义登录用户名不为空时，自动填充使用该用户名字段；否则使用邮箱地址字段。

客户端授权码需要先由邮箱服务商生成，再保存为密码管理器条目的密码。选择条目后，应用仍使用既有 Keychain / Keystore 方式保存连接所需凭据；不会读取整个密码库。OAuth 令牌可以保存为条目中的密码并填入，但密码管理器不负责邮箱的 OAuth 授权或令牌刷新。

当前未配置服务商域名的 Associated Domains / Digital Asset Links，因此不承诺自动匹配 QQ、Gmail 等网站的所有现有密码；可以在密码管理器的选择界面手动查找条目。不同系统与提供方的实际选择界面需在真机验证。

实现依据：[Apple Password AutoFill](https://developer.apple.com/documentation/security/enabling-password-autofill-on-a-text-input-view)、[Android Autofill](https://developer.android.com/identity/autofill/autofill-optimize)。

## 在 Android 上授权 Outlook 邮箱

Outlook / Microsoft 365 的 OAuth 配置可以没有访问令牌或明文密码。先导入加密配置并保存账号，再在“邮箱”列表点击标为“待授权”的邮箱：

1. 点击“开始授权登录”，获得 Microsoft 授权码。
2. 复制授权码，点击“打开 Microsoft 授权页面”，在网页输入授权码，使用此邮箱登录并同意权限。
3. 返回简信。应用会自动保存访问令牌和刷新令牌并读取邮箱；账号、令牌仍由 Android Keystore 加密保存。

导入器读取 `auth.client_id`、`token_endpoint` 和 `scopes`。Outlook 未指定端点时使用 `consumers` 的 Microsoft v2.0 端点，默认请求 IMAP、SMTP 和 `offline_access` 权限。组织邮箱应使用配置中的 `common`、`organizations` 或租户端点。OAuth 应用必须支持公共客户端设备授权；配置缺少 Client ID 时，先在邮箱编辑页填写应用 ID。手动添加时可选择“Outlook”服务器预设并启用“使用 OAuth 授权登录”。应用配置详见 [Outlook 接入说明](../docs/outlook-oauth2.md)。

手机获得自己的令牌，不执行桌面端的凭据命令，也不读取桌面端文件 / Keychain 引用或迁移刷新令牌。访问令牌临近到期时，在下一次收发邮件前刷新并保存轮换后的令牌；刷新令牌失效时回到授权页。相同邮箱、服务器、用户名和 OAuth 配置重复导入时保留手机上已有的授权；更换 Client ID、端点或权限时需要重新授权。取消、拒绝、超时和网络失败会显示提示，可重新开始；离开授权页面会停止轮询。iOS 与其他服务商暂不支持此手机授权流程。

实现依据：[Microsoft 设备授权](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-device-code)、[IMAP/SMTP OAuth 权限](https://learn.microsoft.com/en-us/exchange/client-developer/legacy-protocols/how-to-authenticate-an-imap-pop-smtp-application-by-using-oauth)、[刷新令牌请求](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-auth-code-flow#refresh-the-access-token)。

## 当前边界

- 两端支持密码、客户端授权码和已有 OAuth 访问令牌的 XOAUTH2 登录。Android 另支持 Outlook / Microsoft 365 的设备授权登录和令牌刷新；iOS 与其他服务商仍需提供有效访问令牌。
- 同步在应用前台手动触发；尚未实现后台推送、新邮件通知与 IMAP IDLE。
- 草稿保存在手机本地，不同步到服务器草稿箱；服务器草稿文件夹中的邮件可阅读。
- 正文显示原生文本，不还原 HTML 布局或加载外部图片。附件从完整邮件中解析，不做单个 MIME 部分的延迟下载。
- 单封邮件上限 25 MB；添加单个附件上限 15 MB，附件合计上限 18 MB。服务器的限制可能更小。
- Android 手机自带的邮箱数据无法自动导入；可手动添加账号或导入加密 config.json。

## 目录与设计

根目录的 `ios/Jianxin` 是独立 Swift 实现；`android/app` 是独立 Kotlin 实现。`scripts/` 保存配置加密、资源生成与测试入口，`tests/mobile/` 保存两端共用测试与测试数据，`docs/mobile.design` 保存共用设计契约。两端协议代码独立，测试均覆盖 MIME、回复头、密送、UIDVALIDITY、UIDPLUS 安全移动和 SMTP 接受/拒绝/未知结果。

Android 使用 `Theme.Material3.Light.NoActionBar` 和 [mobile.design](mobile.design) 的 `platforms.android` 色彩角色，通过主题生成器生成 Kotlin / XML 常量；iOS 继续使用现有 Web 端设计色彩。只更新 Android 主题与提供商图标时运行：

```sh
python3 scripts/generate_theme.py --platform android
python3 scripts/generate_provider_icons.py
```

同时重新生成两端主题及其他资源：

```sh
python3 scripts/generate_theme.py
python3 scripts/generate-icons.py
python3 scripts/generate_xcode_project.py
```

这些脚本只在开发时使用；安装后的应用不依赖 Python、前端或仓库路径。

协议参考：[IMAP RFC 9051](https://www.rfc-editor.org/rfc/rfc9051.html)、[SMTP RFC 5321](https://www.rfc-editor.org/rfc/rfc5321.html)、[邮件格式 RFC 5322](https://www.rfc-editor.org/rfc/rfc5322.html)、[MIME RFC 2045](https://www.rfc-editor.org/rfc/rfc2045.html)、[参数编码 RFC 2231](https://www.rfc-editor.org/rfc/rfc2231.html)。

## 从 CDN 导入加密 config.json

Android 也可在“邮箱 → 从密文文件导入”使用系统文件选择器打开本地或文档提供方的 `.txt`、`.enc`、`.json` 文件，选择生成文件时的加密方式并输入解密密码，点击“读取文件并解密”。读取和解密在本机完成，随后预览并选择要保存的账号，导入邮箱配置和密码 / OAuth 访问令牌。支持 AES-256-GCM、AES-128-GCM 与旧版 JSON，文件上限为 2 MB。导入本地文件不会覆盖已记住的 CDN URL；错误密码或损坏文件不会修改已有账号。

两端均可在“邮箱 → 从 URL 导入加密配置”填写 HTTPS URL、选择加密方式并输入解密密码，下载后在本机解密，再预览、选择并保存账号。不会删除配置中未出现的现有账号。同一邮箱地址、IMAP 主机和 IMAP 登录用户名匹配时更新账号，并保留本地账号 ID、邮件缓存和草稿；收发邮件时再验证连接。更换收信服务器或登录用户名会新增账号。

先在电脑上使用配套加密工具（仅需 Node.js 18+，无 npm 依赖）：

```sh
node scripts/config_crypto.mjs encrypt \
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

- 导入 `email`、`display_name`（缺省显示 `id`）、`auth.username`、`password/app_password` 的字符串密码或 `oauth2/o_auth2` 的字符串 `access_token`。Android 的 OAuth 账号没有字符串访问令牌时也可保存为“待授权”。Microsoft 账号同时保留 `auth.client_id`、`token_endpoint`、`scopes`，用于手机上的新授权。
- 未指定 `imap` / `smtp` 时按桌面端相同的服务商表补齐，未知域名推测 `imap.<domain>` / `smtp.<domain>`。显式服务器支持 `host`、`port`、`tls`（`tls` / `starttls` / `start_tls`）和独立 `username`；支持 `folders` 的文件夹角色覆盖。
- 不执行 `command` / `token_command`，不读取电脑的 `env`、`file`、`keychain` 凭据引用。密码账号需先在待加密的配置中填入字符串凭据；Android OAuth 账号可以先导入配置再授权。不导入桌面端刷新令牌或 `client_secret`；手机通过公共客户端授权获得自己的令牌。
- `general` 的 API token、Agent、桌面路径、同步策略等不应用到手机。iOS OAuth 仅导入已有访问令牌；明文邮箱连接不导入。
- URL 可包含 CDN 签名查询参数；不允许 URL 用户名、密码或片段。只接受 HTTPS，始终校验系统证书，重定向最多 5 次且不得降级到 HTTP；HTTP 错误和超过 2 MB 的响应会终止导入。配置支持 1 至 100 个账号。
- URL 和所选加密方式保存在既有受保护的本地存储中，解密密码每次输入，不上传、不保存。明文配置不写入文件；选中的账号仍存入 iOS Keychain / Android Keystore 加密存储。

新格式为 `Base64(nonce || ciphertext || tag)`，其中 nonce 为每次加密新生成的 12 字节随机值，认证标签为 16 字节。文件不含随机 salt、算法名称、版本、KDF 名称或迭代次数字段；手机完全按所选算法解析，不自动推断或尝试其他算法。Base64 是传输编码，解码后的 nonce 和 tag 属于解密所需的非秘密参数。

密码仍按 UTF-8 编码，使用 PBKDF2-HMAC-SHA256（600000 次）派生密钥。AES-256-GCM 为 32 字节密钥，AES-128-GCM 为 16 字节密钥。派生的固定应用上下文为 `jianxin-config:password-key:v2:<cipher>`（作为 PBKDF2 的固定 salt 参数），GCM AAD 为 `jianxin-config:v2:pbkdf2-sha256:600000:<cipher>`，其中 `<cipher>` 为 `aes-256-gcm` 或 `aes-128-gcm`。解密时使用相同固定上下文，不生成新 salt 或 nonce。

这满足文件无 salt 字段、手机只选算法并输入密码的操作需求。固定上下文不会提供每文件随机 salt 对批量预计算的隔离，同一算法下相同密码派生同一密钥；随机 nonce 确保多次加密产生不同密文。[RFC 8018](https://www.rfc-editor.org/rfc/rfc8018.html) 中的 salt 本身无需保密，它在密码派生中也不用于登录比对。

之前的 JSON 文件仍可在手机手动选择“旧版 JSON（兼容）”导入，保持既有用户文件可读；新的发布工具只生成纯 Base64 文件。更新后的客户端需重新安装；旧版客户端无法解析新格式。

非浏览器验证：

```sh
node tests/mobile/config_crypto.test.mjs
scripts/test_swift.sh
python3 scripts/test_tls.py
```

跨端测试使用合成配置和中文/emoji 密码，覆盖发布工具 → Swift / Kotlin 两种算法的解密互通、错误密码、错误算法、nonce/密文/标签修改、格式校验、账号合并与旧凭据兼容、下载状态、响应大小及重定向限制。没有下载你的 CDN 或向真实邮箱发信；两端的导入界面与 CDN 更新需在设备上手动验证。

### 在 Android 设备上检查 IMAP 兼容性

仅连接一台设备，在 `android` 下执行以下命令。Windows 使用 `gradlew.bat` 替代 `./gradlew`。安装采用覆盖更新方式，保留应用的本地账号。

```bash
./gradlew :app:assembleDebug :app:assembleDebugAndroidTest
adb install -r app/build/outputs/apk/debug/app-debug.apk
adb install -r app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk
adb shell am instrument -w com.jianxin.mail.test/com.jianxin.mail.ImapRuntimeChecks
```

成功时输出 `Android IMAP runtime checks: 8/8 passed`。检查使用合成 IMAP 响应、本机回环连接及临时加密记录，覆盖登录后的 CAPABILITY / ID、文件夹列表、邮件头 literal 读取、中文主题、会话分组、回复头、无 Date 头的服务器日期回退、阻塞读取取消、加密记录的并发读写及无额外请求的文件夹计数，不连接真实邮箱。临时加密记录在测试结束后删除。Android 的 ICU 正则引擎对花括号的转义要求与桌面 JVM 不同；literal 标记中的左右花括号均需转义，不能仅依靠 JVM 单元测试验证。应用错误日志只记录异常类型和调用位置，不记录密码、令牌、协议内容或邮件正文。测试后可以 `adb uninstall com.jianxin.mail.test` 移除测试包；不要卸载主应用。
