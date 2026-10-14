export function Settings({ store }) {
  const open = ref(store.state.open);
  const off = store.onStateChange(s => open.as(s.open));
  return View({ attributes: { n: 'settings-host' }, onUnmounted() { off(); open.destroy(); } }, [
    Show({ when: open, ok: () => SettingsDialog({ store }) }),
  ]);
}

function SettingsDialog({ store }) {
  const state = ref(store.state), derived = [];
  const off = store.onStateChange(s => state.as(s));
  const pick = fn => { const value = Timeless.computed(state, fn); derived.push(value); return value; };
  return View({ as: 'dialog', class: 'jx-import-dialog',
      attributes: { n: 'settings-dialog', 'aria-labelledby': 'settings-title' },
      onMounted({ target: host }) {
        const dialog = host.get$elm?.() || host, previous = document.activeElement;
        const cancel = event => { event.preventDefault(); store.close(); };
        dialog.addEventListener('cancel', cancel); dialog.showModal();
        return () => { dialog.removeEventListener('cancel', cancel); previous?.focus?.(); };
      }, onUnmounted() { off(); derived.forEach(value => value.destroy()); state.destroy(); } }, [
      View({ class: 'jx-import-heading jx-row', attributes: { n: 'settings-heading' } }, [
        View({ as: 'h2', attributes: { n: 'settings-title', id: 'settings-title' } }, ['配置']),
        View({ class: 'jx-spacer', attributes: { n: 'settings-heading-spacer' } }),
        View({ as: 'button', class: 'jx-button jx-button--ghost', attributes: { n: 'settings-close', type: 'button', disabled: pick(s => s.saving ? '' : undefined) }, onClick() { store.close(); } }, ['关闭']),
      ]),
      View({ as: 'label', class: 'jx-row', attributes: { n: 'tray-unread-count-field' } }, [
        View({ as: 'input', attributes: { n: 'tray-unread-count-toggle', type: 'checkbox', disabled: pick(s => !s.loaded || s.busy || s.saving ? '' : undefined) },
          onMounted({ target: host }) {
            const target = host.get$elm?.() || host;
            const apply = () => { target.checked = store.state.showUnreadCount; };
            const change = () => store.setUnreadCount(target.checked);
            apply(); const stop = store.onStateChange(apply); target.addEventListener('change', change);
            return () => { stop(); target.removeEventListener('change', change); };
          } }),
        View({ as: 'span', attributes: { n: 'tray-unread-count-label' } }, ['托盘显示未读数']),
      ]),
      View({ as: 'p', class: 'jx-muted', attributes: { n: 'tray-unread-count-help' } }, ['控制 macOS 菜单栏托盘图标旁的未读数字。']),
      Show({ when: pick(s => s.busy || s.saving || s.status), ok: () => View({ as: 'p', class: 'jx-muted', attributes: { n: 'settings-status', role: 'status' } }, [pick(s => s.busy ? '正在读取配置…' : s.saving ? '正在保存…' : s.status)]) }),
      Show({ when: pick(s => s.error), ok: () => View({ as: 'p', attributes: { n: 'settings-error', role: 'alert' } }, [pick(s => s.error)]) }),
      Show({ when: pick(s => !s.loaded && !s.busy && s.error), ok: () => View({ as: 'button', class: 'jx-button', attributes: { n: 'settings-retry', type: 'button' }, onClick() { store.load(); } }, ['重新读取']) }),
    ]);
}
