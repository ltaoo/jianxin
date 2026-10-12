import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import { webcrypto } from 'node:crypto';
import { CONFIG_LIMIT, decryptConfig } from '../packages/jxui/config-crypto.js';
import { ConfigImportCore } from '../packages/jxui/config-import-core.js';
import { MailService } from '../src/services.js';
import { ConfigImport } from '../packages/jxui/config-import.js';
import { mailboxDraft, mailboxConfig } from '../packages/jxui/mailbox-config-core.js';

for (const file of ['timeless.lite.umd.min.js', 'timeless.dom.umd.min.js']) vm.runInThisContext(fs.readFileSync(new URL(`../vendor/${file}`, import.meta.url), 'utf8'));
Object.assign(globalThis, Timeless);
const find = (node, name) => node?.state?.attributes?.n === name ? node : node?.children?.map(child => find(child, name)).find(Boolean);

const fixture = JSON.parse(fs.readFileSync(new URL('../../tests/mobile/fixtures/config.json', import.meta.url), 'utf8'));
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
    async configPresets() { return { providers: [
      { key: 'qq', name: 'QQ', platform: 'qq', provider: 'generic-imap', imap: { host: 'imap.qq.com', port: 993, tls: 'tls' }, smtp: { host: 'smtp.qq.com', port: 465, tls: 'tls' } },
      { key: 'outlook', name: 'Outlook', platform: 'outlook', provider: 'microsoft', imap: { host: 'outlook.office365.com', port: 993, tls: 'tls' }, smtp: { host: 'smtp-mail.outlook.com', port: 587, tls: 'start_tls' } },
      { key: 'custom', name: '其他邮箱', provider: 'generic-imap' },
    ] }; },
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

test('manual mailbox validates through the shared preview, saves once and clears credentials', async () => {
  const service = fakeService(), previews = [];
  const preview = service.configPreview;
  service.configPreview = async value => { previews.push(value); return preview(value); };
  let imported = 0;
  const core = ConfigImportCore({ service, imported: () => imported++ });
  core.open(); await core.edit('mode', 'manual'); core.chooseProvider('qq');
  assert.equal(core.state.draft.imap_host, 'imap.qq.com');
  core.editMailbox('email', ' manual@qq.com '); core.editMailbox('password', ' private-password ');
  core.editMailbox('display_name', '工作邮箱');
  await core.save(); assert.equal(service.imports.length, 0);
  await core.read();
  assert.equal(previews[0].verify_connection, true);
  assert.equal(previews[0].accounts[0].auth.password, ' private-password ');
  assert.equal(previews[0].accounts[0].display_name, '工作邮箱');
  core.edit('mode', 'manual'); core.editMailbox('advanced', true);
  await core.save();
  assert.equal(service.imports.length, 1); assert.equal(imported, 1);
  assert.equal(service.imports[0].accounts[0].email, 'manual@qq.com');
  assert(!Object.hasOwn(service.imports[0], 'verify_connection'));
  assert(!JSON.stringify(core.state).includes('private-password'));
  core.destroy();
});

test('custom mailbox checks endpoints and port bounds; protocol switches choose their standard ports', async () => {
  const core = ConfigImportCore({ service: fakeService() });
  core.open(); await core.edit('mode', 'manual'); core.chooseProvider('custom');
  assert.equal(core.state.draft.advanced, true);
  core.editMailbox('email', 'local@example.com'); core.editMailbox('password', 'secret');
  await core.read(); assert.match(core.state.error, /IMAP/);
  for (const [field, value] of Object.entries({ imap_host: 'imap.example.com', smtp_host: 'smtp.example.com', username: 'login', imap_username: 'receive', smtp_username: 'send' })) core.editMailbox(field, value);
  for (const port of ['0', '65536', '1.5', '12foo', '-1']) {
    core.editMailbox('imap_port', port); await core.read(); assert.match(core.state.error, /端口/);
  }
  core.editMailbox('imap_tls', 'start_tls'); core.editMailbox('smtp_tls', 'start_tls');
  assert.equal(core.state.draft.imap_port, '143'); assert.equal(core.state.draft.smtp_port, '587');
  const account = mailboxConfig(core.state.draft).accounts[0];
  assert.equal(account.auth.username, 'login'); assert.equal(account.imap.username, 'receive'); assert.equal(account.smtp.username, 'send');
  for (const host of ['', 'https://imap.example.com', 'imap.example.com:993', 'imap.example.com:443', 'imap.example.com/path', 'user@imap.example.com', 'imap example.com']) assert.throws(() => mailboxConfig({ ...core.state.draft, imap_host: host }), /服务器/);
  await core.read(); assert.equal(core.state.preview.length, 1);
  core.editMailbox('email', 'changed@example.com'); await core.save(); assert.equal(core.state.preview.length, 0);
  core.destroy();
});

