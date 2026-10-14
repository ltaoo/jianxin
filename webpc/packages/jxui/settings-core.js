import { ControlCore } from './core.js';

export function SettingsCore({ service }) {
  const core = ControlCore({ label: '配置', open: false, loaded: false, busy: false, saving: false,
    showUnreadCount: true, error: '', status: '' });
  const destroy = core.destroy.bind(core);
  let request, destroyed = false;
  return Object.assign(core, {
    click() { return this.open(); },
    open() { core.set({ open: true }); return this.load(); },
    close() { if (!core.state.saving) { request?.abort(); core.set({ open: false }); } },
    async load() {
      if (destroyed || core.state.saving) return;
      request?.abort(); const current = request = new AbortController();
      core.set({ busy: true, loaded: false, error: '', status: '' });
      try {
        const result = await service.configSettings(current.signal);
        if (!destroyed && !current.signal.aborted) core.set({ showUnreadCount: result.tray_show_unread_count, loaded: true });
      } catch (error) {
        if (!destroyed && !current.signal.aborted) core.set({ error: `配置读取失败：${error.message}` });
      } finally { if (!destroyed && request === current) core.set({ busy: false }); }
    },
    async setUnreadCount(show) {
      if (destroyed || !core.state.loaded || core.state.busy || core.state.saving || typeof show !== 'boolean' || show === core.state.showUnreadCount) return;
      const previous = core.state.showUnreadCount;
      core.set({ showUnreadCount: show, saving: true, error: '', status: '' });
      try {
        const result = await service.saveConfigSettings({ tray_show_unread_count: show });
        if (!destroyed) core.set({ showUnreadCount: result.tray_show_unread_count, status: '已保存，托盘将在 5 秒内更新' });
      } catch (error) {
        if (!destroyed) core.set({ showUnreadCount: previous, error: `配置保存失败：${error.message}` });
      } finally { if (!destroyed) core.set({ saving: false }); }
    },
    destroy() { destroyed = true; request?.abort(); destroy(); },
  });
}
