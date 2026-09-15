# 多邮件厂商 Email Client 架构设计

**版本：** v1.0  
**目标：** 构建支持 Gmail、Outlook/Microsoft 365、QQ、163、阿里云及通用 IMAP/SMTP 邮箱的现代 Email Client。

---

# 1. 设计目标

## 1.1 核心目标

客户端需要支持：

- 多邮箱账户
- 多邮件厂商
- OAuth 2.0
- 用户名/密码或 App Password
- IMAP
- SMTP
- 厂商原生 API
- 增量同步
- 邮件发送
- 附件
- 文件夹
- 已读/未读
- Flag / Star
- 移动
- 删除
- Draft
- Reply / Forward
- 搜索
- 多账户统一 Inbox
- 离线缓存
- Push / IDLE / Polling

同时避免：

> **所有邮箱都必须实现 IMAP**

这个设计会导致 Gmail / Outlook 的能力被最低协议能力限制。

---

# 2. 总体架构

推荐采用：

```text
┌──────────────────────────────────────────────┐
│                    UI Layer                  │
│                                              │
│  Inbox │ Mail │ Compose │ Search │ Settings │
└──────────────────────┬───────────────────────┘
                       │
                       ▼
┌──────────────────────────────────────────────┐
│                 Application Layer            │
│                                              │
│  MailService                                 │
│  AccountService                              │
│  SyncService                                 │
│  SearchService                               │
│  ComposeService                              │
│  NotificationService                         │
└──────────────────────┬───────────────────────┘
                       │
                       ▼
┌──────────────────────────────────────────────┐
│                  Domain Layer                │
│                                              │
│  Account                                      │
│  Folder                                       │
│  Message                                      │
│  Thread                                       │
│  Attachment                                   │
│  Draft                                        │
│                                              │
│  MailProvider Interface                       │
└───────────────┬──────────────────────────────┘
                │
        ┌───────┼───────────────┐
        ▼       ▼               ▼
┌──────────┐ ┌──────────┐ ┌──────────────┐
│ Outlook  │ │  Gmail   │ │ Generic IMAP │
│ Provider │ │ Provider │ │   Provider   │
└────┬─────┘ └────┬─────┘ └──────┬───────┘
     │             │              │
     ▼             ▼              ▼
 Graph API      Gmail API     IMAP / SMTP
 OAuth          OAuth         OAuth/Password
```

关键点：

> **Provider 是基础设施，不是 Domain Model。**

---

# 3. Provider 架构

定义统一 Provider：

```ts
interface MailProvider {
  authenticate(): Promise<AuthSession>

  getAccount(): Promise<Account>

  listFolders(): Promise<Folder[]>

  getFolder(folderId: string): Promise<Folder>

  listMessages(
    folderId: string,
    options?: ListMessagesOptions
  ): Promise<MessagePage>

  getMessage(
    messageId: string
  ): Promise<Message>

  sendMessage(
    message: ComposeMessage
  ): Promise<SendResult>

  createDraft(
    message: ComposeMessage
  ): Promise<Message>

  updateDraft(
    messageId: string,
    message: ComposeMessage
  ): Promise<Message>

  reply(
    messageId: string,
    message: ComposeMessage
  ): Promise<Message>

  forward(
    messageId: string,
    message: ComposeMessage
  ): Promise<Message>

  markRead(
    messageId: string,
    read: boolean
  ): Promise<void>

  setFlag(
    messageId: string,
    flag: boolean
  ): Promise<void>

  moveMessage(
    messageId: string,
    folderId: string
  ): Promise<void>

  deleteMessage(
    messageId: string
  ): Promise<void>

  search(
    query: SearchQuery
  ): Promise<MessagePage>

  sync(
    state: SyncState
  ): Promise<SyncResult>

  watch?(
    options: WatchOptions
  ): Promise<WatchHandle>
}
```

但这里有一个非常重要的设计：

**不要假设所有 Provider 都支持所有能力。**

例如：

```ts
interface ProviderCapabilities {
  search: boolean
  push: boolean
  deltaSync: boolean
  threads: boolean
  labels: boolean
  categories: boolean
  drafts: boolean
  html: boolean
  attachments: boolean
}
```

Provider 初始化后返回：

```ts
{
  search: true,
  push: true,
  deltaSync: true,
  threads: true,
  labels: false,
  categories: true,
  drafts: true,
  html: true,
  attachments: true
}
```

