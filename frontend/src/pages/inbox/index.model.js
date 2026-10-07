import { ControlCore, MailboxCore, NotificationCore } from '../../../packages/jxui/core.js';
import { MailService } from '../../services.js';

export function MessageCore(message, { service, options, expanded, changed, failed = () => {} }) {
  const core = ControlCore({ ...message, expanded, options, pending: false, error: '', status: '', autoAttempted: false });
  const destroyCore = core.destroy.bind(core);
  let destroyed = false;
  let operation;
  return Object.assign(core, {
    toggle() { core.set({ expanded: !core.state.expanded }); },
    configure(next) { core.set({ options: next }); },
    async setRead(read) {
      if (core.state.pending || destroyed) return;
      operation = new AbortController();
      core.set({ pending: true, error: '', status: '', autoAttempted: true });
      try {
        const result = await service.setRead(message.id, read, operation.signal);
        if (destroyed) return;
        core.set({ is_read: result.is_read, status: '' });
        changed(result);
      } catch (error) {
        if (!destroyed && error.name !== 'AbortError') { core.set({ error: error.message, status: '' }); failed(); }
      } finally {
        if (!destroyed) core.set({ pending: false });
      }
    },
    loaded(visible) {
      if (visible && core.state.expanded && !core.state.is_read && !core.state.autoAttempted) return this.setRead(true);
    },
    destroy() { destroyed = true; operation?.abort(); destroyCore(); },
  });
}

