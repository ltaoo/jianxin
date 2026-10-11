import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import { webcrypto } from 'node:crypto';
import { CONFIG_LIMIT, decryptConfig } from '../packages/jxui/config-crypto.js';
import { ConfigImportCore } from '../packages/jxui/config-import-core.js';
import { MailService } from '../src/services.js';
import { ConfigImport } from '../packages/jxui/config-import.js';

for (const file of ['timeless.lite.umd.min.js', 'timeless.dom.umd.min.js']) vm.runInThisContext(fs.readFileSync(new URL(`../vendor/${file}`, import.meta.url), 'utf8'));
Object.assign(globalThis, Timeless);
const find = (node, name) => node?.state?.attributes?.n === name ? node : node?.children?.map(child => find(child, name)).find(Boolean);

const fixture = JSON.parse(fs.readFileSync(new URL('../../mobile/tests/fixtures/config.json', import.meta.url), 'utf8'));
const raw = method => new TextEncoder().encode(method === 'legacy-json' ? JSON.stringify(fixture.envelope) : fixture[method]);
for (const method of ['aes-256-gcm', 'aes-128-gcm', 'legacy-json']) {
  test(`Web Crypto reads the Android publishing fixture: ${method}, Chinese and emoji password`, async () => {
    assert.deepEqual(await decryptConfig(raw(method), fixture.password, method, webcrypto), JSON.parse(fixture.plaintext));
    await assert.rejects(decryptConfig(raw(method), 'wrong-key', method, webcrypto), /解密失败/);
  });
}
test('wrong cipher, tampering, incomplete Base64 and oversized input cannot be imported', async () => {
  await assert.rejects(decryptConfig(raw('aes-128-gcm'), fixture.password, 'aes-256-gcm', webcrypto), /解密失败/);
  const tampered = raw('aes-256-gcm'); tampered[40] = tampered[40] === 65 ? 66 : 65;
  await assert.rejects(decryptConfig(tampered, fixture.password, 'aes-256-gcm', webcrypto), /解密失败/);
  await assert.rejects(decryptConfig(new TextEncoder().encode('broken'), fixture.password), /密文格式/);
  await assert.rejects(decryptConfig(new Uint8Array(CONFIG_LIMIT + 1), fixture.password), /2 MB/);
});

function fakeService() {
  const imports = [], urls = [];
  return { imports, urls,
    async configSource(url) { urls.push(url); return { ciphertext: fixture['aes-256-gcm'] }; },
    async configPreview(config) { return { accounts: config.accounts.map((account, index) => ({ index, email: account.email, existing: false })), warnings: [] }; },
    async configImport(config) { imports.push(config); return { imported: config.accounts.length, daemon: 'not_running' }; },
  };
}
const decrypt = (data, key, method) => decryptConfig(data, key, method, webcrypto);
test('local files preview before saving, support selecting accounts, and clear the key', async () => {
  const service = fakeService(), core = ConfigImportCore({ service, decrypt });
  core.open(); core.edit('password', fixture.password);
  core.chooseFile({ name: 'config.enc.txt', size: raw('aes-256-gcm').length, async arrayBuffer() { return raw('aes-256-gcm').buffer; } });
  await core.read();
  assert.equal(service.imports.length, 0); assert.equal(core.state.preview.length, 2); assert.equal(core.state.password, '');
  assert(!JSON.stringify(core.state).includes('sample-password'));
  core.select(0, false); await core.save();
  assert.equal(service.imports.length, 1); assert.equal(service.imports[0].accounts.length, 1);
  assert.equal(service.imports[0].accounts[0].email, 'sample@example.com');
  assert.equal(core.state.preview.length, 0); core.close(); assert.equal(core.state.password, ''); core.destroy();
});
test('HTTPS source uses the same decrypt flow and never sends the key to the download service', async () => {
  const service = fakeService(), core = ConfigImportCore({ service, decrypt });
  core.open(); core.edit('mode', 'url'); core.edit('url', 'https://example.com/config.enc?v=2'); core.edit('password', fixture.password);
  await core.read(); assert.equal(core.state.preview.length, 2); assert.deepEqual(service.urls, ['https://example.com/config.enc?v=2']);
  core.edit('url', 'http://example.com/config.enc'); await core.read(); assert.equal(service.urls.length, 1); assert.match(core.state.error, /密钥/);
  core.edit('password', fixture.password); await core.read(); assert.match(core.state.error, /HTTPS/); core.destroy();
});
test('wrong keys never save and late decrypt results are discarded after closing', async () => {
  const service = fakeService(), core = ConfigImportCore({ service, decrypt });
  core.open(); core.edit('mode', 'url'); core.edit('url', 'https://example.com/a'); core.edit('password', 'wrong-key');
  await core.read(); await core.save(); assert.match(core.state.error, /解密失败/); assert.equal(service.imports.length, 0);
  core.edit('password', fixture.password); const pending = core.read(); core.close(); await pending;
  assert.equal(core.state.open, false); assert.equal(core.state.preview.length, 0); assert.equal(core.state.password, ''); core.destroy();
});
test('HTTP import operations use same-origin POST and the existing request marker', async () => {
  const calls = [];
  const service = MailService('', async (url, options) => { calls.push({ url, ...options }); return { ok: true, json: async () => ({}) }; });
  await service.configSource('https://example.com/a'); await service.configPreview({ accounts: [] }); await service.configImport({ accounts: [] });
  for (const call of calls) { assert.equal(call.method, 'POST'); assert.equal(call.headers['X-Jianxin-Request'], '1'); assert.equal(call.credentials, 'same-origin'); }
  assert.deepEqual(JSON.parse(calls[0].body), { url: 'https://example.com/a' });
});
test('real Timeless import view uses native file/password controls and clears mounted key fields', () => {
  const core = ConfigImportCore({ service: fakeService() }); core.open(); core.edit('password', 'temporary-key');
  const view = ConfigImport({ store: core }), dialog = find(view, 'config-import-dialog');
  assert.equal(dialog.state.as, 'dialog');
  assert.equal(find(view, 'config-import-file').state.as, 'input');
  const password = find(view, 'config-import-password');
  assert.equal(password.state.attributes.type, 'password'); assert.equal(password.state.attributes.disabled, undefined);
  const listeners = new Map();
  const target = { value: '', addEventListener: (name, listener) => listeners.set(name, listener), removeEventListener: name => listeners.delete(name) };
  password.onMounted({ target: { get$elm: () => target } });
  assert.equal(target.value, 'temporary-key'); core.set({ password: '' }); assert.equal(target.value, '');
  password.onUnmounted(); assert.equal(listeners.size, 0);
  dialog.onUnmounted(); view.onUnmounted(); core.close(); core.destroy();
});
