import { ControlCore } from './core.js';
export function AgentCore({ service, notify, refresh, context = () => null, timers = globalThis }) {
  const core = ControlCore({ open: false, tab: 'agent', account: '', configured: false, messages: [], trace: [], drafts: [], busy: false, error: '', stream: '', loading: false });
  const destroyCore = core.destroy;
  let generation = 0, loadVersion = 0, timer, destroyed = false;
  function cancelTimer() { timers.clearTimeout(timer); }
  async function load() {
    const version = generation, request = ++loadVersion, account = core.state.account;
    try {
      const state = await service.status(account);
      if (destroyed || version !== generation || request !== loadVersion) return;
      core.set({ ...state, error: state.error || core.state.error, loading: false });
      cancelTimer();
      if (state.error && state.error !== core.state.notifiedError) { core.set({ notifiedError: state.error }); notify?.(state.error); }
      if (state.busy) timer = timers.setTimeout(load, 700);
    } catch (error) { if (!destroyed && version === generation && request === loadVersion) { core.set({ error: error.message, loading: false }); notify?.(error.message); } }
  }
  async function act(operation, payload = {}) {
    const version = generation, account = core.state.account;
    loadVersion++; cancelTimer(); core.set({ loading: true });
    try {
      await service.action(operation, { ...payload, account });
      if (!destroyed && version === generation) { core.set({ error: '' }); await load(); if (operation === 'draft' || operation === 'read') refresh?.(); return true; }
    } catch (error) { if (!destroyed && version === generation) { core.set({ error: error.message, loading: false }); notify?.(error.message); } return false; }
  }
  return Object.assign(core, {
    toggle() { core.set({ open: !core.state.open }); if (core.state.open) load(); },
    scope(account) { if (account === core.state.account) return; generation++; cancelTimer(); core.set({ account, messages: [], trace: [], drafts: [], busy: false, error: '', stream: '', notifiedError: '', loading: true }); if (core.state.open) load(); },
    tab(tab) { core.set({ tab }); },
    send(prompt) { if (!prompt.trim() || core.state.busy || core.state.loading || !core.state.configured) return; core.set({ error: '', notifiedError: '' }); return act('chat', { prompt, context: context() }); },
    stop() { return act('stop'); }, clear() { return act('clear'); },
    saveDraft(id, subject, body) { return act('draft', { id, subject, body }); },
    confirmRead(id) { return act('read', { id }); }, discard(id) { return act('discard', { id }); },
    reload: load,
    destroy() { destroyed = true; generation++; cancelTimer(); destroyCore(); },
  });
}
