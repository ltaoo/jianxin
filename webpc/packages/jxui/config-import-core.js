import { ControlCore } from './core.js';
import { CONFIG_LIMIT, decryptConfig } from './config-crypto.js';

export function ConfigImportCore({ service, imported = () => {}, decrypt = decryptConfig }) {
  const initial = () => ({ open: false, mode: 'file', url: '', password: '', cipher: 'aes-256-gcm', filename: '',
    busy: false, saving: false, preview: [], selected: [], warnings: [], error: '', status: '' });
  const core = ControlCore(initial());
  const destroy = core.destroy.bind(core);
  let file, config, generation = 0, request, destroyed = false;
  function clear() { file = undefined; config = undefined; request?.abort(); ++generation; }
  return Object.assign(core, {
    open() { if (destroyed || core.state.open) return; clear(); core.set({ ...initial(), open: true }); },
    close() { if (core.state.saving) return; clear(); core.set(initial()); },
    edit(field, value) {
      if (destroyed || core.state.busy || core.state.saving || !['mode', 'url', 'password', 'cipher'].includes(field)) return;
      config = undefined;
      core.set({ [field]: value, preview: [], selected: [], warnings: [], error: '', status: '' });
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
      if (!password) { core.set({ error: '请输入解密密钥' }); return; }
      if (mode === 'file' && !file) { core.set({ error: '请选择密文文件' }); return; }
      if (mode === 'url') {
        try {
          const address = new URL(url);
          if (address.protocol !== 'https:' || address.username || address.password || address.hash) throw new Error();
        } catch { core.set({ error: '请输入完整 HTTPS URL，不要包含用户名、密码或片段' }); return; }
      }
      const current = ++generation;
      request = new AbortController(); config = undefined;
      core.set({ busy: true, preview: [], selected: [], warnings: [], error: '', status: mode === 'url' ? '正在下载并解密…' : '正在读取并解密…' });
      let bytes;
      try {
        if (mode === 'file') {
          if (file.size > CONFIG_LIMIT) throw new Error('加密配置不能超过 2 MB');
          bytes = new Uint8Array(await file.arrayBuffer());
        } else {
          const result = await service.configSource(url.trim(), request.signal);
          bytes = new TextEncoder().encode(result.ciphertext);
        }
        if (destroyed || current !== generation) return;
        const decoded = await decrypt(bytes, password, cipher);
        if (destroyed || current !== generation) return;
        core.set({ password: '' });
        const preview = await service.configPreview(decoded, request.signal);
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
        core.set({ password: '', filename: '', preview: [], selected: [], warnings: [],
          status: `已导入 ${result.imported} 个邮箱。${result.daemon === 'failed' ? '后台收信重启失败，可稍后重试。' : '邮箱列表已更新。'}` });
        imported(result);
      } catch (error) { if (!destroyed && current === generation) core.set({ error: error.message, status: '' }); }
      finally { if (!destroyed && current === generation) core.set({ saving: false }); }
    },
    destroy() { destroyed = true; clear(); core.set(initial()); destroy(); },
  });
}
