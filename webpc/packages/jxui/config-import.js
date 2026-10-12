import { Button, Glyph } from './index.js';
import { ControlCore } from './core.js';

export function ConfigImport({ store, root = '' }) {
  const open = ref(store.state.open);
  const off = store.onStateChange(s => open.as(s.open));
  return View({ attributes: { n: 'config-import-host' }, onUnmounted() { off(); open.destroy(); } },
    [Show({ when: open, ok: () => ImportDialog({ store, root }) })]);
}

function ImportDialog({ store, root }) {
  const state = ref(store.state), derived = [], controls = [];
  const off = store.onStateChange(s => state.as(s));
  const pick = fn => { const value = Timeless.computed(state, fn); derived.push(value); return value; };
  const disabled = pick(s => s.busy || s.saving ? '' : undefined);
  function button(label, action, variant = 'ghost', locked = s => s.busy || s.saving) {
    const core = ControlCore({ label, disabled: locked(store.state) }, { click: action });
    const stop = store.onStateChange(s => core.set({ disabled: locked(s) }));
    controls.push(() => { stop(); core.destroy(); });
    return Button({ store: core, variant, attributes: { n: `mailbox-action-${label}` } });
  }
  function field(name, label, type, placeholder = '', manual = false) {
    const id = manual ? `mailbox-${name}` : `config-import-${name}`;
    return View({ as: 'label', class: 'jx-import-field', attributes: { n: `${id}-field` } }, [View({ as: 'span', attributes: { n: `${id}-label` } }, [label]),
      View({ as: 'input', class: 'jx-input', attributes: { n: id, type, placeholder, 'aria-label': label,
        autocomplete: manual ? (name === 'email' || name === 'username' ? 'username' : name === 'password' ? pick(s => s.draft.oauth ? 'off' : 'current-password') : 'off') : 'off', spellcheck: 'false', disabled, maxlength: type === 'password' ? '1024' : '4096' },
        onMounted({ target: host }) {
          const target = host.get$elm?.() || host;
          const apply = () => { const value = manual ? store.state.draft[name] : store.state[name]; if (target.value !== value) target.value = value; };
          const input = e => store[manual ? 'editMailbox' : 'edit'](name, e.target.value);
          apply(); const stop = store.onStateChange(apply); target.addEventListener('input', input);
          return () => { stop(); target.removeEventListener('input', input); };
        } }),
    ]);
  }
  function choice(name, label, options) {
    return View({ as: 'label', class: 'jx-import-field', attributes: { n: `mailbox-${name}-field` } }, [
      View({ as: 'span', attributes: { n: `mailbox-${name}-label` } }, [label]),
      View({ as: 'select', class: 'jx-input', attributes: { n: `mailbox-${name}`, 'aria-label': label, disabled }, onMounted({ target: host }) {
        const target = host.get$elm?.() || host;
        const apply = () => { target.value = String(store.state.draft[name]); };
        const change = () => store.editMailbox(name, name === 'oauth' ? target.value === 'true' : target.value);
        apply(); const stop = store.onStateChange(apply); target.addEventListener('change', change);
        return () => { stop(); target.removeEventListener('change', change); };
      } }, options.map(([value, title]) => View({ as: 'option', attributes: { n: `mailbox-${name}-${value}`, value } }, [title]))),
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
    View({ class: 'jx-import-heading jx-row' }, [Glyph('mail'), View({ as: 'h2', attributes: { n: 'add-mailbox-title', id: 'config-import-title' } }, ['添加邮箱']),
      View({ class: 'jx-spacer' }), button('关闭', () => store.close(), 'ghost', s => s.saving)]),
    View({ as: 'p', class: 'jx-muted', attributes: { n: 'add-mailbox-description' } }, ['导入已有的加密配置，或选择服务商手动添加邮箱。']),
    View({ class: 'jx-import-source jx-row', attributes: { n: 'add-mailbox-methods', role: 'group', 'aria-label': '添加邮箱方式' } },
      [['file', '密文文件'], ['url', 'HTTPS URL'], ['manual', '手动添加']].map(([mode, label]) => View({ as: 'button', class: 'jx-button', attributes: { n: `add-mailbox-method-${mode}`, 'aria-label': label, type: 'button', disabled, 'aria-pressed': pick(s => String(s.mode === mode)) }, onClick() { store.edit('mode', mode); } }, [label]))),
    Show({ when: pick(s => s.mode === 'manual'), ok: () => View({ attributes: { n: 'manual-mailbox-form' } }, [
      View({ as: 'h3', attributes: { n: 'mailbox-provider-heading' } }, ['选择邮箱服务商']),
      View({ class: 'jx-import-providers', attributes: { n: 'mailbox-provider-options', role: 'group', 'aria-label': '邮箱服务商' } }, [
        For({ each: pick(s => s.providers), render: provider => View({ as: 'button', class: 'jx-button jx-import-provider', attributes: { n: `mailbox-provider-${provider.key}`, 'aria-label': `选择 ${provider.name}`, type: 'button', disabled, 'aria-pressed': pick(s => String(s.draft.preset === provider.key)) }, onClick() { store.chooseProvider(provider.key); } }, [
          provider.platform ? View({ as: 'img', class: 'jx-mailbox-favicon', attributes: { n: `mailbox-provider-${provider.key}-logo`, src: `${root}/assets/mail-providers/${provider.platform}.svg`, alt: '', 'aria-hidden': 'true' } }) : Glyph('mail'),
          View({ as: 'span', attributes: { n: `mailbox-provider-${provider.key}-name` } }, [provider.name]),
        ]) }),
      ]),
      Show({ when: pick(s => !s.busy && !s.providers.length), ok: () => button('重新加载服务商', () => store.loadProviders()) }),
      Show({ when: pick(s => !!s.draft.preset), ok: () => View({ attributes: { n: 'mailbox-account-fields' } }, [
        field('email', '邮箱地址', 'email', 'you@example.com', true),
        field('display_name', '显示名称（可选）', 'text', '', true),
        Show({ when: pick(s => !s.draft.oauth), ok: () => field('password', '密码 / 客户端授权码', 'password', '', true) }),
        Show({ when: pick(s => s.draft.oauth), ok: () => View({ as: 'p', class: 'jx-muted', attributes: { n: 'mailbox-oauth-help' } }, ['可以先保存邮箱，再点击邮箱状态图标完成 OAuth 授权登录。']) }),
        View({ as: 'p', class: 'jx-muted', attributes: { n: 'mailbox-credential-help' } }, ['QQ、163 等邮箱请先开启 IMAP/SMTP，并使用客户端授权码。']),
        View({ as: 'button', class: 'jx-button', attributes: { n: 'mailbox-advanced-toggle', 'aria-label': pick(s => s.draft.advanced ? '收起高级设置' : '高级设置'), type: 'button', disabled, 'aria-expanded': pick(s => String(s.draft.advanced)), 'aria-controls': 'mailbox-advanced-settings' }, onClick() { store.editMailbox('advanced', !store.state.draft.advanced); } }, [pick(s => s.draft.advanced ? '收起高级设置' : '高级设置')]),
        Show({ when: pick(s => s.draft.advanced), ok: () => View({ attributes: { n: 'mailbox-advanced-settings', id: 'mailbox-advanced-settings' } }, [
          field('username', '登录用户名（留空使用邮箱地址）', 'text', '', true),
          choice('oauth', '登录方式', [['false', '密码 / 客户端授权码'], ['true', 'OAuth 授权登录']]),
          Show({ when: pick(s => s.draft.oauth), ok: () => View({ attributes: { n: 'mailbox-oauth-settings' } }, [
            field('password', '已有访问令牌（可选）', 'password', '', true),
            field('client_id', 'OAuth Client ID（应用 ID，可稍后配置）', 'text', '', true),
            field('token_endpoint', 'OAuth 令牌端点', 'url', '', true),
          ]) }),
          ...['imap', 'smtp'].map(protocol => View({ attributes: { n: `mailbox-${protocol}-settings` } }, [
            View({ as: 'h3', attributes: { n: `mailbox-${protocol}-heading` } }, [protocol === 'imap' ? '收信 · IMAP' : '发信 · SMTP']),
            field(`${protocol}_host`, '服务器地址', 'text', '', true),
            field(`${protocol}_port`, '端口', 'number', '', true),
            field(`${protocol}_username`, '专用用户名（可选）', 'text', '', true),
            choice(`${protocol}_tls`, '安全连接', [['tls', '隐式 TLS'], ['start_tls', 'STARTTLS']]),
          ])),
        ]) }),
        View({ as: 'p', class: 'jx-muted', attributes: { n: 'mailbox-preview-help' } }, ['验证收信连接并预览配置，确认后保存。待授权邮箱可以先保存再登录。']),
        button('验证并预览', () => store.read(), 'primary'),
      ]) }),
    ]) }),
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
    Show({ when: pick(s => s.mode !== 'manual'), ok: () => View({ attributes: { n: 'encrypted-mailbox-import' } }, [
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
    ]) }),
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
          account.needs_authorization ? View({ as: 'span', class: 'jx-muted' }, ['待授权 · 保存后手动完成 OAuth 登录']) : undefined,
          account.connection_verified ? View({ as: 'span', class: 'jx-muted', attributes: { n: 'mailbox-connection-verified' } }, ['收信连接验证通过']) : undefined]),
        View({ as: 'span', class: 'jx-badge' }, [account.existing ? '更新' : '添加']),
      ]) }),
      For({ each: pick(s => s.warnings), render: warning => View({ as: 'p', class: 'jx-muted' }, [warning]) }),
      button('保存所选邮箱', () => store.save(), 'primary', s => s.busy || s.saving || !s.selected.length),
    ]) }),
    Show({ when: pick(s => s.error), ok: () => View({ as: 'p', class: 'jx-import-error', attributes: { role: 'alert' } }, [pick(s => s.error)]) }),
    View({ as: 'p', class: 'jx-muted', attributes: { role: 'status', 'aria-live': 'polite' } }, [pick(s => s.status)]),
  ]);
}
