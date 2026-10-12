import { ControlCore } from '../../../packages/jxui/core.js';
import { MailService } from '../../services.js';

export function SyncLogsModel({ root, location, history, service = MailService(root), timers = globalThis }) {
  const url = new URL(location.href);
  const core = ControlCore({ accounts: [], logs: [], account: url.searchParams.get('account') || '',
    page: 0, more: false, loading: false, loaded: false, error: '', retention: 1000 });
  const destroy = core.destroy.bind(core);
  let request, generation = 0, destroyed = false, timer;
  const ui = {
    refresh: ControlCore({ label: '刷新日志' }, { click() { return core.load(); } }),
    prev: ControlCore({ label: '上一页', disabled: true }, { click() { return core.paginate(-1); } }),
    next: ControlCore({ label: '下一页', disabled: true }, { click() { return core.paginate(1); } }),
  };
  const off = core.onStateChange(s => {
    ui.refresh.set({ disabled: s.loading });
    ui.prev.set({ disabled: s.loading || s.page === 0 });
    ui.next.set({ disabled: s.loading || !s.more });
  });
  return Object.assign(core, { ui,
    async load() {
      if (destroyed) return;
      request?.abort(); request = new AbortController();
      const current = ++generation;
      core.set({ loading: true, error: '' });
      try {
        const result = await service.syncLogs({ account: core.state.account, page: core.state.page }, request.signal);
        if (!destroyed && current === generation) core.set({ accounts: result.accounts,
          logs: result.logs.map(entry => ({ ...entry, email: result.accounts.find(account => account.id === entry.account_id)?.email || '未知邮箱' })),
          more: result.more, retention: result.retention, loaded: true });
      } catch (error) {
        if (!destroyed && current === generation && error.name !== 'AbortError') core.set({ error: '同步日志加载失败，请刷新重试。' });
      } finally { if (!destroyed && current === generation) core.set({ loading: false }); }
    },
    select(account) {
      if (destroyed || account === core.state.account) return;
      core.set({ account, page: 0, logs: [], more: false, loaded: false });
      history?.replaceState({}, '', `${root}/logs${account ? '?' + new URLSearchParams({ account }) : ''}`);
      return core.load();
    },
    paginate(delta) {
      if (core.state.loading || ![-1, 1].includes(delta) || delta < 0 && !core.state.page || delta > 0 && !core.state.more) return;
      core.set({ page: core.state.page + delta, logs: [], loaded: false });
      return core.load();
    },
    start() {
      if (destroyed || timer !== undefined) return;
      timer = timers.setInterval(() => { if (!core.state.loading && core.state.page === 0) core.load(); }, 5000);
      return core.load();
    },
    destroy() { destroyed = true; ++generation; request?.abort(); timers.clearInterval(timer); off(); Object.values(ui).forEach(item => item.destroy()); destroy(); },
  });
}
