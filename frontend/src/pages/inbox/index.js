import { AgentCore } from '../../../packages/jxui/agent-core.js';
import { AgentPanel } from '../../../packages/jxui/agent-panel.js';
import { AgentService } from '../../services.js';
import { Button, Input, MailboxGroup, NavItem, Panel, Toolbar, EmptyState, ConversationRow, Badge, Glyph, NotificationCenter } from '../../../packages/jxui/index.js';
import { ControlCore } from '../../../packages/jxui/core.js';
import { MailMessage } from '../../../packages/jxui/mail-message.js';
import { InboxPageModel } from './index.model.js';

export function InboxPageView(props) {
  let storage;
  try { storage = typeof window !== 'undefined' ? window.localStorage : undefined; } catch {}
  const vm = InboxPageModel({ ...props, storage });
  const { state: s, ui, methods } = vm;
  const agent = AgentCore({ service: AgentService(props.root), context: () => s.selected.value,
    refresh: methods.refresh, notify: message => ui.notifications.record('agent-request', '邮件助手请求失败', message) });
  const offAgentScope = s.account.subscribe({ onChange: value => agent.scope(value) });
  agent.scope(s.account.value);
  const agentToggle = ControlCore({ label: '邮件助手' }, { click: () => agent.toggle() });
  let cleanupSearch = () => {};
  let statusTimer;
  const refreshStatuses = () => { if (!document.hidden) methods.refreshStatuses(); };
  const derived = [];
  const computed = (deps, fn) => { const value = Timeless.computed(deps, fn); derived.push(value); return value; };
  const combine = (deps, fn) => { const value = Timeless.combine(deps, fn); derived.push(value); return value; };
  const folderLabels = { inbox: '收件箱', sent: '已发送', drafts: '草稿箱', archive: '归档', trash: '垃圾箱', spam: '垃圾邮件' };
  const heading = combine({ folders: s.folders, folder: s.folder, unread: s.unread, query: s.query }, d =>
    d.query.trim() ? '搜索结果' : d.unread ? '未读邮件' : d.folders.find(f => f.id === d.folder)?.name || '全部邮件');
  const selected = computed(s.selected, value => !!value);
  function nav(store, field, predicate, icon) {
    const off = field.subscribe({ onChange(value) { store.set({ active: predicate(value) }); } });
    const offAccount = s.account.subscribe({ onChange() { store.set({ active: predicate(field.value) }); } });
    const offUnread = s.unread.subscribe({ onChange() { store.set({ active: predicate(field.value) }); } });
    store.set({ active: predicate(field.value) });
    return View({ onUnmounted() { off(); offUnread(); offAccount(); } }, [NavItem({ store, icon, showCount: true })]);
  }
  function Sidebar() {
    return Panel({ kind: 'sidebar', as: 'aside', attributes: { 'aria-label': '邮箱导航' } }, [
      View({ class: 'jx-brand' }, [View({ class: 'jx-brand-mark' }, [Glyph('mail')]), View({ class: 'jx-brand-copy' }, [View({ as: 'strong' }, ['简信']), View({ as: 'span' }, ['JIANXIN · 邮件工作区'])])]),
      View({ as: 'nav', class: 'jx-navigation' }, [
        nav(ui.all, s.folder, value => !s.account.value && !value && !s.unread.value, 'all'),
        nav(ui.unread, s.unread, value => !s.account.value && value, 'unread'),
        View({ class: 'jx-section-label jx-row' }, ['我的邮箱', Badge({}, [computed(s.accounts, items => String(items.length))])]),
        For({ each: s.accountGroups, render(group) {
          return MailboxGroup({ store: group }, (folder, accountId) => {
            const active = () => s.account.value === accountId && s.folder.value === folder.id;
            const core = ControlCore({ label: folderLabels[folder.kind] || folder.name, active: active() }, { click() { methods.selectFolder(accountId, folder.id); } });
            const offs = [s.folder, s.account].map(field => field.subscribe({ onChange() { core.set({ active: active() }); } }));
            return View({ onUnmounted() { offs.forEach(off => off()); core.destroy(); } }, [NavItem({ store: core, icon: folder.kind, count: folder.unread_count })]);
          });
        } }),
      ]),
      View({ class: 'jx-sidebar-footer' }, [View({ class: 'jx-row' }, [Glyph('server'), '本地邮件阅读器']),
        View({ as: 'p', class: 'jx-muted' }, ['后台自动收信，刷新查看新邮件。'])]),
    ]);
  }
  function List() {
    return Panel({ kind: 'list', id: computed(selected, v => v ? 'conversation-list' : 'mail-content'), attributes: { 'aria-label': '邮件对话列表' } }, [
      Toolbar({}, [View({ class: 'jx-list-title jx-row' }, [Glyph('inbox'), View({ as: 'h1' }, [heading])]),
        View({ as: 'span', class: 'jx-muted jx-list-count' }, [computed(s.threads, t => `${t.length} 条对话`)]),
        Button({ store: ui.refresh, variant: 'ghost', icon: 'refresh', class: 'jx-icon-button' }, [])]),
      Show({ when: s.error, ok: () => EmptyState({ title: '邮件加载失败', description: s.error }, [Button({ store: ui.retry })]) }),
      Show({ when: combine({ loading: s.loading, error: s.error }, d => d.loading && !d.error), ok: () => View({ class: 'jx-loading', attributes: { role: 'status' } }, ['正在读取本地邮件…']) }),
      View({ class: 'jx-conversation-list', attributes: { 'aria-busy': computed(s.loading, v => String(v)) } }, [
        For({ each: s.threads, render(thread) {
          const isSelected = value => (value?.kind === 'message' ? value.thread_id : value?.id) === thread.id;
          const core = ControlCore({ label: thread.subject, href: `${props.root}/thread/${thread.id}`, active: isSelected(s.selected.value) }, { click() { methods.select(thread); } });
          const off = s.selected.subscribe({ onChange(value) { core.set({ active: isSelected(value) }); } });
          return View({ onUnmounted() { off(); core.destroy(); } }, [ConversationRow({ store: core, thread })]);
        } }),
        Show({ when: combine({ loading: s.loading, threads: s.threads, error: s.error }, d => !d.loading && !d.error && !d.threads.length), ok: () => EmptyState({
          title: computed(s.query, q => q ? '没有找到匹配的邮件' : '这里还没有邮件'),
          description: computed(s.query, q => q ? '试试其他主题、发件人或摘要关键词。' : '后台同步完成后，点击刷新查看本地邮件。'),
        }) }),
      ]),
      Toolbar({ class: 'jx-pagination', attributes: { 'aria-label': '对话分页' } }, [
        View({ as: 'span', class: 'jx-muted' }, [computed(s.page, p => `第 ${p + 1} 页`)]),
        Button({ store: ui.prev, variant: 'ghost', icon: 'prev', class: 'jx-icon-button' }, []),
        Button({ store: ui.next, variant: 'ghost', icon: 'next', class: 'jx-icon-button' }, []),
      ]),
    ]);
  }
  function Reader() {
    const owned = [];
    const computed = (deps, fn) => { const value = Timeless.computed(deps, fn); owned.push(value); return value; };
    return Panel({ kind: 'reader', as: 'main', id: 'mail-content', onUnmounted() { owned.forEach(value => value.destroy()); }, attributes: { 'aria-label': '邮件阅读' } }, [
      Toolbar({}, [Button({ store: ui.back, variant: 'ghost', icon: 'back' }, ['返回列表']),
        View({ class: 'jx-spacer' }), Button({ store: ui.text, variant: 'ghost', icon: 'text' }), Button({ store: ui.images, variant: 'ghost', icon: 'image' }),
      ]),
      View({ class: 'jx-reader-scroll' }, [
        Show({ when: s.detail, ok: () => View({ class: 'jx-reader-heading' }, [
          View({ as: 'h2' }, [computed(s.detail, d => d?.subject || '（无主题）')]),
          View({ class: 'jx-row jx-muted' }, [computed(s.detail, d => `${d?.message_count || 1} 封邮件`),
            Badge({}, [computed(s.detail, d => `${d?.unread_count || 0} 封未读`)])]),
          View({ as: 'p', class: 'jx-reader-notice' }, [computed(s.options, o => o.images ? '已允许 HTTPS 远程图片。' : '远程图片已屏蔽。')]),
        ]) }),
        Show({ when: s.detailError, ok: () => EmptyState({ title: '邮件读取失败', description: s.detailError }, [Button({ store: ui.retryDetail })]) }),
        For({ each: s.messages, render(store) { return MailMessage({ store, service: vm.services.mail }); } }),
        Show({ when: s.detailLoading, ok: () => View({ class: 'jx-loading', attributes: { role: 'status' } }, ['正在读取邮件…']) }),
        Show({ when: s.detailMore, ok: () => View({ class: 'jx-load-more' }, [Button({ store: ui.loadMore })]) }),
      ]),
    ]);
  }
  return View({ class: combine({ selected, sidebar: s.sidebar }, d => `jx-app ${d.selected ? 'has-reader' : ''} ${d.sidebar ? 'has-sidebar' : ''}`),
    onMounted() { window.addEventListener('popstate', methods.restore); document.addEventListener('visibilitychange', refreshStatuses); statusTimer = setInterval(refreshStatuses, 5000); methods.init(); },
    onUnmounted() { agent.destroy(); agentToggle.destroy(); offAgentScope(); clearInterval(statusTimer); document.removeEventListener('visibilitychange', refreshStatuses); window.removeEventListener('popstate', methods.restore); derived.forEach(value => value.destroy()); methods.destroy(); vm.destroy(); },
  }, [Sidebar(), View({ class: 'jx-workspace' }, [
    Toolbar({ as: 'header', class: 'jx-header' }, [Button({ store: ui.menu, variant: 'ghost', icon: 'menu', class: 'jx-menu-button' }, []),
      View({ as: 'form', class: 'jx-search', attributes: { role: 'search' }, onMounted({ target }) {
        target.addEventListener('submit', submit);
        function submit(e) { e.preventDefault(); methods.search(); }
        cleanupSearch = () => target.removeEventListener('submit', submit);
      }, onUnmounted() { cleanupSearch(); } }, [Glyph('search'), Input({ store: ui.search, label: '搜索邮件', placeholder: '搜索主题、发件人或邮件摘要…' }),
        Button({ store: ui.searchButton, variant: 'ghost', icon: 'search', class: 'jx-icon-button' }, [])]),
      View({ class: 'jx-header-caption jx-row jx-muted' }, [Glyph('clock'), '专注于每一封邮件']), NotificationCenter({ store: ui.notifications }), Button({ store: agentToggle, icon: 'agent', variant: 'ghost' }),
    ]),
    View({ class: 'jx-main-row' }, [View({ class: 'jx-split' }, [List(), Show({ when: selected, ok: Reader })]), AgentPanel({ store: agent })]),
  ])]);
}
