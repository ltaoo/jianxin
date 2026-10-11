import { AgentCore } from '../../../packages/jxui/agent-core.js';
import { AgentPanel } from '../../../packages/jxui/agent-panel.js';
import { AgentService } from '../../services.js';
import { Button, Input, MailboxGroup, NavItem, Panel, Toolbar, EmptyState, ConversationRow, Badge, Glyph, NotificationCenter } from '../../../packages/jxui/index.js';
import { ControlCore } from '../../../packages/jxui/core.js';
import { MailMessage } from '../../../packages/jxui/mail-message.js';
import { ReplyWindow } from '../../../packages/jxui/reply-window.js';
import { ConfigImport } from '../../../packages/jxui/config-import.js';
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
  const heading = combine({ folders: s.folders, folder: s.folder, unread: s.unread, starred: s.starred, query: s.query }, d =>
    d.query.trim() ? '搜索结果' : d.starred ? '收藏' : d.unread ? '未读邮件' : d.folders.find(f => f.id === d.folder)?.name || '全部邮件');
  const selected = computed(s.selected, value => !!value);
  function nav(store, field, predicate, icon) {
    const off = field.subscribe({ onChange(value) { store.set({ active: predicate(value) }); } });
    const offAccount = s.account.subscribe({ onChange() { store.set({ active: predicate(field.value) }); } });
    const offUnread = s.unread.subscribe({ onChange() { store.set({ active: predicate(field.value) }); } });
    const offStarred = s.starred.subscribe({ onChange() { store.set({ active: predicate(field.value) }); } });
    store.set({ active: predicate(field.value) });
    return View({ attributes: { n: `mail-navigation-${icon}` }, onUnmounted() { off(); offUnread(); offStarred(); offAccount(); } }, [NavItem({ store, icon, showCount: true })]);
  }
  function Sidebar() {
    return Panel({ kind: 'sidebar', as: 'aside', attributes: { 'aria-label': '邮箱导航' } }, [
      View({ class: 'jx-brand' }, [View({ class: 'jx-brand-mark' }, [Glyph('mail')]), View({ class: 'jx-brand-copy' }, [View({ as: 'strong' }, ['简信']), View({ as: 'span' }, ['JIANXIN · 邮件工作区'])])]),
      View({ as: 'nav', class: 'jx-navigation' }, [
        nav(ui.all, s.folder, value => !s.account.value && !value && !s.unread.value && !s.starred.value, 'all'),
        nav(ui.unread, s.unread, value => !s.account.value && value && !s.starred.value, 'unread'),
        nav(ui.starred, s.starred, value => !s.account.value && value, 'starred'),
        View({ class: 'jx-section-label jx-row' }, ['我的邮箱', Badge({}, [computed(s.accounts, items => String(items.length))])]),
        For({ each: s.accountGroups, render(group) {
          return MailboxGroup({ store: group, root: props.root }, (folder, accountId) => {
            const active = () => s.account.value === accountId && s.folder.value === folder.id;
            const core = ControlCore({ label: folderLabels[folder.kind] || folder.name, active: active() }, { click() { methods.selectFolder(accountId, folder.id); } });
            const offs = [s.folder, s.account].map(field => field.subscribe({ onChange() { core.set({ active: active() }); } }));
            return View({ onUnmounted() { offs.forEach(off => off()); core.destroy(); } }, [NavItem({ store: core, icon: folder.kind, count: folder.unread_count })]);
          });
        } }),
      ]),
      View({ class: 'jx-sidebar-footer', attributes: { n: 'mailbox-sidebar-footer' } }, [
        Button({ store: ui.importMailboxes, variant: 'primary', icon: 'mail', attributes: { n: 'import-mailbox-config' } }),
        Button({ store: ui.reloadMailboxes, variant: 'ghost', attributes: { n: 'refresh-mailbox-config' } }),
        Show({ when: s.configStatus, ok: () => View({ as: 'p', class: 'jx-muted', attributes: { n: 'mailbox-config-feedback', role: 'status' } }, [s.configStatus]) }),
        View({ class: 'jx-row' }, [Glyph('server'), '本地邮件阅读器']),
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
          return ConversationRow({ store: methods.conversation(thread), thread, mailbox: methods.mailbox(thread.account_id), root: props.root, favorite: s.starred.value });
        } }),
        Show({ when: combine({ loading: s.loading, threads: s.threads, error: s.error }, d => !d.loading && !d.error && !d.threads.length), ok: () => EmptyState({
          title: combine({ query: s.query, starred: s.starred }, d => d.query ? '没有找到匹配的邮件' : d.starred ? '暂无收藏邮件' : '这里还没有邮件'),
          description: combine({ query: s.query, starred: s.starred }, d => d.query ? '试试其他主题、发件人或摘要关键词。' : d.starred ? '打开邮件，点击“收藏”即可在这里查看。' : '后台同步完成后，点击刷新查看本地邮件。'),
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
    const combine = (deps, fn) => { const value = Timeless.combine(deps, fn); owned.push(value); return value; };
    const folderName = combine({ messages: s.messages, accounts: s.accounts }, d => {
      const message = d.messages[0]?.state;
      const folder = d.accounts.find(account => account.id === message?.account_id)?.folders.find(folder => folder.id === message?.folder_id);
      return folder ? folderLabels[folder.kind] || folder.name : '';
    });
    return Panel({ kind: 'reader', as: 'main', id: 'mail-content', onUnmounted() { owned.forEach(value => value.destroy()); }, attributes: { 'aria-label': '邮件阅读' } }, [
      Toolbar({ class: 'jx-reader-toolbar', attributes: { n: 'reader-toolbar' } }, [Button({ store: ui.back, variant: 'ghost', icon: 'back', class: 'jx-icon-button', attributes: { n: 'reader-back' } }, []),
        Button({ store: ui.text, variant: 'ghost', icon: 'text', class: 'jx-icon-button', attributes: { n: 'reader-text-toggle' } }, []),
        Button({ store: ui.images, variant: 'ghost', icon: 'image', class: 'jx-icon-button', attributes: { n: 'reader-images-toggle' } }, []),
        Button({ store: ui.readerWidth, variant: 'ghost', icon: 'expand', class: 'jx-icon-button jx-reader-width-toggle', attributes: { n: 'reader-width-toggle', 'aria-pressed': computed(s.readerWide, value => String(value)) } }, []),
        View({ class: 'jx-spacer', attributes: { n: 'reader-toolbar-spacer' } }),
        View({ as: 'span', class: 'jx-muted jx-reader-position', attributes: { n: 'reader-conversation-position', role: 'status' } }, [s.conversationPosition]),
        Button({ store: ui.previousConversation, variant: 'ghost', icon: 'prev', class: 'jx-icon-button', attributes: { n: 'reader-previous-conversation' } }, []),
        Button({ store: ui.nextConversation, variant: 'ghost', icon: 'next', class: 'jx-icon-button', attributes: { n: 'reader-next-conversation' } }, []),
      ]),
      View({ class: 'jx-reader-scroll', attributes: { n: 'reader-scroll' } }, [
        Show({ when: s.detail, ok: () => View({ class: 'jx-reader-heading', attributes: { n: 'reader-heading' } }, [
          View({ class: 'jx-reader-subject-row', attributes: { n: 'reader-subject-row' } }, [
            View({ as: 'h2', attributes: { n: 'reader-subject' } }, [computed(s.detail, d => d?.subject || '（无主题）')]),
            Show({ when: folderName, ok: () => View({ as: 'span', class: 'jx-badge', attributes: { n: 'reader-folder-label' } }, [folderName]) }),
          ]),
          Show({ when: computed(s.detail, d => (d?.message_count || 1) > 1), ok: () => View({ as: 'p', class: 'jx-reader-notice', attributes: { n: 'reader-message-count' } }, [computed(s.detail, d => `${d?.message_count || 1} 封邮件`)]) }),
          Show({ when: computed(s.options, o => !o.images), ok: () => View({ as: 'p', class: 'jx-reader-notice', attributes: { n: 'reader-images-notice' } }, ['远程图片已屏蔽']) }),
        ]) }),
        Show({ when: s.detailError, ok: () => EmptyState({ title: '邮件读取失败', description: s.detailError }, [Button({ store: ui.retryDetail })]) }),
        For({ each: s.messages, render(store) { return MailMessage({ store, service: vm.services.mail }); } }),
        Show({ when: s.detailLoading, ok: () => View({ class: 'jx-loading', attributes: { role: 'status' } }, ['正在读取邮件…']) }),
        Show({ when: s.detailMore, ok: () => View({ class: 'jx-load-more' }, [Button({ store: ui.loadMore })]) }),
      ]),
    ]);
  }
  return View({ class: combine({ selected, sidebar: s.sidebar, readerWide: s.readerWide }, d => `jx-app ${d.selected ? 'has-reader' : ''} ${d.sidebar ? 'has-sidebar' : ''} ${d.readerWide ? 'is-reader-wide' : ''}`),
    onMounted() { window.addEventListener('popstate', methods.restore); document.addEventListener('visibilitychange', refreshStatuses); statusTimer = setInterval(refreshStatuses, 5000); methods.init(); if (props.location.hash === '#config-import') ui.configImport.open(); },
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
  ]), ReplyWindow({ store: ui.reply }), ConfigImport({ store: ui.configImport })]);
}
