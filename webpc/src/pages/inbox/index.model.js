import { ControlCore, MailboxCore, NotificationCore } from '../../../packages/jxui/core.js';
import { MailService } from '../../services.js';
import { ReplyCore } from '../../../packages/jxui/reply-core.js';
import { ConfigImportCore } from '../../../packages/jxui/config-import-core.js';

export function MessageCore(message, { service, options, expanded, changed, starredChanged = () => {}, started = () => {}, settled = () => {}, failed = () => {}, reply = () => {} }) {
  const core = ControlCore({ ...message, expanded, options, pending: false, error: '', status: '', autoAttempted: false,
    infoPanel: '', menuOpen: false, metadata: null, metadataPending: false, metadataError: '', starPending: false, starError: '' });
  const destroyCore = core.destroy.bind(core);
  let destroyed = false;
  let operation, metadataRequest, starOperation;
  return Object.assign(core, {
    toggle() { core.set({ expanded: !core.state.expanded }); },
    configure(next) { core.set({ options: next }); },
    toggleMenu() { if (!destroyed) core.set({ menuOpen: !core.state.menuOpen }); },
    closeMenu() { if (core.state.menuOpen) core.set({ menuOpen: false }); },
    openReply() { if (!destroyed) { this.closeMenu(); return reply(message.id); } },
    async setStarred(starred) {
      if (destroyed || core.state.starPending || !!core.state.is_starred === starred) return;
      const previous = !!core.state.is_starred;
      starOperation = new AbortController();
      started();
      core.set({ is_starred: starred, starPending: true, starError: '' });
      starredChanged({ ...message, is_starred: starred }, previous);
      try {
        const result = await service.setStarred(message.id, starred, starOperation.signal);
        if (!destroyed) core.set({ is_starred: result.is_starred });
        starredChanged({ ...message, ...result }, starred);
      } catch (error) {
        starredChanged({ ...message, is_starred: previous }, starred);
        if (!destroyed) core.set({ is_starred: previous, starError: error.name === 'AbortError' ? '' : `收藏失败：${error.message}` });
      } finally {
        if (!destroyed) core.set({ starPending: false });
        settled();
      }
    },
    toggleInfo(panel) {
      if (destroyed || !['sender', 'metadata'].includes(panel)) return;
      core.set({ infoPanel: core.state.infoPanel === panel ? '' : panel, menuOpen: false });
      if (core.state.infoPanel && !core.state.metadata && !core.state.metadataPending) return this.loadMetadata();
    },
    closeInfo() { core.set({ infoPanel: '' }); },
    async loadMetadata() {
      if (destroyed || core.state.metadataPending) return;
      metadataRequest = new AbortController(); core.set({ metadataPending: true, metadataError: '' });
      try {
        const metadata = await service.metadata(message.id, metadataRequest.signal);
        if (!destroyed) core.set({ metadata });
      } catch (error) {
        if (!destroyed && error.name !== 'AbortError') core.set({ metadataError: error.message });
      } finally { if (!destroyed) core.set({ metadataPending: false }); }
    },
    async setRead(read) {
      if (core.state.pending || destroyed) return;
      const previous = core.state.is_read;
      if (previous === read) return;
      operation = new AbortController();
      started();
      core.set({ is_read: read, pending: true, error: '', status: '', autoAttempted: true });
      changed({ ...message, is_read: read }, previous);
      try {
        const result = await service.setRead(message.id, read, operation.signal);
        if (!destroyed) core.set({ is_read: result.is_read, status: '' });
        changed({ ...message, ...result }, read);
      } catch (error) {
        changed({ ...message, is_read: previous }, read);
        if (!destroyed) core.set({ is_read: previous, error: error.name === 'AbortError' ? '' : `操作失败：${error.message}`, status: '' });
        if (error.name !== 'AbortError') failed();
      } finally {
        if (!destroyed) core.set({ pending: false });
        settled();
      }
    },
    loaded(visible) {
      if (visible && core.state.expanded && !core.state.is_read && !core.state.autoAttempted) return this.setRead(true);
    },
    destroy() { destroyed = true; operation?.abort(); metadataRequest?.abort(); starOperation?.abort(); destroyCore(); },
  });
}

