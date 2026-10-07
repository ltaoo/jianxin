import { tokens, design } from '../packages/design/index.js';
import { Button, Input, NavItem, Avatar, Badge, Toolbar, EmptyState, ConversationRow, MailboxGroup, Glyph, NotificationCenter } from '../packages/jxui/index.js';
import { ControlCore, MailboxCore, NotificationCore } from '../packages/jxui/core.js';

/** Standalone component examples; never inject fixtures into the actual inbox. */
export function GalleryPageView({ root }) {
  const cores = [];
  const control = (state, actions) => { const core = ControlCore(state, actions); cores.push(core); return core; };
  const notice = ref('组件示例，不会操作真实邮件');
  const clickable = label => control({ label }, { click() { notice.as(`已点击：${label}`); } });
  const thread = { id: 'example', sender: '组件示例', subject: '一条对话，多个清晰的状态', snippet: '用于检查列表密度、对齐和选中状态。', account_email: 'example@example.com', message_count: 2, unread_count: 1, last_message_at: 0 };
  const mailbox = MailboxCore({ id: 'example', email: 'example@example.com', status: { label: '已同步', tone: 'success' }, folders: [{ id: 'example-inbox', name: 'Inbox' }] });
  cores.push(mailbox);
  const notifications = NotificationCore();
  notifications.record('example', '示例：邮箱状态更新失败', '组件示例消息，不代表真实邮箱发生错误。');
  cores.push(notifications);
  function section(title, children) { return View({ as: 'section', class: 'jx-gallery-section' }, [View({ as: 'h2' }, [title]), ...children]); }
  return View({ as: 'main', class: 'jx-gallery', onUnmounted() { cores.forEach(c => c.destroy()); notice.destroy(); } }, [
    View({ as: 'a', attributes: { href: `${root}/` } }, ['← 返回邮件']),
    View({ as: 'h1' }, [`design + jxui · ${design.version}`]),
    View({ as: 'p', class: 'jx-muted' }, ['基于 Agentic Inbox 的浅色邮件工作区。组件使用 Timeless Core 驱动。']),
    section('语义颜色', [View({ class: 'jx-swatches' }, Object.entries(tokens.color).map(([name, color]) =>
      View({ class: 'jx-swatch' }, [View({ class: 'jx-swatch-color', style: { background: color } }), View({ as: 'code' }, [name])])))]),
    section('按钮与输入', [View({ class: 'jx-row' }, [
      Button({ store: clickable('主要操作'), variant: 'primary' }), Button({ store: clickable('次要操作') }),
      Button({ store: clickable('刷新'), variant: 'ghost', icon: 'refresh' }),
      Button({ store: control({ label: '不可用', disabled: true }) }),
    ]), Input({ store: control({ value: '' }), label: '输入组件示例', placeholder: '在这里输入搜索内容…' }),
      View({ attributes: { role: 'status', 'aria-live': 'polite' }, class: 'jx-muted' }, [notice])]),
    section('导航、徽章与头像', [NavItem({ store: control({ label: '收件箱', active: true }), icon: 'inbox', count: 12 }),
      NavItem({ store: control({ label: '已发送' }), icon: 'sent' }), View({ class: 'jx-row' }, [Avatar({ name: '简信' }), Badge({}, ['2 封邮件']), Badge({}, ['未读'])])]),
    section('邮箱分组', [MailboxGroup({ store: mailbox }, folder => NavItem({ store: clickable(folder.name), icon: 'inbox' }))]),
    section('Timeless 图标', [View({ class: 'jx-row' }, ['inbox', 'sent', 'drafts', 'archive', 'trash', 'spam', 'refresh', 'search', 'notification', 'attachment'].map(name => Glyph(name)))]),
    section('通知中心', [NotificationCenter({ store: notifications })]),
    section('对话行', [ConversationRow({ store: control({ href: '#examples', active: true }), thread })]),
    section('工具栏与空状态', [Toolbar({}, [View({ as: 'strong' }, ['全部邮件']), Button({ store: clickable('刷新列表'), variant: 'ghost', icon: 'refresh' })]),
      EmptyState({ title: '这里还没有邮件', description: '后台同步完成后，点击刷新查看本地邮件。' })]),
  ]);
}
