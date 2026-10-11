# jxui

轻量 Timeless 邮件 UI 组件，颜色和尺寸来自 `../design` / `../../.design`。

```js
import { Button, Input, NavItem } from './index.js';
import { ControlCore } from './core.js';

const refresh = ControlCore({ label: '刷新邮件', disabled: false }, {
  click() { inbox.methods.loadList(); },
});
Button({ store: refresh, variant: 'ghost', icon: 'refresh' });
```

状态组件接受 `store`，不直接持有或更改业务状态。`ControlCore` 提供
`state`、`set`、`click`、`input`、`keydown`、`onStateChange` 和 `destroy`。
视图卸载时取消订阅并清理派生 ref；Core 的拥有者负责销毁 Core。

Button 提供 primary / secondary / ghost；图标按钮必须给 Core 提供 label。
Input 提供 label / placeholder。Select 接受 options ref（`{id,email}[]`）。
NavItem 接受 icon / count。ConversationRow 接受线程摘要和导航 Core。
收藏列表的 ConversationRow 接受 favorite=true 和页面持有的 Core，更多菜单提供取消整段对话收藏；菜单按钮与导航链接互为兄弟节点。
MailboxGroup 接受 MailboxCore、服务 root 和文件夹渲染函数，显示邮箱名称、平台 SVG 图标及可独立折叠的文件夹。
MailboxCore 按 platform → provider 选择图标，正常状态不显示文字，异常角标点击后显示脱敏提示；
提示开关由 Core 维护。状态及配置更新保留文件夹展开状态，不重建文件夹视图。
Panel 的 kind 为 sidebar / list / reader。Toolbar、Badge、Avatar 和 EmptyState
为无状态组件。MailMessage 使用页面的 MessageCore，控制展开、已读同步和
正文选项，并在卸载时清理 ResizeObserver、动画帧和事件监听。
ReplyWindow 使用页面持有的 ReplyCore：可拖拽和调整大小的独立回复窗口，包含 To/Cc 收件人标签、
本地联系人搜索选择、正文与附件。窗口位置、收件人、附件读取及发送状态均由 Core 维护，切换邮件不会丢失回复。
标题栏支持方向键移动，收件人下拉支持方向键和 Enter，附件可选择文件或拖入窗口。

所有控件为原生 HTML 元素，键盘焦点可见；SVG 图标使用统一 viewBox、
currentColor、线宽和圆角端点。正文仅出现在 sandbox iframe，不进入可信视图树。

Glyph 使用 Timeless.Icon 和官方 @timeless/inner-icons ASN 定义，所需图标子集随应用离线打包。
NotificationCenter 接受 NotificationCore：错误历史最多保留 100 项，重复错误合并计数；
支持单条已读／未读、全部已读，以及未读数量。打开面板不会自动标记已读。
传入 storage 时保存到当前浏览器来源的存储；未提供或存储不可用时仍可记录本次页面会话。