UI / Application Layer 根据 Capability 决定是否启用高级功能。

---

# 4. Authentication 不应该和 Mail Provider 强绑定

这是整个架构最重要的地方之一。

不要：

```text
OutlookProvider
   └── login()
```

而应该：

```text
Authentication
      │
      ├── OAuth2
      ├── Password
      ├── AppPassword
      └── APIKey
```

定义：

```ts
interface AuthProvider {
  authorize(): Promise<AuthSession>

  refresh(
    session: AuthSession
  ): Promise<AuthSession>

  revoke?(
    session: AuthSession
  ): Promise<void>
}
```

然后：

```text
Microsoft
    └── MicrosoftOAuth

Google
    └── GoogleOAuth

Generic IMAP
    ├── OAuth2
    ├── Password
    └── AppPassword
```

这样未来支持：

```text
Yahoo OAuth
Zoho OAuth
GitHub SMTP
企业 SSO
```

都不需要修改 Mail Domain。

---

# 5. Provider Registry

客户端启动时注册 Provider：

```ts
providerRegistry.register({
  id: "microsoft",
  matcher: [
    "outlook.com",
    "hotmail.com",
    "live.com"
  ],
  provider: MicrosoftProvider
})

providerRegistry.register({
  id: "google",
  matcher: [
    "gmail.com",
    "googlemail.com"
  ],
  provider: GmailProvider
})

providerRegistry.register({
  id: "generic-imap",
  provider: GenericIMAPProvider
})
```

用户输入：

```text
funzm@outlook.com
```

执行：

```text
detectProvider(email)
        │
        ▼
    microsoft
        │
        ▼
MicrosoftProvider
```

---

# 6. 不要仅靠邮箱域名判断 Provider

这一点也很重要。

例如：

```text
user@example.com
```

可能是：

```text
Google Workspace
Microsoft 365
企业 Exchange
自建邮箱
阿里云企业邮箱
腾讯企业邮箱
```

所以 Provider Detection 应该分成多级：

```text
1. Domain Known
       ↓
2. MX / Autodiscover / Well-known discovery
       ↓
3. OAuth provider discovery
       ↓
4. Generic IMAP detection
       ↓
5. Manual configuration
```

例如：

```text
tao@company.com
      │
      ▼
company.com
      │
      ├── Microsoft 365?
      │
      ├── Google Workspace?
      │
      ├── Aliyun?
      │
      ├── Tencent?
      │
      └── Generic IMAP?
```

---

# 7. Outlook / Microsoft Provider

Outlook 应作为一个独立 Provider。

```text
MicrosoftProvider
       │
       ├── Authentication
       │       └── Microsoft OAuth 2.0
       │
       ├── Mail
       │       └── Microsoft Graph
       │
       ├── Sync
       │       ├── delta
       │       └── webhook
       │
       └── Advanced
               ├── categories
               ├── conversation
               └── Outlook properties
```

Microsoft Graph 可以直接访问个人 Microsoft Account 和组织账户中的 Outlook Mail，并支持邮件文件夹、消息、Draft、Reply、Forward、Send、Delete、增量同步以及 webhook 通知等能力。

因此：

> **Outlook 第一实现不建议走 IMAP。**

---

# 8. Microsoft OAuth

认证流程：

```text
                    Email Client
                         │
                         │
                         ▼
                Microsoft OAuth
                         │
                         ▼
               Microsoft Identity
                         │
                  Authorization
                         │
                         ▼
                  Authorization Code
                         │
                         ▼
                  Token Endpoint
                         │
                  ┌──────┴──────┐
                  ▼             ▼
             Access Token   Refresh Token
```

桌面应用推荐：

```text
Authorization Code
+
PKCE
```

Token 不应该进入普通数据库明文。

推荐：

```text
macOS
  ↓
Keychain

Windows
  ↓
Credential Manager / DPAPI

Linux
  ↓
Secret Service / Keyring
```

---

# 9. Outlook 两种访问方式

Microsoft Provider 内部实际上可以支持两种 Transport：

```text
MicrosoftProvider
       │
       ├── GraphTransport
       │
       └── IMAPTransport
```

第一阶段：

```text
GraphTransport
```

第二阶段：

```text
IMAPTransport
```

为什么保留 IMAP？

