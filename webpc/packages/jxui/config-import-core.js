import { ControlCore } from './core.js';
import { CONFIG_LIMIT, decryptConfig } from './config-crypto.js';
import { mailboxDraft, mailboxConfig } from './mailbox-config-core.js';

export function ConfigImportCore({ service, imported = () => {}, decrypt = decryptConfig }) {
  const initial = () => ({ open: false, mode: 'file', url: '', password: '', cipher: 'aes-256-gcm', filename: '',
    busy: false, saving: false, preview: [], selected: [], warnings: [], error: '', status: '', providers: [], draft: mailboxDraft() });
  const core = ControlCore(initial());
  const destroy = core.destroy.bind(core);
  let file, config, generation = 0, request, destroyed = false;
  function clear() { file = undefined; config = undefined; request?.abort(); ++generation; }
  return Object.assign(core, {
    open() { if (destroyed || core.state.open) return; clear(); core.set({ ...initial(), open: true }); },
    close() { if (core.state.saving) return; clear(); core.set(initial()); },
    edit(field, value) {
      if (destroyed || core.state.busy || core.state.saving || !['mode', 'url', 'password', 'cipher'].includes(field)) return;
      if (field === 'mode') {
        if (!['file', 'url', 'manual'].includes(value)) return;
        if (value === core.state.mode) return;
        clear();
        core.set({ draft: mailboxDraft(), password: '', filename: '' });
      }
      config = undefined;
      core.set({ [field]: value, preview: [], selected: [], warnings: [], error: '', status: '' });
      if (field === 'mode' && value === 'manual' && !core.state.providers.length) return this.loadProviders();
    },
    async loadProviders() {
      if (destroyed || core.state.busy || core.state.saving) return;
      const current = ++generation;
      request = new AbortController(); core.set({ busy: true, error: '', status: '正在加载邮箱服务商…' });
      try {
        const result = await service.configPresets(request.signal);
        if (!destroyed && current === generation) core.set({ providers: result.providers, status: '' });
      } catch (error) {
        if (!destroyed && current === generation && error.name !== 'AbortError') core.set({ error: '服务商加载失败，请重试', status: '' });
      } finally { if (!destroyed && current === generation) core.set({ busy: false }); }
    },
    chooseProvider(key) {
      if (destroyed || core.state.busy || core.state.saving) return;
      const preset = core.state.providers.find(provider => provider.key === key);
      if (!preset) return;
      const { email, display_name } = core.state.draft;
      config = undefined;
      core.set({ draft: { ...mailboxDraft(preset), email, display_name }, preview: [], selected: [], warnings: [], error: '', status: '' });
    },
    editMailbox(field, value) {
      if (destroyed || core.state.busy || core.state.saving || !Object.hasOwn(core.state.draft, field) || ['preset', 'provider', 'platform'].includes(field)) return;
      const draft = { ...core.state.draft, [field]: value };
      if (field === 'advanced') { core.set({ draft }); return; }
      config = undefined;
      if (field === 'oauth') draft.password = '';
      if (field === 'imap_tls') draft.imap_port = value === 'start_tls' ? '143' : '993';
      if (field === 'smtp_tls') draft.smtp_port = value === 'start_tls' ? '587' : '465';
      core.set({ draft, preview: [], selected: [], warnings: [], error: '', status: '' });
    },
    chooseFile(next) {
      if (destroyed || core.state.busy || core.state.saving) return;
      file = next; config = undefined;
      core.set({ filename: next?.name || '', preview: [], selected: [], warnings: [], error: '', status: '' });
    },
    select(index, selected) {
      if (core.state.saving) return;
      const next = new Set(core.state.selected);
      selected ? next.add(index) : next.delete(index);
      core.set({ selected: [...next] });
    },
    async read() {
      if (destroyed || core.state.busy || core.state.saving) return;
      const { mode, password, url, cipher } = core.state;
      let manual;
      if (mode === 'manual') {
        try {
          if (!core.state.draft.preset) throw new Error('请选择邮箱服务商');
          manual = mailboxConfig(core.state.draft);
        } catch (error) { core.set({ error: error.message }); return; }
      }
      if (mode !== 'manual' && !password) { core.set({ error: '请输入解密密钥' }); return; }
      if (mode === 'file' && !file) { core.set({ error: '请选择密文文件' }); return; }
      if (mode === 'url') {
        try {
          const address = new URL(url);
          if (address.protocol !== 'https:' || address.username || address.password || address.hash) throw new Error();
        } catch { core.set({ error: '请输入完整 HTTPS URL，不要包含用户名、密码或片段' }); return; }
      }
      const current = ++generation;
      request = new AbortController(); config = undefined;
      core.set({ busy: true, preview: [], selected: [], warnings: [], error: '', status: mode === 'manual' ? '正在验证邮箱连接…' : mode === 'url' ? '正在下载并解密…' : '正在读取并解密…' });
      let bytes;
      try {
        if (mode === 'file') {
          if (file.size > CONFIG_LIMIT) throw new Error('加密配置不能超过 2 MB');
          bytes = new Uint8Array(await file.arrayBuffer());
        } else if (mode === 'url') {
          const result = await service.configSource(url.trim(), request.signal);
          bytes = new TextEncoder().encode(result.ciphertext);
        }
        if (destroyed || current !== generation) return;
        const decoded = manual || await decrypt(bytes, password, cipher);
        if (destroyed || current !== generation) return;
        core.set({ password: '' });
        const preview = await service.configPreview({ ...decoded, verify_connection: mode === 'manual' }, request.signal);
        if (destroyed || current !== generation) return;
        config = decoded;
        core.set({ preview: preview.accounts, selected: preview.accounts.map(account => account.index), warnings: preview.warnings, status: '' });
      } catch (error) {
        if (!destroyed && current === generation && error.name !== 'AbortError') core.set({ error: error.message, status: '' });
      } finally {
        bytes?.fill(0);
        if (!destroyed && current === generation) core.set({ busy: false });
      }
    },
    async save() {
      if (destroyed || core.state.busy || core.state.saving || !config || !core.state.selected.length) return;
      const current = generation;
      core.set({ saving: true, error: '', status: '正在保存邮箱…' });
      try {
        const result = await service.configImport({ accounts: core.state.selected.map(index => config.accounts[index]) });
        if (destroyed || current !== generation) return;
        config = undefined; file = undefined;
        core.set({ password: '', draft: mailboxDraft(), filename: '', preview: [], selected: [], warnings: [],
          status: `已保存 ${result.imported} 个邮箱。${result.daemon === 'failed' ? '后台收信重启失败，可稍后重试。' : '邮箱列表已更新。'}` });
        imported(result);
      } catch (error) { if (!destroyed && current === generation) core.set({ error: error.message, status: '' }); }
      finally { if (!destroyed && current === generation) core.set({ saving: false }); }
    },
    destroy() { destroyed = true; clear(); core.set(initial()); destroy(); },
  });
}