export function InboxPageModel({ root, location, history, storage, service = MailService(root), timers = globalThis }) {
  const initial = new URL(location.href);
  const state = {
    accounts: ref([]), accountGroups: ref([]), folders: ref([]), account: ref(initial.searchParams.get('account') || ''),
    folder: ref(initial.searchParams.get('folder') || ''), query: ref(initial.searchParams.get('q') || ''),
    unread: ref(initial.searchParams.get('unread') === '1'), page: ref(Number(initial.searchParams.get('page')) || 0),
    starred: ref(initial.searchParams.get('starred') === '1'),
    threads: ref([]), more: ref(false), loading: ref(true), error: ref(''),
    configStatus: ref(''),
    selected: ref(null), messages: ref([]), detail: ref(null), detailLoading: ref(false), detailError: ref(''),
    detailMore: ref(false), detailPage: ref(0), sidebar: ref(false), readerWide: ref(false),
    conversationPosition: ref(''),
    options: ref({ images: initial.searchParams.get('images') !== '0', text: initial.searchParams.get('text') === '1' }),
  };
  let listRequest, detailRequest, mailboxRequest, destroyed = false;
  let pendingWrites = 0, flagVersion = 0;
  const threadCounts = new Map();
  const notifications = NotificationCore({ storage });
  const controls = [notifications];
  const accountCores = new Map();
  const conversationCores = new Map(), unstarRequests = new Map();
  const loginRequests = new Map();
  function stopLogin(id) {
    const request = loginRequests.get(id);
    request?.controller.abort(); timers.clearTimeout(request?.timer); loginRequests.delete(id);
  }
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
      q: state.query.value, unread: state.unread.value ? '1' : '', starred: state.starred.value ? '1' : '', page: state.page.value || '' })) {
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
    reply: ReplyCore({ service, timers, replied() { methods.refresh({ silent: true }); } }),
    configImport: ConfigImportCore({ service, imported() { methods.init(); } }),
    importMailboxes: control({ label: '添加邮箱' }, { click() { ui.configImport.open(); } }),
    search: control({ value: state.query.value }, { input() {},
      keydown(event) { if (event.key === 'Enter') { event.preventDefault(); methods.search(); } if (event.key === 'Escape') { state.query.as(''); ui.search.set({ value: '' }); methods.search(); } } }),
    searchButton: control({ label: '搜索邮件' }, { click() { methods.search(); } }),
    refresh: control({ label: '刷新本地邮件' }, { click() { methods.refresh(); } }),
    reloadMailboxes: control({ label: '刷新邮箱配置' }, { click() { methods.reloadMailboxes(); } }),
    prev: control({ label: '上一页', disabled: state.page.value === 0 }, { click() { methods.paginate(-1); } }),
    next: control({ label: '下一页', disabled: true }, { click() { methods.paginate(1); } }),
    back: control({ label: '返回邮件列表' }, { click() { methods.close(); } }),
    readerWidth: control({ label: '展开阅读' }, { click() { methods.toggleReaderWidth(); } }),
    previousConversation: control({ label: '上一封对话', disabled: true }, { click() { methods.navigateConversation(-1); } }),
    nextConversation: control({ label: '下一封对话', disabled: true }, { click() { methods.navigateConversation(1); } }),
    menu: control({ label: '切换邮箱侧栏' }, { click() { state.sidebar.toggle(); } }),
    images: control({ label: state.options.value.images ? '屏蔽远程图片' : '加载远程图片' }, { click() { methods.configure({ images: !state.options.value.images }); } }),
    text: control({ label: state.options.value.text ? 'HTML 视图' : '纯文本视图' }, { click() { methods.configure({ text: !state.options.value.text }); } }),
    all: control({ label: '全部邮件', count: 0 }, { click() { methods.selectFolder('', '', false); } }),
    unread: control({ label: '未读邮件', count: 0 }, { click() { methods.selectFolder('', '', true); } }),
    starred: control({ label: '收藏', count: 0 }, { click() { methods.selectFolder('', '', false, true); } }),
    retry: control({ label: '重新加载', disabled: false }, { click() { if (!state.accounts.value.length) methods.init(); else methods.refresh(); } }),
    retryDetail: control({ label: '重新加载邮件' }, { click() { methods.loadDetail(); } }),
    loadMore: control({ label: '加载后续邮件' }, { click() { methods.loadDetail(true); } }),
  };
  controls.push(ui.reply, ui.configImport);
  function conversationIndex() {
    const selected = state.selected.value;
    return selected ? state.threads.value.findIndex(thread => thread.id === (selected.kind === 'message' ? selected.thread_id : selected.id)) : -1;
  }
  function updateConversationNavigation() {
    const index = conversationIndex(), length = state.threads.value.length;
    const selected = state.selected.value;
    const selectedId = selected?.kind === 'message' ? selected.thread_id : selected?.id;
    for (const [id, core] of conversationCores) {
      if (core.state.active !== (id === selectedId)) core.set({ active: id === selectedId });
    }
    state.conversationPosition.as(index < 0 ? '' : `第 ${index + 1} 个会话，共 ${length} 个${state.more.value ? '（本页）' : ''}`);
    ui.previousConversation.set({ disabled: index <= 0 });
    ui.nextConversation.set({ disabled: index < 0 || index >= length - 1 });
  }
  groupSubscriptions.push(...[state.selected, state.threads, state.more].map(field => field.subscribe({ onChange: updateConversationNavigation })));
  const methods = {
    conversation(thread) {
      let core = conversationCores.get(thread.id);
      if (!core) {
        const selected = state.selected.value;
        const active = (selected?.kind === 'message' ? selected.thread_id : selected?.id) === thread.id;
        core = ControlCore({ label: thread.subject, href: `${root}/thread/${thread.id}`, active, menuOpen: false, pending: false, error: '' }, { click() { methods.select(thread); } });
        Object.assign(core, {
          toggleMenu() { core.set({ menuOpen: !core.state.menuOpen }); },
          closeMenu() { if (core.state.menuOpen) core.set({ menuOpen: false }); },
          unstar() { return methods.unstarConversation(thread.id, core); },
        });
        conversationCores.set(thread.id, core);
      }
      return core;
    },
    async unstarConversation(id, core) {
      if (destroyed || core.state.pending) return;
      const request = new AbortController(); unstarRequests.set(id, request);
      core.set({ menuOpen: false, pending: true, error: '' }); pendingWrites++; flagVersion++;
      try {
        // Fetch all headers before writing, including starred mails after the first page.
        const messages = [];
        for (let page = 0; ; page++) {
          const result = await service.detail('thread', id, page, request.signal);
          if (destroyed || request.signal.aborted) return;
          messages.push(...result.messages.filter(message => message.is_starred));
          if (!result.more) break;
        }
        for (const message of messages) {
          const result = await service.setStarred(message.id, false, request.signal);
          if (destroyed || request.signal.aborted) return;
          methods.starredChanged({ ...message, ...result }, true);
          state.messages.value.find(core => core.state.id === message.id)?.set({ is_starred: result.is_starred, starError: '' });
        }
      } catch (error) {
        if (!destroyed && error.name !== 'AbortError') core.set({ error: `取消收藏失败：${error.message}，请重试。` });
      } finally {
        unstarRequests.delete(id); pendingWrites--; flagVersion++;
        if (!destroyed) { core.set({ pending: false }); await methods.refresh({ silent: true }); }
      }
    },
    toggleReaderWidth() { state.readerWide.toggle(); ui.readerWidth.set({ label: state.readerWide.value ? '恢复分栏' : '展开阅读' }); },
    navigateConversation(delta) {
      const index = conversationIndex();
      if (index < 0 || ![-1, 1].includes(delta)) return;
      const thread = state.threads.value[index + delta];
      if (thread) methods.select(thread);
    },
    mailbox(id) { return accountCores.get(id); },
    async loadMailboxes({ reload = false } = {}) {
      mailboxRequest?.abort(); const request = mailboxRequest = new AbortController();
      const version = flagVersion;
      const result = await service[reload ? 'reloadMailboxes' : 'mailboxes'](request.signal);
      if (destroyed || mailboxRequest !== request) return;
      const preserveCounts = pendingWrites > 0 || version !== flagVersion;
      if (result.counts && !preserveCounts) {
        ui.all.set({ count: result.counts.total });
        ui.unread.set({ count: result.counts.unread });
        ui.starred.set({ count: result.counts.starred || 0 });
      }
      const accounts = result.accounts.map(account => preserveCounts ? { ...account, folders: account.folders.map(folder => {
        const current = accountCores.get(account.id)?.state.folders.find(item => item.id === folder.id);
        return current ? { ...folder, unread_count: current.unread_count } : folder;
      }) } : account);
      const groups = accounts.map(account => {
        let core = accountCores.get(account.id);
        if (!core) {
          core = MailboxCore(account, { click() { methods.selectAccount(account.id); }, login(core) { return methods.startOAuth(core); }, saveOAuth(core) { return methods.saveOAuthSettings(core); } });
          accountCores.set(account.id, core);
        } else {
          // Keep folder VNodes and focus when only the sync status changes.
          const folders = JSON.stringify(core.state.folders) === JSON.stringify(account.folders) ? core.state.folders : account.folders;
          if (core.state.email !== account.email || core.state.platform !== account.platform || core.state.provider !== account.provider || folders !== core.state.folders || JSON.stringify(core.state.status) !== JSON.stringify(account.status) || JSON.stringify(core.state.oauth_config) !== JSON.stringify(account.oauth_config)) {
            core.updateAccount({ ...account, folders });
          }
        }
        return core;
      });
      for (const [id, core] of accountCores) {
        if (!result.accounts.some(account => account.id === id)) { stopLogin(id); core.destroy(); accountCores.delete(id); }
      }
      if (JSON.stringify(state.accounts.value) !== JSON.stringify(accounts)) state.accounts.as(accounts);
      if (groups.length !== state.accountGroups.value.length || groups.some((core, i) => core !== state.accountGroups.value[i])) state.accountGroups.as(groups);
      updateFolders(); updateGroupSelection();
      return result;
    },
    async saveOAuthSettings(core) {
      if (destroyed || core.state.oauth.busy) return;
      const { client_id, token_endpoint } = core.state.oauthDraft;
      if (!client_id.trim()) { core.set({ oauth: { state: 'failed', busy: false, error: '请填写 OAuth 应用 ID（Client ID）' } }); return; }
      core.set({ oauth: { state: 'saving', busy: true } });
      try {
        await service.saveOAuthSettings(core.state.id, { client_id: client_id.trim(), token_endpoint: token_endpoint.trim() });
        if (destroyed) return;
        await methods.loadMailboxes();
        if (!destroyed) core.set({ oauth: { state: 'idle', busy: false } });
      } catch (error) { if (!destroyed) core.set({ oauth: { state: 'failed', busy: false, error: error.message } }); }
    },
    async startOAuth(core) {
      const id = core.state.id;
      if (destroyed || loginRequests.has(id)) return;
      const request = { controller: new AbortController() };
      loginRequests.set(id, request);
      core.set({ statusOpen: true, oauth: { state: 'starting', busy: true } });
      async function update(start = false) {
        try {
          const result = await service[start ? 'startOAuth' : 'oauthStatus'](id, request.controller.signal);
          if (destroyed || loginRequests.get(id) !== request) return;
          if (result.verification_uri) {
            const url = new URL(result.verification_uri);
            if (url.protocol !== 'https:' || url.username || url.password) throw new Error('授权服务返回了无效的 HTTPS 登录地址');
          }
          core.set({ oauth: result });
          if (result.busy) request.timer = timers.setTimeout(() => update(), 1000);
          else {
            loginRequests.delete(id);
            if (result.state === 'complete') {
              const refreshed = await methods.reloadMailboxes();
              if (!destroyed && !refreshed) core.set({ oauth: { ...result, error: '授权成功，请点击侧栏“刷新邮箱配置”更新凭据。' } });
            }
          }
        } catch (error) {
          if (!destroyed && loginRequests.get(id) === request && error.name !== 'AbortError') {
            loginRequests.delete(id); core.set({ oauth: { state: 'failed', busy: false, error: error.message } });
          }
        }
      }
      await update(true);
    },
    async refreshStatuses() {
      if (ui.reloadMailboxes.state.disabled) return;
      try { await methods.loadMailboxes(); }
      catch (error) { if (!destroyed && error.name !== 'AbortError') notifications.record('mailbox-status', '邮箱状态更新失败', '暂时无法更新邮箱状态，系统会自动重试。'); }
    },
    async reloadMailboxes() {
      if (destroyed || ui.reloadMailboxes.state.disabled) return;
      state.configStatus.as('');
      ui.reloadMailboxes.set({ disabled: true, label: '正在刷新邮箱配置…' });
      try {
        const result = await methods.loadMailboxes({ reload: true });
        if (destroyed) return;
        state.error.as('');
        if (state.account.value && !state.accounts.value.some(account => account.id === state.account.value)) {
          resetDetail(); state.selected.as(null); state.account.as(''); state.folder.as(''); state.page.as(0); setUrl(true);
        }
        await methods.loadList();
        if (destroyed) return;
        if (result?.daemon?.error) throw new Error(result.daemon.error);
        state.configStatus.as(result?.daemon?.status === 'restarted'
          ? '邮箱配置已刷新，后台收信已重启，正在更新邮箱状态。'
          : '邮箱配置已刷新，当前配置的后台收信服务未运行。');
        return true;
      } catch (error) {
        if (!destroyed && error.name !== 'AbortError') {
          state.configStatus.as(error.message);
          state.error.as(error.message);
          notifications.record('mailbox-config', '邮箱配置刷新失败', error.message);
        }
      } finally {
        if (!destroyed) ui.reloadMailboxes.set({ disabled: false, label: '刷新邮箱配置' });
      }
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
      const version = flagVersion;
      if (!silent) { state.loading.as(true); ui.refresh.set({ disabled: true }); }
      state.error.as('');
      try {
        const result = await service.threads({ account: state.account.value, folder: state.folder.value,
          q: state.query.value, unread: state.unread.value ? '1' : '0', starred: state.starred.value ? '1' : '0', page: String(state.page.value) }, request.signal);
        if (destroyed || listRequest !== request) return;
        if (version !== flagVersion && !pendingWrites) return;
        state.threads.as(result.threads.map(thread => threadCounts.has(thread.id)
          ? { ...thread, unread_count: threadCounts.get(thread.id) } : thread)); state.more.as(result.more);
        for (const [id, core] of conversationCores) {
          if (!core.state.pending && !result.threads.some(thread => thread.id === id)) { core.destroy(); conversationCores.delete(id); }
        }
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
        const result = await service.detail(selected.kind, selected.id, page, request.signal, state.folder.value);
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
        const detail = result.thread || { subject: result.subject, message_count: 1, unread_count: result.messages.filter(m => !m.is_read).length };
        state.detail.as(threadCounts.has(detail.id) ? { ...detail, unread_count: threadCounts.get(detail.id) } : detail);
        const cores = result.messages.map(message => MessageCore(message, {
          service, options: state.options.value,
          expanded: true,
          started() { pendingWrites++; flagVersion++; },
          settled() {
            pendingWrites--; flagVersion++;
            if (!destroyed && !pendingWrites) { threadCounts.clear(); methods.refresh({ silent: true }); }
          },
          failed() { if (!destroyed) notifications.record('read-sync', '操作失败：已读状态同步失败', '邮件状态和未读数量已恢复，请在邮件中重试。'); },
          changed(result, previous) { methods.readChanged(result, previous); },
          starredChanged(result, previous) { methods.starredChanged(result, previous); },
          reply(id) { return ui.reply.open(id); },
        }));
        state.messages.as([...state.messages.value, ...cores]);
        state.detailMore.as(result.more); state.detailPage.as(page);
      } catch (error) { if (!destroyed && detailRequest === request && error.name !== 'AbortError') { state.detailError.as(error.message); notifications.record('mail-detail', '邮件读取失败', '无法读取邮件内容，请稍后重试。'); } }
      finally { if (!destroyed && detailRequest === request) { state.detailLoading.as(false); ui.loadMore.set({ disabled: false }); } }
    },
    select(thread) { resetDetail(); state.selected.as({ kind: 'thread', id: thread.id }); state.sidebar.as(false); setUrl(); methods.loadDetail(); },
    close() { resetDetail(); state.selected.as(null); setUrl(); },
    filter(folder, unread = false, starred = false) { resetDetail(); threadCounts.clear(); state.selected.as(null); state.folder.as(folder); state.unread.as(unread); state.starred.as(starred); state.page.as(0); state.sidebar.as(false); setUrl(); methods.loadList(); },
    selectFolder(account, folder, unread = false, starred = false) {
      state.account.as(account); updateFolders(); methods.filter(folder, unread, starred);
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
    starredChanged(result, previous) {
      if (!destroyed && result.is_starred !== previous) {
        ui.starred.set({ count: Math.max(0, ui.starred.state.count + (result.is_starred ? 1 : -1)) });
      }
    },
    readChanged(result, previous) {
      if (destroyed || result.is_read === previous) return;
      const delta = result.is_read ? -1 : 1;
      const count = value => Math.max(0, (value || 0) + delta);
      ui.unread.set({ count: count(ui.unread.state.count) });
      const detail = state.detail.value;
      const selected = state.selected.value;
      const inFolder = !state.folder.value || state.folder.value === result.folder_id;
      const matchesDetail = detail && (selected?.kind === 'message' ? selected.id === result.id : inFolder && result.thread_id && detail.id === result.thread_id);
      const thread = inFolder && state.threads.value.find(thread => thread.id === result.thread_id);
      const current = threadCounts.get(result.thread_id) ?? thread?.unread_count ?? (matchesDetail ? detail.unread_count : undefined);
      if (inFolder && result.thread_id && current != null) threadCounts.set(result.thread_id, count(current));
      if (matchesDetail) {
        state.detail.as({ ...detail, unread_count: count(detail.unread_count) });
      }
      state.threads.as(state.threads.value.map(thread => {
        if (!inFolder || thread.id !== result.thread_id) return thread;
        return { ...thread, unread_count: threadCounts.get(thread.id) };
      }));
      state.accounts.as(state.accounts.value.map(account => {
        if (account.id !== result.account_id) return account;
        const folders = account.folders.map(folder => folder.id === result.folder_id
          ? { ...folder, unread_count: count(folder.unread_count) } : folder);
        accountCores.get(account.id)?.set({ folders });
        return { ...account, folders };
      }));
      updateFolders();
    },
    restore() {
      const url = new URL(location.href);
      resetDetail();
      threadCounts.clear();
      state.account.as(url.searchParams.get('account') || ''); state.folder.as(url.searchParams.get('folder') || '');
      state.query.as(url.searchParams.get('q') || ''); ui.search.set({ value: state.query.value });
      state.unread.as(url.searchParams.get('unread') === '1'); state.page.as(Number(url.searchParams.get('page')) || 0);
      state.starred.as(url.searchParams.get('starred') === '1');
      methods.configure({ images: url.searchParams.get('images') !== '0', text: url.searchParams.get('text') === '1' }, false);
      updateFolders(); state.selected.as(routeSelection(url));
      methods.loadList(); if (state.selected.value) methods.loadDetail();
    },
    destroy() {
      destroyed = true; listRequest?.abort(); detailRequest?.abort(); mailboxRequest?.abort();
      unstarRequests.forEach(request => request.abort()); unstarRequests.clear();
      conversationCores.forEach(core => core.destroy()); conversationCores.clear();
      for (const id of loginRequests.keys()) stopLogin(id);
      state.messages.value.forEach(core => core.destroy()); controls.forEach(core => core.destroy());
      groupSubscriptions.forEach(off => off()); accountCores.forEach(core => core.destroy()); accountCores.clear();
      Object.values(state).forEach(value => value.destroy());
    },
  };
  return defineModel({ state, methods, ui, services: { mail: service } });
}