因为某些 Graph 没有完全覆盖的邮件协议场景，可以通过 IMAP 作为 fallback。

Microsoft 官方同时支持通过 OAuth 访问 IMAP / POP / SMTP，并要求通过 XOAUTH2 传递 access token。

---

# 10. Outlook IMAP OAuth

如果走 IMAP：

```text
OAuth Access Token
       │
       ▼
XOAUTH2
       │
       ▼
IMAP
```

认证：

```text
AUTHENTICATE XOAUTH2 <base64>
```

Microsoft 官方定义的 XOAUTH2 payload 本质是：

```text
user=<email>
auth=Bearer <access_token>
```

再进行 Base64 编码。

因此：

```text
Password Authentication
```

与：

```text
OAuth Authentication
```

不要在 Domain 层混为一谈。

---

# 11. Outlook Graph Transport

推荐第一版直接实现：

```ts
class MicrosoftGraphTransport {

  async listFolders() {}

  async listMessages() {}

  async getMessage() {}

  async sendMessage() {}

  async createDraft() {}

  async reply() {}

  async forward() {}

  async deleteMessage() {}

  async moveMessage() {}

  async markRead() {}

  async setFlag() {}

  async deltaSync() {}

}
```

Graph API 可以完成邮件读取、发送、回复、转发、更新、删除以及文件夹和增量同步等操作。

---

# 12. Gmail Provider

Gmail 同样不要强制走 Generic IMAP。

```text
GmailProvider
      │
      ├── GmailOAuth
      │
      ├── GmailAPITransport
      │
      └── optional IMAPTransport
```

因此整体结构：

```text
Microsoft
    └── Graph

Gmail
    └── Gmail API

Others
    └── IMAP
```

这三个 Provider 是第一优先级。

---

# 13. Generic IMAP Provider

用于：

```text
163
126
QQ
阿里云
腾讯企业邮箱
Zoho
自建邮箱
```

结构：

```text
GenericIMAPProvider
        │
        ├── IMAP
        │
        ├── SMTP
        │
        └── Authentication
               ├── Password
               ├── AppPassword
               └── OAuth2
```

配置：

```ts
interface IMAPConfig {
  host: string
  port: number
  tls: boolean
  auth: AuthConfig
}

interface SMTPConfig {
  host: string
  port: number
  tls: boolean
  auth: AuthConfig
}
```

---

# 14. Domain Model

千万不要把 Graph Message 或 IMAP Message 直接暴露给 UI。

应该建立自己的统一 Message Model：

```ts
interface Message {
  id: MessageId

  accountId: AccountId

  folderId: FolderId

  threadId?: string

  messageId?: string

  inReplyTo?: string

  references?: string[]

  from: Address[]

  to: Address[]

  cc: Address[]

  bcc: Address[]

  replyTo?: Address[]

  subject: string

  snippet?: string

  textBody?: string

  htmlBody?: string

  attachments: Attachment[]

  receivedAt?: Date

  sentAt?: Date

  isRead: boolean

  isStarred: boolean

  isDraft: boolean

  isDeleted: boolean

  labels: string[]

  providerData?: unknown
}
```

---

# 15. Provider ID 与本地 ID 必须分离

这是非常重要的数据库设计。

不要：

```text
message.id = Graph message.id
```

应该：

```text
message.id
    ↓
本地 UUID

message.remoteId
    ↓
Provider ID
```

例如：

```text
Message
────────────────────────
id          = local UUID
accountId   = account UUID
remoteId    = AAMkAG...
provider    = microsoft
folderId    = local folder UUID
```

原因是不同 Provider 的 ID 规则完全不同。

而且 Microsoft Graph 官方特别提醒，普通 message/folder ID 在某些操作（例如 move/copy）后可能发生变化；如果需要稳定引用，应使用 immutable ID。

因此：

```text
local ID
    +
provider
    +
remote ID
    +
immutable ID (if available)
```

是更稳妥的设计。

---

# 16. Folder Model

```ts
interface Folder {
  id: string

  accountId: string

  remoteId: string

  parentId?: string

  name: string

  type:
    | "inbox"
    | "sent"
    | "drafts"
    | "trash"
    | "spam"
    | "archive"
    | "custom"

  unreadCount?: number

  totalCount?: number
}
```

不要直接使用：

```text
Inbox
Sent
Trash
```

作为数据库 ID。

因为不同邮箱可能：

