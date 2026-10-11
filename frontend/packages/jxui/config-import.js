import { Button, Glyph } from './index.js';
import { ControlCore } from './core.js';

export function ConfigImport({ store }) {
  const open = ref(store.state.open);
  const off = store.onStateChange(s => open.as(s.open));
  return View({ attributes: { n: 'config-import-host' }, onUnmounted() { off(); open.destroy(); } },
    [Show({ when: open, ok: () => ImportDialog({ store }) })]);
}

function ImportDialog({ store }) {
  const state = ref(store.state), derived = [], controls = [];
  const off = store.onStateChange(s => state.as(s));
  const pick = fn => { const value = Timeless.computed(state, fn); derived.push(value); return value; };
  const disabled = pick(s => s.busy || s.saving ? '' : undefined);
  function button(label, action, variant = 'ghost', locked = s => s.busy || s.saving) {
    const core = ControlCore({ label, disabled: locked(store.state) }, { click: action });
    const stop = store.onStateChange(s => core.set({ disabled: locked(s) }));
    controls.push(() => { stop(); core.destroy(); });
    return Button({ store: core, variant });
  }
  function field(name, label, type, placeholder = '') {
    return View({ as: 'label', class: 'jx-import-field' }, [View({ as: 'span' }, [label]),
      View({ as: 'input', class: 'jx-input', attributes: { n: `config-import-${name}`, type, placeholder,
        autocomplete: 'off', spellcheck: 'false', disabled, maxlength: type === 'password' ? '1024' : '4096' },
        onMounted({ target: host }) {
          const target = host.get$elm?.() || host;
          const apply = () => { if (target.value !== store.state[name]) target.value = store.state[name]; };
          const input = e => store.edit(name, e.target.value);
          apply(); const stop = store.onStateChange(apply); target.addEventListener('input', input);
          return () => { stop(); target.removeEventListener('input', input); };
        } }),
    ]);
  }
  return View({ as: 'dialog', class: 'jx-import-dialog', attributes: { n: 'config-import-dialog', 'aria-labelledby': 'config-import-title' },
    onMounted({ target: host }) {
      const dialog = host.get$elm?.() || host;
      const previous = document.activeElement;
      const cancel = e => { e.preventDefault(); store.close(); };
      dialog.addEventListener('cancel', cancel); dialog.showModal();
      return () => { dialog.removeEventListener('cancel', cancel); previous?.focus?.(); };
    },
    onUnmounted() { off(); controls.forEach(cleanup => cleanup()); derived.forEach(value => value.destroy()); state.destroy(); },
  }, [
    View({ class: 'jx-import-heading jx-row' }, [Glyph('mail'), View({ as: 'h2', attributes: { id: 'config-import-title' } }, ['导入邮箱配置']),
      View({ class: 'jx-spacer' }), button('关闭', () => store.close(), 'ghost', s => s.saving)]),
    View({ as: 'p', class: 'jx-muted' }, ['选择密文来源并输入解密密钥，然后预览要保存的邮箱。']),
    View({ class: 'jx-import-source jx-row', attributes: { role: 'group', 'aria-label': '密文来源' } }, [
      View({ as: 'button', class: 'jx-button', attributes: { type: 'button', disabled, 'aria-pressed': pick(s => String(s.mode === 'file')) }, onClick() { store.edit('mode', 'file'); } }, ['密文文件']),
      View({ as: 'button', class: 'jx-button', attributes: { type: 'button', disabled, 'aria-pressed': pick(s => String(s.mode === 'url')) }, onClick() { store.edit('mode', 'url'); } }, ['HTTPS URL']),
    ]),
    Show({ when: pick(s => s.mode === 'file'), ok: () => View({ as: 'label', class: 'jx-import-field' }, [View({ as: 'span' }, ['密文文件']),
      View({ as: 'input', class: 'jx-import-file', attributes: { type: 'file', n: 'config-import-file', accept: '.txt,.enc,.json,text/plain,application/json', disabled },
        onMounted({ target: host }) {
          const target = host.get$elm?.() || host;
          const change = () => store.chooseFile(target.files?.[0]);
          target.addEventListener('change', change);
          const stop = store.onStateChange(s => { if (!s.filename) target.value = ''; });
          return () => { stop(); target.removeEventListener('change', change); };
        } }), View({ as: 'span', class: 'jx-muted' }, [pick(s => s.filename || '支持 .txt、.enc、.json，最大 2 MB')]),
    ]) }),
    Show({ when: pick(s => s.mode === 'url'), ok: () => field('url', '配置 URL', 'url', 'https://example.com/config.enc.txt') }),
    View({ as: 'label', class: 'jx-import-field' }, [View({ as: 'span' }, ['加密方式']),
      View({ as: 'select', class: 'jx-input', attributes: { n: 'config-import-cipher', disabled }, onMounted({ target: host }) {
        const target = host.get$elm?.() || host;
        target.value = store.state.cipher;
        const change = () => store.edit('cipher', target.value); target.addEventListener('change', change);
        return () => target.removeEventListener('change', change);
      } }, [['aes-256-gcm', 'AES-256-GCM'], ['aes-128-gcm', 'AES-128-GCM'], ['legacy-json', '旧版 JSON（兼容）']].map(([value, label]) =>
        View({ as: 'option', attributes: { value } }, [label]))),
    ]),
    field('password', '解密密钥', 'password'),
    View({ as: 'p', class: 'jx-muted' }, ['解密在当前浏览器完成，密钥不会保存。']),
    View({ class: 'jx-row' }, [button('读取并解密', () => store.read(), 'primary')]),
    Show({ when: pick(s => s.preview.length > 0), ok: () => View({ class: 'jx-import-preview' }, [
      View({ as: 'h3' }, ['选择要导入的邮箱']),
      View({ as: 'p', class: 'jx-muted' }, ['新邮箱会添加到列表；已有邮箱只更新所选账号，其余邮箱保留。']),
      For({ each: pick(s => s.preview), render: account => View({ as: 'label', class: 'jx-import-account' }, [
        View({ as: 'input', attributes: { type: 'checkbox', 'aria-label': `导入 ${account.email}`, disabled }, onMounted({ target: host }) {
          const target = host.get$elm?.() || host;
          const apply = () => { target.checked = store.state.selected.includes(account.index); };
          const change = () => store.select(account.index, target.checked);
          apply(); const stop = store.onStateChange(apply); target.addEventListener('change', change);
          return () => { stop(); target.removeEventListener('change', change); };
        } }), View({ class: 'jx-import-account-copy' }, [View({ as: 'strong' }, [account.email]),
          View({ as: 'span', class: 'jx-muted' }, [account.name || (account.existing ? '已有邮箱 · 将更新配置' : '新邮箱')]),
          account.needs_authorization && View({ as: 'span', class: 'jx-muted' }, ['待授权 · 保存后手动完成 OAuth 登录'])]),
        View({ as: 'span', class: 'jx-badge' }, [account.existing ? '更新' : '添加']),
      ]) }),
      For({ each: pick(s => s.warnings), render: warning => View({ as: 'p', class: 'jx-muted' }, [warning]) }),
      button('保存所选邮箱', () => store.save(), 'primary', s => s.busy || s.saving || !s.selected.length),
    ]) }),
    Show({ when: pick(s => s.error), ok: () => View({ as: 'p', class: 'jx-import-error', attributes: { role: 'alert' } }, [pick(s => s.error)]) }),
    View({ as: 'p', class: 'jx-muted', attributes: { role: 'status', 'aria-live': 'polite' } }, [pick(s => s.status)]),
  ]);
}