test('Outlook saves a pending OAuth account; switching providers or modes removes stale credentials and preview', async () => {
  const service = fakeService(), core = ConfigImportCore({ service });
  core.open(); await core.edit('mode', 'manual'); core.chooseProvider('qq');
  core.editMailbox('email', 'person@outlook.com'); core.editMailbox('password', 'old-secret');
  core.chooseProvider('outlook');
  assert.equal(core.state.draft.oauth, true); assert.equal(core.state.draft.password, '');
  core.editMailbox('client_id', 'application-id'); await core.read(); await core.save();
  const account = service.imports[0].accounts[0];
  assert.equal(account.provider, 'microsoft'); assert.equal(account.auth.type, 'oauth2'); assert.equal(account.auth.client_id, 'application-id');
  assert(!Object.hasOwn(account.auth, 'password')); assert(!Object.hasOwn(account.auth, 'access_token'));
  core.chooseProvider('qq'); core.editMailbox('password', 'discard-secret');
  core.edit('mode', 'url'); assert.equal(core.state.draft.password, ''); assert.equal(core.state.preview.length, 0);
  assert.throws(() => mailboxConfig({ ...mailboxDraft(), email: 'invalid', password: 'secret' }), /邮箱地址/);
  core.destroy();
});

test('manual preview failures and late responses cannot save an unverified or closed mailbox', async () => {
  const service = fakeService(), core = ConfigImportCore({ service });
  core.open(); await core.edit('mode', 'manual'); core.chooseProvider('qq');
  core.editMailbox('email', 'manual@qq.com'); core.editMailbox('password', 'secret');
  service.configPreview = async () => { throw new Error('连接验证失败'); };
  await core.read(); await core.save(); assert.equal(service.imports.length, 0); assert.match(core.state.error, /连接验证失败/);
  let finish;
  service.configPreview = () => new Promise(resolve => { finish = resolve; });
  const pending = core.read(); assert.equal(core.state.busy, true);
  core.editMailbox('email', 'changed@qq.com'); assert.equal(core.state.draft.email, 'manual@qq.com');
  core.close(); finish({ accounts: [{ index: 0, email: 'manual@qq.com' }], warnings: [] }); await pending;
  assert.equal(core.state.open, false); assert.equal(core.state.draft.password, ''); await core.save(); assert.equal(service.imports.length, 0);
  core.destroy();
});

test('encrypted imports cannot enable connection verification through their decoded fields', async () => {
  const service = fakeService(), previews = [];
  service.configPreview = async value => { previews.push(value); return { accounts: [{ index: 0, email: 'person@example.com' }], warnings: [] }; };
  const core = ConfigImportCore({ service, decrypt: async () => ({ accounts: [{ email: 'person@example.com', auth: { type: 'app_password', password: { command: 'do-not-execute' } } }], verify_connection: true }) });
  core.open(); core.edit('password', 'decrypt-key'); core.chooseFile({ name: 'config.enc', size: 1, async arrayBuffer() { return new Uint8Array(1).buffer; } });
  await core.read(); assert.equal(previews[0].verify_connection, false); assert.equal(core.state.preview.length, 1);
  core.destroy();
});

test('manual view binds provider choice, password manager controls and advanced fields to its Model', async () => {
  const core = ConfigImportCore({ service: fakeService() });
  core.open(); await core.edit('mode', 'manual'); core.chooseProvider('qq');
  core.editMailbox('password', 'temporary-secret'); core.editMailbox('advanced', true);
  const view = ConfigImport({ store: core, root: '/secret' });
  assert(find(view, 'add-mailbox-method-manual')); assert(find(view, 'mailbox-provider-qq'));
  assert.equal(find(view, 'mailbox-provider-qq-logo').state.attributes.src, '/secret/assets/mail-providers/qq.svg');
  assert.equal(find(view, 'mailbox-email').state.attributes.autocomplete, 'username');
  assert.equal(find(view, 'mailbox-email').state.attributes['aria-label'], '邮箱地址');
  const password = find(view, 'mailbox-password');
  assert.equal(password.state.attributes.type, 'password'); assert.equal(password.state.attributes.autocomplete, 'current-password');
  assert(!Object.hasOwn(password.state.attributes, 'value'));
  assert(find(view, 'mailbox-imap_host')); assert(find(view, 'mailbox-smtp_tls'));
  const listeners = new Map(), target = { value: '', addEventListener: (event, fn) => listeners.set(event, fn), removeEventListener: event => listeners.delete(event) };
  password.onMounted({ target }); assert.equal(target.value, 'temporary-secret');
  core.close(); assert.equal(target.value, ''); password.onUnmounted();
  assert.equal(listeners.size, 0); find(view, 'config-import-dialog')?.onUnmounted(); view.onUnmounted(); core.destroy();
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