```text
INBOX
Inbox
收件箱
AAMk...
```

所以应该统一映射：

```text
remote folder
       ↓
FolderType
```

---

# 17. Thread 不应该强依赖 Provider

定义：

```ts
interface Thread {
  id: string

  accountId: string

  subject: string

  messageIds: string[]

  lastMessageAt: Date

  unreadCount: number
}
```

Provider 可以提供：

```text
conversationId
```

例如 Outlook Graph 有 `conversationId` 等 Outlook 特性。

但是本地仍然需要：

```text
Thread
```

作为统一模型。

否则 Gmail / IMAP 没有同样的概念时会很难处理。

实现补充（Phase 1 已落地）：本地归组按五级信号依次回退——

1. RFC 5322 祖先链：`In-Reply-To` → `References`（倒序前 10 条）查已知 `Message-ID`；
2. 服务端线程 key：Gmail 经 IMAP 扩展取 `X-GM-THRID`，或 RFC 5256 `THREAD=REFERENCES` 分组（key 取组内最小 UID），存储为 `messages.server_thread_key`；
3. Outlook `Thread-Index` 头前 16 字节 GUID（`messages.thread_index_root`）；
4. 归一化主题（剥 `Re:`/`Fwd:`/`答复:`/`转发:` 前缀与 `[list-tag]`）；
5. 新建线程。

`X-GM-THRID` 与 `THREAD` 响应无法经过 imap crate 的解析器（封闭枚举，遇到未知行会中止读取并毒化会话），因此在一条专用原始连接上手工解析（见 `crates/eeeemail-provider/src/imap/extensions.rs`），失败仅降级为本地归组。回复发件时会带 `In-Reply-To`/`References` 头，保证我们的回复也能正确归入会话。

---

# 18. Sync Architecture

同步是整个 Email Client 最复杂的部分。

不要：

```text
每次打开 App
    ↓
重新拉全部邮件
```

应该：

```text
Initial Sync
     ↓
Local Database
     ↓
Incremental Sync
     ↓
Local Database
```

统一接口：

```ts
interface SyncEngine {
  initialSync(
    accountId: string
  ): Promise<void>

  incrementalSync(
    accountId: string
  ): Promise<SyncResult>
}
```

---

# 19. 不同 Provider 的 Sync Strategy

### Microsoft

```text
Graph
  ↓
delta query
  ↓
cursor / deltaLink
```

Microsoft Graph 原生支持邮件和文件夹的增量变化同步。

### Gmail

```text
Gmail History API
```

### IMAP

```text
UID
UIDVALIDITY
MODSEQ
IDLE
```

因此统一：

```ts
interface SyncCursor {
  provider: string

  accountId: string

  folderId: string

  cursorType: string

  cursor: string
}
```

不要假设所有 Provider 都是：

```text
nextPageToken
```

---

# 20. Sync Pipeline

推荐：

```text
Provider
    │
    ▼
Remote Changes
    │
    ▼
Normalizer
    │
    ▼
Domain Events
    │
    ▼
Local Database
    │
    ▼
UI
```

例如 Outlook：

```text
Graph delta
     ↓
GraphMessage
     ↓
MicrosoftNormalizer
     ↓
MessageUpserted
     ↓
SQLite
     ↓
UI update
```

---

# 21. 本地数据库

如果是 Desktop Client：

> **SQLite 非常适合。**

推荐：

```text
accounts
folders
messages
threads
attachments
contacts
sync_states
labels
message_labels
drafts
```

核心关系：

```text
accounts
    │
    ├── folders
    │      │
    │      └── messages
    │
    ├── threads
    │
    └── sync_states
```

---

# 22. Account Schema

```sql
accounts (
  id              TEXT PRIMARY KEY,
  provider        TEXT NOT NULL,
  email           TEXT NOT NULL,
  display_name    TEXT,

  auth_type       TEXT NOT NULL,

  created_at      INTEGER NOT NULL,
  updated_at      INTEGER NOT NULL
)
```

Token 不放这里。

应该：

```text
accounts
    │
    └── credential reference
             │
             ▼
          Keychain
```

---

# 23. Credential Model

```ts
interface AuthSession {
  provider: string

  accountId: string

  accessToken?: string

  refreshToken?: string

  expiresAt?: Date

  scopes: string[]
}
```

但是：

> **AuthSession 只是运行时对象。**

