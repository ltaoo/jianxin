import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import { SettingsCore } from '../packages/jxui/settings-core.js';
import { Settings } from '../packages/jxui/settings.js';
import { MailService } from '../src/services.js';

for (const name of ['timeless.lite.umd.min.js', 'timeless.dom.umd.min.js']) {
  vm.runInThisContext(fs.readFileSync(new URL(`../vendor/${name}`, import.meta.url), 'utf8'));
}
Object.assign(globalThis, Timeless);

test('tray preference loads, saves once, rolls back errors and reloads persisted values', async () => {
  let saved = true, fail = false, release, writes = 0;
  const gate = new Promise(resolve => { release = resolve; });
  const core = SettingsCore({ service: {
    async configSettings() { return { tray_show_unread_count: saved }; },
    async saveConfigSettings(value) {
      writes++; await gate;
      if (fail) throw new Error('read only');
      saved = value.tray_show_unread_count; return value;
    },
  } });
  await core.open(); assert.equal(core.state.showUnreadCount, true);
  const save = core.setUnreadCount(false);
  core.close(); assert.equal(core.state.open, true);
  await core.setUnreadCount(true); assert.equal(writes, 1);
  release(); await save; assert.equal(saved, false); assert.match(core.state.status, /已保存/);
  core.close(); await core.open(); assert.equal(core.state.showUnreadCount, false);
  fail = true; await core.setUnreadCount(true);
  assert.equal(core.state.showUnreadCount, false); assert.match(core.state.error, /read only/);
  await core.setUnreadCount('false'); assert.equal(writes, 2);
  core.destroy();
});

test('closed reads cannot change preferences and failed reads can retry', async () => {
  let complete, fail = false;
  const core = SettingsCore({ service: { configSettings() {
    if (fail) throw new Error('offline');
    return new Promise(resolve => { complete = resolve; });
  } } });
  const read = core.open(); core.close(); complete({ tray_show_unread_count: false }); await read;
  assert.equal(core.state.showUnreadCount, true); assert.equal(core.state.loaded, false);
  fail = true; await core.open(); assert.match(core.state.error, /offline/);
  fail = false; const retry = core.load(); complete({ tray_show_unread_count: false }); await retry;
  assert.equal(core.state.loaded, true); assert.equal(core.state.showUnreadCount, false);
  core.destroy();
});

test('settings dialog binds its checkbox and cancellation to the Model', async () => {
  let saved = false;
  const core = SettingsCore({ service: {
    async configSettings() { return { tray_show_unread_count: saved }; },
    async saveConfigSettings(value) { saved = value.tray_show_unread_count; return value; },
  } });
  await core.open();
  const view = Settings({ store: core });
  const find = (node, name) => node?.state?.attributes?.n === name ? node : node?.children?.map(child => find(child, name)).find(Boolean);
  const checkbox = find(view, 'tray-unread-count-toggle');
  const listeners = new Map(), dialogListeners = new Map();
  const target = { checked: true, addEventListener(name, fn) { listeners.set(name, fn); }, removeEventListener(name) { listeners.delete(name); } };
  const previous = globalThis.document; let focused = false, shown = false;
  globalThis.document = { activeElement: { focus() { focused = true; } } };
  try {
    const dialog = find(view, 'settings-dialog');
    const mounts = dialog.children.map(child => child.onMounted);
    dialog.children.forEach(child => { child.onMounted = undefined; });
    dialog.onMounted({ target: { get$elm: () => ({ showModal() { shown = true; }, addEventListener(name, fn) { dialogListeners.set(name, fn); }, removeEventListener(name) { dialogListeners.delete(name); } }) } });
    dialog.children.forEach((child, index) => { child.onMounted = mounts[index]; });
    checkbox.onMounted({ target: { get$elm: () => target } });
    assert.equal(target.checked, false); assert.equal(shown, true);
    target.checked = true; listeners.get('change')();
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(saved, true); assert.equal(core.state.showUnreadCount, true);
    dialogListeners.get('cancel')({ preventDefault() {} }); assert.equal(core.state.open, false);
    checkbox.onUnmounted(); dialog.onUnmounted(); assert.equal(focused, true); assert.equal(listeners.size, 0);
  } finally { globalThis.document = previous; view.onUnmounted(); core.destroy(); }
});

test('settings service uses scoped URLs, cancellation and authorized JSON writes', async () => {
  const calls = [];
  const service = MailService('/secret', async (url, options) => { calls.push({ url, options }); return { ok: true, async json() { return {}; } }; });
  const controller = new AbortController();
  await service.configSettings(controller.signal); await service.saveConfigSettings({ tray_show_unread_count: false });
  assert.equal(calls[0].url, '/secret/api/config/settings'); assert.equal(calls[0].options.signal, controller.signal);
  assert.equal(calls[1].options.method, 'POST'); assert.equal(calls[1].options.headers['X-Jianxin-Request'], '1');
  assert.equal(calls[1].options.credentials, 'same-origin');
  assert.deepEqual(JSON.parse(calls[1].options.body), { tray_show_unread_count: false });
});
