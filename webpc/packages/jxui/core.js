/** Small control Core shared by the jxui views; business state stays in the page Model. */
export function ControlCore(initial = {}, actions = {}) {
  let state = { label: '', disabled: false, active: false, value: '', ...initial };
  const listeners = new Set();
  return {
    get state() { return state; },
    onStateChange(listener) { listeners.add(listener); return () => listeners.delete(listener); },
    set(next) { state = { ...state, ...next }; for (const listener of listeners) listener(state); },
    click(event) { if (!state.disabled) actions.click?.(event); },
    input(value) { if (!state.disabled) { this.set({ value }); actions.input?.(value); } },
    keydown(event) { actions.keydown?.(event); },
    destroy() { listeners.clear(); },
  };
}

/** Persistent presentation Core for one mailbox, including its collapsed state. */
export function MailboxCore(account, actions) {
  const platforms = { gmail: 'gmail', google: 'gmail', outlook: 'outlook', microsoft: 'outlook',
    qq: 'qq', foxmail: 'foxmail', '163': 'netease163', netease163: 'netease163', '126': 'netease126', netease126: 'netease126',
    yeah: 'yeah', aliyun: 'aliyun', sina: 'sina', '139': 'mail139', mail139: 'mail139', icloud: 'icloud',
    zoho: 'zoho', yahoo: 'yahoo', fastmail: 'fastmail', proton: 'proton', tencent: 'tencent', exmail: 'tencent' };
  const platformIcon = next => {
    const platform = String(next.platform || '').trim().toLowerCase() || String(next.provider || '').trim().toLowerCase();
    return Object.hasOwn(platforms, platform) ? platforms[platform] : '';
  };
  const problem = next => ['warning', 'error'].includes(next.status?.tone) ? next.status.label || '邮箱状态异常' : '';
  const core = ControlCore({ ...account, platformIcon: platformIcon(account), problem: problem(account),
    active: false, expanded: true, statusOpen: false, oauth: { state: 'idle', busy: false },
    oauthDraft: { client_id: account.oauth_config?.client_id || '', token_endpoint: account.oauth_config?.token_endpoint || '' } }, actions);
  return Object.assign(core, {
    toggle() { core.set({ expanded: !core.state.expanded }); },
    toggleStatus() { if (core.state.problem) core.set({ statusOpen: !core.state.statusOpen }); },
    closeStatus() { if (core.state.statusOpen) core.set({ statusOpen: false }); },
    startOAuth() { if (core.state.status?.state === 'oauth_required' && !core.state.oauth.busy && !core.state.oauth_config?.needs_setup) return actions?.login?.(core); },
    editOAuth(field, value) {
      if (!core.state.oauth.busy && ['client_id', 'token_endpoint'].includes(field)) core.set({ oauthDraft: { ...core.state.oauthDraft, [field]: value } });
    },
    saveOAuthSettings() { if (!core.state.oauth.busy) return actions?.saveOAuth?.(core); },
    updateAccount(next) {
      const nextProblem = problem(next);
      core.set({ ...next, platformIcon: platformIcon(next), problem: nextProblem,
        ...(JSON.stringify(next.oauth_config) !== JSON.stringify(core.state.oauth_config)
          ? { oauthDraft: { client_id: next.oauth_config?.client_id || '', token_endpoint: next.oauth_config?.token_endpoint || '' } } : {}),
        statusOpen: !!nextProblem && nextProblem === core.state.problem && core.state.statusOpen });
    },
  });
}

/** Bounded notification history; repeated failures retain the user's read state. */
export function NotificationCore({ storage, now = () => Date.now() } = {}) {
  let entries = [];
  try {
    const saved = JSON.parse(storage?.getItem('jianxin.notifications') || '[]');
    if (Array.isArray(saved)) entries = saved.filter(item => item && typeof item.id === 'string' && typeof item.title === 'string' && typeof item.message === 'string' && Number.isFinite(item.timestamp)).slice(0, 100);
  } catch {}
  const core = ControlCore({ open: false, entries, unread: entries.filter(item => !item.read).length });
  function save(next) {
    core.set({ entries: next, unread: next.filter(item => !item.read).length });
    try { storage?.setItem('jianxin.notifications', JSON.stringify(next)); } catch {}
  }
  return Object.assign(core, {
    toggle() { core.set({ open: !core.state.open }); },
    close() { core.set({ open: false }); },
    record(id, title, message) {
      const previous = core.state.entries.find(item => item.id === id);
      const entry = { id, title, message, timestamp: Math.floor(now() / 1000), read: previous?.read || false, count: (previous?.count || 0) + 1 };
      save([entry, ...core.state.entries.filter(item => item.id !== id)].slice(0, 100));
    },
    setRead(id, read) { save(core.state.entries.map(item => item.id === id ? { ...item, read } : item)); },
    readAll() { save(core.state.entries.map(item => ({ ...item, read: true }))); },
  });
}