不要直接 serialize 到普通 SQLite。

---

# 24. Capability System

这是多 Provider 架构非常关键的一层。

```ts
interface ProviderCapabilities {

  // Basic
  read: boolean
  send: boolean
  drafts: boolean
  attachments: boolean

  // Organization
  folders: boolean
  labels: boolean
  categories: boolean

  // Message
  reply: boolean
  forward: boolean
  threading: boolean

  // Sync
  deltaSync: boolean
  push: boolean

  // Search
  serverSearch: boolean

  // Advanced
  sharedMailbox: boolean
  calendar: boolean
  contacts: boolean
}
```

例如：

```text
Microsoft
    read          ✓
    send          ✓
    folders       ✓
    categories    ✓
    deltaSync     ✓
    push          ✓
    sharedMailbox ✓

Generic IMAP
    read          ✓
    send          ✓
    folders       ✓
    categories    -
    deltaSync     maybe
    push          IDLE
    sharedMailbox -
```

---

# 25. Feature Detection

UI 不应该：

```ts
if (provider === "microsoft") {
  ...
}
```

这是一个坏味道。

应该：

```ts
if (capabilities.categories) {
  showCategories()
}
```

而不是：

```ts
if (provider === "microsoft") {
  showCategories()
}
```

这样以后 Gmail 也支持类似能力时，不需要修改 UI。

---

# 26. Provider Extension

有些厂商能力无法抽象成统一 API。

例如 Outlook：

```text
Categories
Focused Inbox
Conversation
MAPI properties
Shared mailbox
```

不要为了统一而强行塞进：

```ts
Message
```

可以：

```ts
interface ProviderExtension {
  provider: string

  getFeature(name: string): unknown
}
```

例如：

```ts
microsoft.getFeature("categories")
```

或者：

```ts
message.providerData
```

这样：

```text
Common Domain
        │
        ├── Universal features
        │
        └── Provider Extensions
```

---

# 27. 搜索架构

搜索也不要完全依赖 Provider。

推荐：

```text
              Search
                │
       ┌────────┴────────┐
       ▼                 ▼
 Local Search       Remote Search
       │                 │
 SQLite FTS          Provider API
```

例如：

```text
搜索最近邮件
    ↓
SQLite FTS

搜索整个 Gmail
    ↓
Gmail API

搜索 Outlook mailbox
    ↓
Graph
```

最终统一结果：

```ts
interface SearchResult {
  messageId: string
  score?: number
  source: "local" | "remote"
}
```

---

# 28. Offline First

Email Client 非常适合 Offline First。

原则：

```text
UI
 ↓
Local DB
 ↓
Provider
```

而不是：

```text
UI
 ↓
Network
 ↓
Provider
```

打开 Inbox：

```text
Local DB
   ↓
立即显示
   ↓
后台 Sync
   ↓
更新 UI
```

这样即使：

```text
飞机
地铁
断网
Outlook API timeout
```

用户仍然可以看到历史邮件。

---

# 29. Send Architecture

发送邮件应该进入 Outbox：

```text
Compose
   ↓
Draft
   ↓
Outbox
   ↓
Provider
   ↓
Sent
```

不要：

```text
点击 Send
   ↓
直接调用 SMTP/API
```

否则网络失败时 UX 很差。

定义：

```ts
interface OutboxJob {
  id: string

  accountId: string

  messageId: string

  status:
    | "pending"
    | "sending"
    | "sent"
    | "failed"

  retryCount: number

  lastError?: string
}
```

---

# 30. Retry

Provider API 可能出现：

```text
401
403
429
500
502
503
timeout
network error
```

统一：

```text
401
 ↓
refresh token
 ↓
retry

429
 ↓
Retry-After
 ↓
backoff

5xx
 ↓
exponential backoff

403
 ↓
permission error
 ↓
don't retry forever
```

---

# 31. Token Refresh

这是 Provider 必须自己负责的事情。

```text
API Request
    │
    ▼
Access Token
    │
    ├── valid ───────→ request
    │
    └── expired
           │
           ▼
      Refresh Token
           │
           ▼
      New Access Token
           │
           ▼
         retry
```

Application Layer 不应该知道：

```text
Microsoft refresh endpoint
Google refresh endpoint
```

这些属于 Auth Provider。

---

# 32. 错误模型

不要直接把 HTTP error 暴露给 UI。