export function InboxPageModel({ root, location, history, storage, service = MailService(root) }) {
  const initial = new URL(location.href);
  const state = {
    accounts: ref([]), accountGroups: ref([]), folders: ref([]), account: ref(initial.searchParams.get('account') || ''),
    folder: ref(initial.searchParams.get('folder') || ''), query: ref(initial.searchParams.get('q') || ''),
    unread: ref(initial.searchParams.get('unread') === '1'), page: ref(Number(initial.searchParams.get('page')) || 0),
    threads: ref([]), more: ref(false), loading: ref(true), error: ref(''),
    selected: ref(null), messages: ref([]), detail: ref(null), detailLoading: ref(false), detailError: ref(''),
    detailMore: ref(false), detailPage: ref(0), sidebar: ref(false),
    options: ref({ images: initial.searchParams.get('images') !== '0', text: initial.searchParams.get('text') === '1' }),
  };
  let listRequest, detailRequest, mailboxRequest, destroyed = false;
  const notifications = NotificationCore({ storage });
  const controls = [notifications];
  const accountCores = new Map();
  function updateGroupSelection() {
    for (const core of accountCores.values()) core.set({ active: core.state.id === state.account.value });
  }
  const groupSubscriptions = [state.account.subscribe({ onChange: updateGroupSelection })];
  const control = (initial, actions) => { const core = ControlCore(initial, actions); controls.push(core); return core; };
  function routeSelection(url) {
    const path = url.pathname.slice(root.length + 1).split('/');
    return ['thread', 'message'].includes(path[0]) && path[1] ? { kind: path[0], id: path[1] } : null;
  }
  function setUrl(replace = false) {
    const params = new URLSearchParams();
    for (const [key, value] of Object.entries({ account: state.account.value, folder: state.folder.value,
      q: state.query.value, unread: state.unread.value ? '1' : '', page: state.page.value || '' })) {
      if (value !== '') params.set(key, value);
    }
    if (!state.options.value.images) params.set('images', '0');
    if (state.options.value.text) params.set('text', '1');
    const selected = state.selected.value;
    const path = selected ? `${selected.kind}/${encodeURIComponent(selected.id)}` : '';
    const query = params.toString();
    history[replace ? 'replaceState' : 'pushState']({}, '', `${root}/${path}${query ? '?' + query : ''}`);
  }
  function resetDetail() {
    detailRequest?.abort();
    state.messages.value.forEach(core => core.destroy());
    state.messages.as([]); state.detail.as(null); state.detailError.as(''); state.detailMore.as(false);
  }
  function updateFolders() {
    const account = state.accounts.value.find(a => a.id === state.account.value);
    const folders = account?.folders || [];
    if (JSON.stringify(folders) !== JSON.stringify(state.folders.value)) state.folders.as(folders);
  }
  const ui = {
    notifications,
    search: control({ value: state.query.value }, { input() {},
      keydown(event) { if (event.key === 'Enter') { event.preventDefault(); methods.search(); } if (event.key === 'Escape') { state.query.as(''); ui.search.set({ value: '' }); methods.search(); } } }),
    searchButton: control({ label: '搜索邮件' }, { click() { methods.search(); } }),
    refresh: control({ label: '刷新本地邮件' }, { click() { methods.refresh(); } }),
    prev: control({ label: '上一页', disabled: state.page.value === 0 }, { click() { methods.paginate(-1); } }),
    next: control({ label: '下一页', disabled: true }, { click() { methods.paginate(1); } }),
    back: control({ label: '返回邮件列表' }, { click() { methods.close(); } }),
    menu: control({ label: '切换邮箱侧栏' }, { click() { state.sidebar.toggle(); } }),
    images: control({ label: state.options.value.images ? '屏蔽远程图片' : '加载远程图片' }, { click() { methods.configure({ images: !state.options.value.images }); } }),
    text: control({ label: state.options.value.text ? 'HTML 视图' : '纯文本视图' }, { click() { methods.configure({ text: !state.options.value.text }); } }),
    all: control({ label: '全部邮件', count: 0 }, { click() { methods.selectFolder('', '', false); } }),
    unread: control({ label: '未读邮件', count: 0 }, { click() { methods.selectFolder('', '', true); } }),
    retry: control({ label: '重新加载', disabled: false }, { click() { if (!state.accounts.value.length) methods.init(); else methods.refresh(); } }),
    retryDetail: control({ label: '重新加载邮件' }, { click() { methods.loadDetail(); } }),
    loadMore: control({ label: '加载后续邮件' }, { click() { methods.loadDetail(true); } }),
  };
  const methods = {
    async loadMailboxes() {
      mailboxRequest?.abort(); const request = mailboxRequest = new AbortController();
      const result = await service.mailboxes(request.signal);
      if (destroyed || mailboxRequest !== request) return;
      if (result.counts) {
        ui.all.set({ count: result.counts.total });
        ui.unread.set({ count: result.counts.unread });
      }
      const groups = result.accounts.map(account => {
        let core = accountCores.get(account.id);
        if (!core) {
          core = MailboxCore(account, { click() { methods.selectAccount(account.id); } });
          accountCores.set(account.id, core);
        } else {
          // Keep folder VNodes and focus when only the sync status changes.
          const folders = JSON.stringify(core.state.folders) === JSON.stringify(account.folders) ? core.state.folders : account.folders;
          if (core.state.email !== account.email || folders !== core.state.folders || JSON.stringify(core.state.status) !== JSON.stringify(account.status)) {
            core.updateAccount({ ...account, folders });
          }
        }
        return core;
      });
      for (const [id, core] of accountCores) {
        if (!result.accounts.some(account => account.id === id)) { core.destroy(); accountCores.delete(id); }
      }
      if (JSON.stringify(state.accounts.value) !== JSON.stringify(result.accounts)) state.accounts.as(result.accounts);
      if (groups.length !== state.accountGroups.value.length || groups.some((core, i) => core !== state.accountGroups.value[i])) state.accountGroups.as(groups);
      updateFolders(); updateGroupSelection();
    },
    async refreshStatuses() {
      try { await methods.loadMailboxes(); }
      catch (error) { if (!destroyed && error.name !== 'AbortError') notifications.record('mailbox-status', '邮箱状态更新失败', '暂时无法更新邮箱状态，系统会自动重试。'); }
    },
    async refresh({ silent = false } = {}) {
      await Promise.all([methods.loadList({ silent }), methods.loadMailboxes().catch(error => {
        if (!destroyed && error.name !== 'AbortError') { state.error.as(error.message); notifications.record('mailbox-status', '邮箱状态更新失败', '暂时无法更新邮箱状态，请稍后重试。'); }
      })]);
    },
    async init() {
      try {
        await methods.loadMailboxes();
        if (destroyed) return;
        state.selected.as(routeSelection(initial));
        await Promise.all([methods.loadList(), state.selected.value ? methods.loadDetail() : undefined]);
      } catch (error) { if (!destroyed && error.name !== 'AbortError') { state.error.as(error.message); state.loading.as(false); notifications.record('initial-load', '邮件初始化失败', '无法加载本地邮件，请稍后重新加载。'); } }
    },
    async loadList({ silent = false } = {}) {
      listRequest?.abort(); const request = listRequest = new AbortController();
      if (!silent) { state.loading.as(true); ui.refresh.set({ disabled: true }); }
      state.error.as('');
      try {
        const result = await service.threads({ account: state.account.value, folder: state.folder.value,
          q: state.query.value, unread: state.unread.value ? '1' : '0', page: String(state.page.value) }, request.signal);
        if (destroyed || listRequest !== request) return;
        state.threads.as(result.threads); state.more.as(result.more);
        ui.prev.set({ disabled: state.page.value === 0 }); ui.next.set({ disabled: !result.more });
      } catch (error) { if (!destroyed && listRequest === request && error.name !== 'AbortError') { state.error.as(error.message); notifications.record('mail-list', '邮件列表加载失败', '无法读取邮件列表，请稍后重试。'); } }
      finally { if (!destroyed && listRequest === request) { state.loading.as(false); ui.refresh.set({ disabled: false }); } }
    },
    async loadDetail(append = false) {
      if (!state.selected.value || (append && state.detailLoading.value)) return;
      detailRequest?.abort(); const request = detailRequest = new AbortController();
      state.detailLoading.as(true); state.detailError.as(''); ui.loadMore.set({ disabled: true });
      const selected = state.selected.value;
      const page = append ? state.detailPage.value + 1 : 0;
      try {
        const result = await service.detail(selected.kind, selected.id, page, request.signal);
        if (destroyed || detailRequest !== request) return;
        if (selected.kind === 'message' && result.messages[0]) {
          const message = result.messages[0];
          // The server-resolved mailbox wins over stale or missing URL filters.
          if (message.account_id && state.accounts.value.some(account => account.id === message.account_id)) {
            const changedAccount = state.account.value !== message.account_id;
            state.account.as(message.account_id); updateFolders();
            accountCores.get(message.account_id)?.set({ expanded: true });
            if (changedAccount) { state.folder.as(''); methods.loadList(); }
          }
          state.selected.as({ ...selected, thread_id: message.thread_id });
        }
        if (!append) { state.messages.value.forEach(core => core.destroy()); state.messages.as([]); }
        state.detail.as(result.thread || { subject: result.subject, message_count: 1, unread_count: result.messages.filter(m => !m.is_read).length });
        const cores = result.messages.map(message => MessageCore(message, {
          service, options: state.options.value,
          expanded: true,
          failed() { notifications.record('read-sync', '已读状态同步失败', '邮箱服务器未能更新邮件状态，请在邮件中重试。'); },
          changed(result) { methods.readChanged(result); },
        }));
        state.messages.as([...state.messages.value, ...cores]);
        state.detailMore.as(result.more); state.detailPage.as(page);
      } catch (error) { if (!destroyed && detailRequest === request && error.name !== 'AbortError') { state.detailError.as(error.message); notifications.record('mail-detail', '邮件读取失败', '无法读取邮件内容，请稍后重试。'); } }
      finally { if (!destroyed && detailRequest === request) { state.detailLoading.as(false); ui.loadMore.set({ disabled: false }); } }
    },
    select(thread) { resetDetail(); state.selected.as({ kind: 'thread', id: thread.id }); state.sidebar.as(false); setUrl(); methods.loadDetail(); },
    close() { resetDetail(); state.selected.as(null); setUrl(); },
    filter(folder, unread = false) { resetDetail(); state.selected.as(null); state.folder.as(folder); state.unread.as(unread); state.page.as(0); state.sidebar.as(false); setUrl(); methods.loadList(); },
    selectFolder(account, folder, unread = false) {
      state.account.as(account); updateFolders(); methods.filter(folder, unread);
    },
    selectAccount(id) { methods.selectFolder(id, '', false); },
    search() { state.query.as(ui.search.state.value.trim()); resetDetail(); state.selected.as(null); state.page.as(0); setUrl(); methods.loadList(); },
    paginate(delta) { if ((delta < 0 && !state.page.value) || (delta > 0 && !state.more.value)) return; state.page.as(state.page.value + delta); setUrl(); methods.loadList(); },
    configure(next, persist = true) {
      state.options.as({ ...state.options.value, ...next });
      state.messages.value.forEach(core => core.configure(state.options.value));
      ui.images.set({ label: state.options.value.images ? '屏蔽远程图片' : '加载远程图片' });
      ui.text.set({ label: state.options.value.text ? 'HTML 视图' : '纯文本视图' }); if (persist) setUrl(true);
    },
    readChanged(result) {
      const detail = state.detail.value;
      if (detail && result.thread_unread != null) state.detail.as({ ...detail, unread_count: result.thread_unread });
      methods.refresh({ silent: true });
    },
    restore() {
      const url = new URL(location.href);
      resetDetail();
      state.account.as(url.searchParams.get('account') || ''); state.folder.as(url.searchParams.get('folder') || '');
      state.query.as(url.searchParams.get('q') || ''); ui.search.set({ value: state.query.value });
      state.unread.as(url.searchParams.get('unread') === '1'); state.page.as(Number(url.searchParams.get('page')) || 0);
      methods.configure({ images: url.searchParams.get('images') !== '0', text: url.searchParams.get('text') === '1' }, false);
      updateFolders(); state.selected.as(routeSelection(url));
      methods.loadList(); if (state.selected.value) methods.loadDetail();
    },
    destroy() {
      destroyed = true; listRequest?.abort(); detailRequest?.abort(); mailboxRequest?.abort();
      state.messages.value.forEach(core => core.destroy()); controls.forEach(core => core.destroy());
      groupSubscriptions.forEach(off => off()); accountCores.forEach(core => core.destroy()); accountCores.clear();
      Object.values(state).forEach(value => value.destroy());
    },
  };
  return defineModel({ state, methods, ui, services: { mail: service } });
}
