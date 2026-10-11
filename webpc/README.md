# 简信 Web / design + jxui

前端以 Cloudflare [Agentic Inbox](https://github.com/cloudflare/agentic-inbox) 的
`demo_app.png`、Sidebar、Header、MailboxSplitView 和 email-list 为视觉参考。
参考 revision：`48039bb6785af34e592c2966f87cde2b255c4c80`。
采用浅灰侧栏、白色分栏、细边界、蓝色未读标记和约 380px 的阅读旁列表。

## 分层

- `.design`：唯一视觉契约，包含语义颜色、字体、间距、尺寸、组件和来源。
- `packages/design`：导出规范中的 tokens / components；构建器将同一份 tokens
  生成 CSS 变量。调整颜色或尺寸应先修改 `.design`。
- `packages/jxui`：可复用 Timeless 组件及 ControlCore。包含 Button、Input、Select、
  NavItem、Badge、Avatar、ConversationRow、Panel、Toolbar、EmptyState、MailMessage、AgentPanel 与 AgentCore。
- `src/pages/inbox/index.model.js`：多邮箱分组、状态刷新、筛选、分页、请求取消、正文选项、已读同步。
- `src/pages/inbox/index.js`：页面组装，只调用 Model / Core 方法。
- `src/services.js`：同源的本机邮件 / Agent API，写操作校验请求标记和 Origin。
- `src/gallery.js`：组件示例页；不读取或修改真实邮件。
- `vendor`：从本地 timeless 0.34.0 复制的 Lite / DOM 发布产物，保留 MIT 许可和 SHA256。
- `dist`：本地手动构建产物，已被 Git 忽略。Cargo 构建会在自己的 `OUT_DIR` 生成并嵌入同一套资源，无运行时 CDN。

## 构建与检查

```sh
npm ci --prefix webpc
node webpc/build.mjs
node webpc/build.mjs --check
node --test webpc/tests/*.test.mjs
cargo build -p jianxin-cli --bin jianxin
python3 webpc/tests/reply-http.py target/debug/jianxin
# 本机模拟 OAuth 服务验证授权、凭据保存和配置刷新，无真实登录：
python3 webpc/tests/http-smoke.py target/debug/jianxin
# 构建后，以下检查使用本机模型和空邮箱，不打开浏览器：
python3 webpc/tests/agent-http.py target/macos-app/jianxin
TMPDIR=/private/tmp cargo test -p jianxin-cli -p jianxin-store -p jianxin-core
bash scripts/build-macos-app.sh --install
```

构建器使用本目录的 esbuild 0.27.4，依赖由 `package-lock.json` 锁定。需要 Node.js 18+，首次构建前运行 `npm ci --prefix webpc`。
CLI 的 Cargo build script 自动构建并嵌入 Web 资源，监听源码和设计契约变化；无需先生成 `dist`，也无需相邻的 timeless 项目。macOS、Windows、Linux 打包均使用这条构建链路。

更新 timeless 时，应从对应版本的本地 `dist/timeless/<version>` 更新两份 runtime，
同时更新 vendor manifest 和许可证，再重新执行全部非浏览器检查。

## 手动验证

```sh
target/macos/Jianxin.app/Contents/Resources/jianxin web --port 0
```

打开命令输出的网址 `http://127.0.0.1:PORT/`。组件展示位于 `/components`，
例如 `http://127.0.0.1:PORT/components`。

检查邮箱和文件夹切换、主题/发件人/摘要搜索、50 条对话分页、选择对话、
返回和浏览器前进后退。宽屏打开右侧阅读面板，窄屏使用单栏和返回按钮。
每次最多加载 50 封邮件头，可继续加载后续邮件；只有展开的邮件加载正文。
检查图片和纯文本开关、正文高度，以及已读同步失败后的重试。
检查白色主题区和文件夹标签、发件人邮箱、顶部及底部回复入口；「更多」菜单提供已读状态、发件人详情和「邮件元数据」，支持外部点击或 Escape 收起。
检查元数据弹窗的 ID、对话 ID 和 Message-ID，以及发件人详情中的完整收件人和抄送地址；关闭或按 Escape 后焦点返回按钮。
检查阅读工具栏的展开阅读、恢复分栏、上一封/下一封，以及列表边界和消息直达链接时的禁用状态。
检查收藏卡片右侧「更多 → 取消收藏」：不打开邮件，取消对话内所有收藏，成功后移除卡片；失败保留剩余收藏并提示重试，支持 Escape 和外部点击关闭菜单。
检查每封邮件的「回复」：悬浮窗口拖动和调整大小，To/Cc 搜索、选择和编辑，附件选择及拖入、移除；
切换邮件、最小化和放大时保留回复，窗口标题显示自动生成的 Re: 主题，点击修改主题展开编辑；底部「发送」才提交邮件。
关闭前确认丢弃，发送中禁止重复点击，失败时保留内容和附件。
手动标为未读后，不会在当前页面自动改回已读。

页面读本地数据，刷新不强制发起邮箱同步。已读写操作立即更新页面状态及未读数量，
后台同步远端；失败时恢复状态和数量并提示重试。邮件助手支持按邮箱独立聊天、搜索与总结、工具执行记录、可编辑回复草稿和已读变更提议。
草稿保存及状态变更必须明确确认；不提供自动发送、删除或归档。模型配置及隐私说明见 [Web Agent](../docs/web.md#邮件-agent)。
不运行或自动操作浏览器；这些交互需要人工验证。