统一：

```ts
enum MailErrorCode {
  AuthenticationRequired,
  AuthenticationFailed,

  PermissionDenied,

  NetworkUnavailable,
  Timeout,

  RateLimited,

  MessageNotFound,
  FolderNotFound,

  SendFailed,

  AttachmentTooLarge,

  ProviderUnavailable,

  UnsupportedFeature,

  Unknown
}
```

例如：

```text
Microsoft Graph 401
       ↓
AuthenticationRequired
```

而不是 UI 判断：

```ts
if (error.status === 401)
```

---

# 33. 推荐项目目录

如果使用 TypeScript：

```text
src/
│
├── domain/
│   ├── account/
│   ├── message/
│   ├── folder/
│   ├── thread/
│   └── attachment/
│
├── application/
│   ├── mail/
│   ├── sync/
│   ├── search/
│   ├── compose/
│   └── account/
│
├── providers/
│   │
│   ├── microsoft/
│   │   ├── auth/
│   │   ├── graph/
│   │   ├── imap/
│   │   ├── mapper/
│   │   └── provider.ts
│   │
│   ├── google/
│   │   ├── auth/
│   │   ├── gmail/
│   │   ├── mapper/
│   │   └── provider.ts
│   │
│   └── generic-imap/
│       ├── imap/
│       ├── smtp/
│       ├── auth/
│       └── provider.ts
│
├── infrastructure/
│   ├── database/
│   ├── keychain/
│   ├── network/
│   ├── logging/
│   └── notification/
│
└── ui/
```

---

# 34. Provider 与 Domain 的边界

这是整个系统最重要的依赖关系：

```text
                 UI
                  │
                  ▼
             Application
                  │
                  ▼
               Domain
                  │
                  ▼
            MailProvider
                  ▲
                  │
        ┌─────────┼──────────┐
        │         │          │
        ▼         ▼          ▼
    Microsoft   Gmail      IMAP
```

依赖方向必须是：

```text
Provider → Domain
```

而不能：

```text
Domain → Microsoft Graph
```

例如 Domain 里面绝对不要出现：

```ts
GraphMessage
MicrosoftMessage
IMAPMessage
```

应该只有：

```ts
Message
Folder
Thread
Attachment
```

---

# 35. 推荐的 Provider 层次

最终：

```text
MailProvider
│
├── ApiMailProvider
│   ├── MicrosoftGraphProvider
│   └── GmailProvider
│
└── ProtocolMailProvider
    └── GenericIMAPProvider
```

进一步：

```text
AuthProvider
│
├── OAuthProvider
│   ├── MicrosoftOAuth
│   └── GoogleOAuth
│
├── PasswordAuth
└── AppPasswordAuth
```

这样认证与数据访问彻底解耦。

---

# 36. 第一阶段实现范围

不要一开始就实现所有能力。

### Phase 1

```text
Account
OAuth
IMAP
SMTP
Message
Folder
Attachment
Send
Read
Delete
Move
Mark Read
Local SQLite
```

支持：

```text
Generic IMAP
Microsoft Graph
Gmail API
```

---

### Phase 2

```text
Incremental Sync
Offline
Draft
Reply
Forward
Search
Thread
Outbox
Retry
```

---

### Phase 3

```text
Push
Webhook
Categories
Labels
Advanced Search
Shared Mailbox
Calendar
Contacts
```

---

# 37. Provider 优先级

我建议：

```text
                   Provider
                      │
       ┌──────────────┼──────────────┐
       ▼              ▼              ▼
   Microsoft        Google        Generic
       │              │              │
       ▼              ▼              ▼
     Graph          Gmail API       IMAP
       │              │              │
    OAuth2          OAuth2       OAuth/Password
```

第一批：

> **Microsoft + Gmail + Generic IMAP**

已经可以覆盖绝大多数场景。

然后：

```text
Yahoo
Zoho
QQ
163
阿里云
腾讯企业邮箱
```

优先通过 Generic IMAP 覆盖。

---

# 38. 一个关键架构原则

不要把：

> **Email Provider**

理解成：

> **Mail Protocol**

这是这个项目最容易犯的架构错误。

实际上应该是：

```text
Provider
   │
   ├── Identity
   │
   ├── Authentication
   │
   ├── Transport
   │
   ├── Sync
   │
   ├── Mapping
   │
   └── Capabilities
```

