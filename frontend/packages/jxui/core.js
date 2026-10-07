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
  const core = ControlCore({ ...account, active: false, expanded: true }, actions);
  return Object.assign(core, {
    toggle() { core.set({ expanded: !core.state.expanded }); },
    updateAccount(next) { core.set(next); },
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
