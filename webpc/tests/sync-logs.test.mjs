import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import { SyncLogsModel } from '../src/pages/logs/index.model.js';
import { SyncLogsView } from '../src/pages/logs/index.js';
import { MailService } from '../src/services.js';

for (const file of ['timeless.lite.umd.min.js', 'timeless.dom.umd.min.js']) vm.runInThisContext(fs.readFileSync(new URL(`../vendor/${file}`, import.meta.url), 'utf8'));
Object.assign(globalThis, Timeless);
const deferred = () => { let resolve; const promise = new Promise(done => resolve = done); return { promise, resolve }; };
const success = { state: 'synced', label: '已同步', tone: 'success' };
const result = { accounts: [{ id: 'a', email: 'a@example.com', status: success, last_sync_at: 1700000000, last_result: success },
  { id: 'b', email: 'b@example.com', status: { state: 'oauth_required', label: '等待 OAuth 登录', tone: 'warning' }, last_sync_at: null, last_result: null }],
  logs: [{ id: 1, account_id: 'a', completed_at: 1700000000, result: success }], more: true, retention: 1000 };
function environment(service, account = '') {
  return { root: '/secret', location: { href: `http://localhost/secret/logs${account ? '?account=' + account : ''}` },
    history: { replaceState(_state, _title, url) { this.url = url; } }, service,
    timers: { setInterval(fn) { this.tick = fn; return 1; }, clearInterval(id) { this.cleared = id; } } };
}
const find = (node, name) => node?.state?.attributes?.n === name ? node : node?.children?.map(child => find(child, name)).find(Boolean);

test('sync log filtering, pagination and latest-page polling live in the Model', async () => {
  const calls = [];
  const props = environment({ async syncLogs(params) { calls.push(params); return result; } });
  const model = SyncLogsModel(props);
  await model.start(); await model.start();
  assert.equal(calls.length, 1); assert.equal(model.state.logs[0].email, 'a@example.com');
  assert.equal(model.ui.prev.state.disabled, true); assert.equal(model.ui.next.state.disabled, false);
  await model.paginate(1); assert.equal(model.state.page, 1);
  props.timers.tick(); assert.equal(calls.length, 2);
  await model.select('b'); assert.equal(model.state.page, 0);
  assert.equal(props.history.url, '/secret/logs?account=b');
  assert.deepEqual(calls[2], { account: 'b', page: 0 });
  props.timers.tick(); await new Promise(setImmediate); assert.equal(calls.length, 4);
  model.destroy(); props.timers.tick(); assert.equal(calls.length, 4); assert.equal(props.timers.cleared, 1);
});

test('late requests cannot replace a selected mailbox or publish after destruction', async () => {
  const gates = [deferred(), deferred(), deferred()], signals = [];
  let index = 0;
  const model = SyncLogsModel(environment({ syncLogs(_params, signal) { signals.push(signal); return gates[index++].promise; } }));
  const first = model.load(); const second = model.select('b');
  assert.equal(signals[0].aborted, true);
  gates[1].resolve({ ...result, logs: [] }); await second;
  gates[0].resolve(result); await first;
  assert.equal(model.state.account, 'b'); assert.deepEqual(model.state.logs, []);
  const last = model.load(); model.destroy(); assert.equal(signals[2].aborted, true);
  gates[2].resolve(result); await last;
  assert.deepEqual(model.state.logs, []);
});

test('failed reads retain history, expose safe feedback and allow retry', async () => {
  let fail = false;
  const model = SyncLogsModel(environment({ async syncLogs() { if (fail) throw new Error('private-secret'); return result; } }));
  await model.load(); fail = true; await model.load();
  assert.equal(model.state.logs.length, 1); assert.match(model.state.error, /刷新重试/);
  assert.doesNotMatch(model.state.error, /private-secret/); assert.equal(model.ui.refresh.state.disabled, false);
  fail = false; await model.load(); assert.equal(model.state.error, ''); model.destroy();
});

test('log page provides labelled mailbox filtering and accessible history using real Timeless views', async () => {
  const calls = [];
  const props = environment({ async syncLogs(params) { calls.push(params); return result; } });
  const view = SyncLogsView(props);
  const filter = find(view, 'sync-logs-account-filter');
  const handlers = {};
  const target = { value: '', addEventListener(name, handler) { handlers[name] = handler; }, removeEventListener(name) { delete handlers[name]; } };
  filter.$elm = target;
  view.onMounted({}); await new Promise(setImmediate);
  assert.equal(filter.state.attributes['aria-label'], '筛选邮箱');
  assert.equal(find(view, 'sync-log-mailbox-b').state.as, 'article');
  assert.equal(find(view, 'sync-log-entry-1').state.as, 'tr');
  assert.equal(find(view, 'sync-logs-heading-time').state.attributes.scope, 'col');
  assert.equal(find(view, 'sync-logs-error'), undefined);
  target.value = 'b'; handlers.change(); await new Promise(setImmediate);
  assert.equal(calls.at(-1).account, 'b');
  filter.onUnmounted(); view.onUnmounted(); assert.equal(props.timers.cleared, 1); assert.equal(handlers.change, undefined);
});

test('service encodes read-only sync log parameters and propagates cancellation', async () => {
  const signal = new AbortController().signal;
  const service = MailService('/secret', async (url, options) => {
    assert.equal(url, '/secret/api/sync-logs?account=a%26b&page=2'); assert.equal(options.signal, signal);
    assert.equal(options.method, undefined); return { ok: true, json: async () => result };
  });
  assert.deepEqual(await service.syncLogs({ account: 'a&b', page: 2 }, signal), result);
});