例如：

```text
Microsoft
   ├── Microsoft OAuth
   ├── Graph
   ├── IMAP XOAUTH2
   ├── Delta Sync
   ├── Webhook
   └── Outlook features
```

而：

```text
Generic IMAP
   ├── Password
   ├── OAuth
   ├── IMAP
   ├── SMTP
   ├── IDLE
   └── UID/UIDVALIDITY
```

这样才真正能处理“Outlook 这种特殊认证厂商”。

---

# 39. 最终架构

最终推荐形成：

```text
┌─────────────────────────────────────────────────┐
│                     UI                          │
└───────────────────────┬─────────────────────────┘
                        │
┌───────────────────────▼─────────────────────────┐
│                Application Layer                │
│                                                 │
│ MailService / Sync / Search / Compose / Outbox │
└───────────────────────┬─────────────────────────┘
                        │
┌───────────────────────▼─────────────────────────┐
│                   Domain                        │
│                                                 │
│ Account / Folder / Message / Thread / Attachment│
└───────────────────────┬─────────────────────────┘
                        │
                Provider Interface
                        │
        ┌───────────────┼────────────────┐
        │               │                │
        ▼               ▼                ▼
   Microsoft          Gmail         Generic IMAP
        │               │                │
        │               │                │
     OAuth2          OAuth2        OAuth / Password
        │               │                │
     Graph API       Gmail API       IMAP / SMTP
        │               │                │
        └───────────────┼────────────────┘
                        │
                        ▼
                  Provider Mapper
                        │
                        ▼
                  Domain Model
                        │
                        ▼
                    SQLite
```

## 最核心的三个决策

### 决策 1：不要 IMAP-first

不要：

```text
所有邮箱
   ↓
IMAP
```

而应该：

```text
Microsoft → Graph
Gmail     → Gmail API
Others    → IMAP
```

Microsoft 自己也推荐通过 Microsoft Graph 访问 Outlook Mail；Graph 提供邮件、文件夹、发送、回复、附件、增量同步和通知等能力。

### 决策 2：Authentication 与 Mail Transport 分离

```text
OAuth
Password
App Password
```

与：

```text
Graph
Gmail API
IMAP
SMTP
```

是两个不同维度。

### 决策 3：Provider Capability 驱动 UI

不要：

```ts
if (provider === "outlook")
```

而应该：

```ts
if (capabilities.deltaSync)
if (capabilities.categories)
if (capabilities.push)
if (capabilities.sharedMailbox)
```

这样未来增加邮箱厂商不会不断修改核心业务代码。

---

# 40. 推荐的最终抽象

如果让我从零开始实现，我会最终收敛成下面这几个核心接口：

```ts
interface MailProvider {
  readonly id: string

  readonly capabilities: ProviderCapabilities

  readonly auth: AuthProvider

  readonly transport: MailTransport

  readonly mapper: MailMapper

  readonly sync: SyncProvider
}
```

```ts
interface MailTransport {
  listFolders(): Promise<RemoteFolder[]>

  listMessages(
    folder: RemoteFolder,
    options?: ListOptions
  ): Promise<RemoteMessage[]>

  getMessage(
    id: string
  ): Promise<RemoteMessage>

  sendMessage(
    message: RemoteMessage
  ): Promise<void>
}
```

```ts
interface SyncProvider {
  initialSync(): Promise<SyncResult>

  incrementalSync(
    cursor?: SyncCursor
  ): Promise<SyncResult>

  subscribe?(): Promise<Subscription>
}
```

```ts
interface MailMapper {
  mapMessage(
    remote: unknown
  ): Message

  mapFolder(
    remote: unknown
  ): Folder
}
```

最终形成：

```text
                    MailProvider
                         │
       ┌─────────────────┼─────────────────┐
       │                 │                 │
      Auth            Transport           Sync
       │                 │                 │
       ▼                 ▼                 ▼
   Microsoft          Graph             Delta
      OAuth
```

这套结构最大的价值是：

> **Outlook 可以很特殊，但它的特殊性被关在 `MicrosoftProvider` 里面，而不会污染你的 Email Client 核心架构。**

这样以后再加入一个“非常奇怪”的邮箱厂商，也只需要增加：

```text
providers/
    weird-mail/
        auth/
        transport/
        mapper/
        sync/
        provider.ts
```

而不是修改整个系统。