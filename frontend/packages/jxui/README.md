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
MailboxGroup 接受 MailboxCore 和文件夹渲染函数，显示邮箱名称、同步状态及可独立折叠的文件夹。
MailboxCore 保留展开状态，状态更新不会重建文件夹视图。
Panel 的 kind 为 sidebar / list / reader。Toolbar、Badge、Avatar 和 EmptyState
为无状态组件。MailMessage 使用页面的 MessageCore，控制展开、已读同步和
正文选项，并在卸载时清理 ResizeObserver、动画帧和事件监听。

所有控件为原生 HTML 元素，键盘焦点可见；SVG 图标使用统一 viewBox、
currentColor、线宽和圆角端点。正文仅出现在 sandbox iframe，不进入可信视图树。

Glyph 使用 Timeless.Icon 和官方 @timeless/inner-icons ASN 定义，所需图标子集随应用离线打包。
NotificationCenter 接受 NotificationCore：错误历史最多保留 100 项，重复错误合并计数；
支持单条已读／未读、全部已读，以及未读数量。打开面板不会自动标记已读。
传入 storage 时保存到当前浏览器来源的存储；未提供或存储不可用时仍可记录本次页面会话。
