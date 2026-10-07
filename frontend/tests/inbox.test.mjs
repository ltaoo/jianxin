import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import { ControlCore, NotificationCore } from '../packages/jxui/core.js';
import { InboxPageModel, MessageCore } from '../src/pages/inbox/index.model.js';
import { MailService } from '../src/services.js';
import { Button, Input, formatDate, RelativeTime, Glyph, NotificationCenter, NavItem } from '../packages/jxui/index.js';
import { MailMessage } from '../packages/jxui/mail-message.js';
import { InboxPageView } from '../src/pages/inbox/index.js';

// Execute the actual Timeless runtimes without a browser or DOM implementation.
for (const name of ['timeless.lite.umd.min.js', 'timeless.dom.umd.min.js']) {
  vm.runInThisContext(fs.readFileSync(new URL(`../vendor/${name}`, import.meta.url), 'utf8'));
}
Object.assign(globalThis, Timeless);
const deferred = () => { let resolve; const promise = new Promise(r => { resolve = r; }); return { promise, resolve }; };
function environment(path = '') {
  const location = { href: `http://127.0.0.1:8787/secret/${path}`, hash: '' };
  const history = { pushState(_, __, path) { location.href = new URL(path, location.href).href; }, replaceState(_, __, path) { this.pushState(_, __, path); } };
  return { root: '/secret', location, history };
}
const message = { id: 'mail-1', account_id: 'a', subject: 'Subject', is_read: false };
test('relative mail times use Unix seconds, handle old and future dates, and retain exact hover time', () => {
  const now = Date.UTC(2026, 9, 1, 12), timestamp = now / 1000;
  for (const [seconds, expected] of [[0, '刚刚'], [59, '刚刚'], [60, '1分钟前'], [300, '5分钟前'], [3600, '1小时前'], [86400, '昨天'], [172800, '前天'], [259200, '3天前'], [2592000, '上个月'], [31536000, '去年'], [-300, '5分钟后']]) {
    assert.equal(formatDate(timestamp - seconds, now), expected);
  }
  for (const value of [undefined, 0, NaN, Infinity, -1]) assert.equal(formatDate(value, now), '未知时间');
  const view = RelativeTime({ timestamp });
  assert.equal(view.state.attributes.datetime, '2026-10-01T12:00:00.000Z');
  assert.ok(view.state.attributes.title); view.onUnmounted();
});
function messageCore(service, expanded = true) {
  return MessageCore(message, { service, options: { images: true, text: false }, expanded, changed() {} });
}

test('read failures preserve state and allow explicit retry', async () => {
  let attempts = 0;
  const core = messageCore({ async setRead() { if (++attempts === 1) throw new Error('连接失败'); return { is_read: true }; } });
  await core.loaded(true);
  assert.equal(core.state.is_read, false); assert.equal(core.state.pending, false); assert.equal(core.state.error, '连接失败');
  await core.loaded(true); assert.equal(attempts, 1);
  await core.setRead(true); assert.equal(core.state.is_read, true); assert.equal(core.state.error, '');
  core.destroy();
});

test('only an expanded, visible, loaded message auto-marks read; manual unread persists', async () => {
  const writes = [];
  const core = messageCore({ async setRead(id, read) { writes.push(read); return { is_read: read }; } }, false);
  await core.loaded(true); assert.deepEqual(writes, []);
  core.toggle(); await core.loaded(false); assert.deepEqual(writes, []);
  await core.loaded(true); assert.deepEqual(writes, [true]);
  await core.setRead(false); await core.loaded(true); assert.deepEqual(writes, [true, false]);
  assert.equal(core.state.is_read, false); core.destroy();
});

test('destroying a message aborts pending work and releases listeners', async () => {
  const pending = deferred(); let signal;
  const core = messageCore({ setRead(id, read, s) { signal = s; return pending.promise; } });
  const write = core.setRead(true); core.destroy(); assert.equal(signal.aborted, true);
  pending.resolve({ is_read: true }); await write; assert.equal(core.state.is_read, false);
});

test('stale list responses cannot overwrite the most recent filter', async () => {
  const one = deferred(), two = deferred(); let requests = 0;
  const model = InboxPageModel({ ...environment(), service: { threads() { return ++requests === 1 ? one.promise : two.promise; } } });
  const first = model.methods.loadList(), second = model.methods.loadList();
  two.resolve({ threads: [{ id: 'new' }], more: false }); await second;
  one.resolve({ threads: [{ id: 'old' }], more: true }); await first;
  assert.equal(model.state.threads.value[0].id, 'new'); assert.equal(model.state.more.value, false);
  model.methods.destroy(); model.destroy();
});

test('all mailboxes keep independent folders and global navigation clears account filters', async () => {
  const calls = [];
  const accounts = ['a', 'b'].map(id => ({ id, email: `${id}@example.com`, folders: [{ id: `${id}-inbox`, name: 'Inbox' }], status: { state: 'synced', label: '已同步', tone: 'success' } }));
  const model = InboxPageModel({ ...environment(), service: {
    async mailboxes() { return { accounts }; },
    async threads(query) { calls.push(query); return { threads: [], more: false }; },
  } });
  await model.methods.init();
  assert.deepEqual(model.state.accountGroups.value.map(core => core.state.expanded), [true, true]);
  model.methods.selectFolder('b', 'b-inbox');
  assert.equal(calls.at(-1).account, 'b'); assert.equal(calls.at(-1).folder, 'b-inbox');
  model.ui.all.click();
  assert.equal(calls.at(-1).account, ''); assert.equal(calls.at(-1).folder, '');
  model.methods.destroy(); model.destroy();
});

test('status refresh preserves collapsed folders, selection and list; failures retain last status', async () => {
  let status = { state: 'synced', label: '已同步', tone: 'success' }, fail = false, lists = 0;
  const model = InboxPageModel({ ...environment(), service: {
    async mailboxes() { if (fail) throw new Error('offline'); return { accounts: [{ id: 'a', email: 'a@example.com', folders: [{ id: 'inbox', name: 'Inbox' }], status }] }; },
    async threads() { lists++; return { threads: [], more: false }; },
  } });
  await model.methods.init(); model.methods.selectFolder('a', 'inbox');
  const groups = model.state.accountGroups.value, core = groups[0], folders = core.state.folders, count = lists;
  core.toggle(); status = { state: 'login_failed', label: '登录失败：更新客户端授权码', tone: 'error' };
  await model.methods.refreshStatuses();
  assert.equal(model.state.accountGroups.value, groups); assert.equal(core.state.folders, folders);
  assert.equal(core.state.expanded, false); assert.equal(core.state.status.state, 'login_failed');
  assert.equal(model.state.folder.value, 'inbox'); assert.equal(lists, count);
  fail = true; await model.methods.refreshStatuses();
  assert.equal(model.ui.notifications.state.unread, 1); assert.equal(model.ui.notifications.state.entries[0].id, 'mailbox-status'); assert.equal(core.state.status.state, 'login_failed');
  model.methods.destroy(); model.destroy();
});

test('global mailbox totals remain independent of pages and filters and refresh to zero', async () => {
  let counts = { total: 1234, unread: 321 }, fail = false;
  const model = InboxPageModel({ ...environment(), service: {
    async mailboxes() { if (fail) throw new Error('offline'); return { accounts: [], counts }; },
    async threads() { return { threads: Array.from({ length: 50 }, (_, i) => ({ id: String(i) })), more: true }; },
  } });
  await model.methods.init();
  assert.equal(model.state.threads.value.length, 50);
  assert.equal(model.ui.all.state.count, 1234); assert.equal(model.ui.unread.state.count, 321);
  model.methods.selectFolder('a', 'inbox');
  assert.equal(model.ui.all.state.count, 1234); assert.equal(model.ui.unread.state.count, 321);
  counts = { total: 1235, unread: 322 }; await model.methods.refreshStatuses();
  assert.equal(model.ui.all.state.count, 1235); assert.equal(model.ui.unread.state.count, 322);
  fail = true; await model.methods.refreshStatuses(); assert.equal(model.ui.unread.state.count, 322);
  fail = false; counts = { total: 0, unread: 0 }; await model.methods.refreshStatuses();
  assert.equal(model.ui.all.state.count, 0); assert.equal(model.ui.unread.state.count, 0);
  const view = NavItem({ store: model.ui.all, showCount: true });
  assert.ok(view.children.length); view.onUnmounted();
  model.methods.destroy(); model.destroy();
});

test('direct message URLs and back/forward restore selection without rewriting the route', async () => {
  const env = environment('message/mail-1?images=0');
  const service = { async mailboxes() { return { accounts: [] }; }, async threads() { return { threads: [], more: false }; },
    async detail() { return { messages: [message], subject: 'Subject', more: false }; } };
  const model = InboxPageModel({ ...env, service });
  await model.methods.init(); assert.equal(model.state.selected.value.kind, 'message');
  assert.equal(model.state.options.value.images, false);
  env.location.href = 'http://127.0.0.1:8787/secret/thread/thread-2?text=1';
  model.methods.restore(); await new Promise(r => setTimeout(r, 0));
  assert.equal(model.state.selected.value.id, 'thread-2'); assert.equal(model.state.options.value.text, true);
  assert.equal(env.location.href, 'http://127.0.0.1:8787/secret/thread/thread-2?text=1');
  model.methods.destroy(); model.destroy();
});

test('notification deep links select the mailbox and thread and expand the exact mail without marking it read', async () => {
  const env = environment('message/mail-1?account=a&folder=inbox#mail-mail-1');
  const filters = []; let reads = 0;
  const service = {
    async mailboxes() { return { accounts: [{ id: 'a', email: 'a@example.com', folders: [{ id: 'inbox' }] },
      { id: 'b', email: 'b@example.com', folders: [] }] }; },
    async threads(filter) { filters.push(filter); return { threads: [{ id: 't1', account_id: 'a' }], more: false }; },
    async detail(kind, id) { assert.equal(kind, 'message'); assert.equal(id, 'mail-1');
      return { messages: [{ ...message, thread_id: 't1' }], subject: 'Subject', more: false }; },
    async setRead() { reads++; },
  };
  const model = InboxPageModel({ ...env, service });
  await model.methods.init();
  assert.equal(model.state.account.value, 'a');
  assert.equal(model.state.folder.value, 'inbox');
  assert.equal(model.state.selected.value.id, 'mail-1');
  assert.equal(model.state.selected.value.thread_id, 't1');
  assert.equal(model.state.messages.value[0].state.expanded, true);
  assert.equal(model.state.accountGroups.value[0].state.active, true);
  assert.equal(filters[0].account, 'a'); assert.equal(filters[0].folder, 'inbox');
  assert.equal(reads, 0);
  model.methods.destroy(); model.destroy();
});

test('direct mail recovery corrects a stale mailbox filter using the returned mail identity', async () => {
  const env = environment('message/mail-1?account=b&folder=old-inbox');
  const service = { async mailboxes() { return { accounts: [{ id: 'a', folders: [] }, { id: 'b', folders: [] }] }; },
    async threads() { return { threads: [], more: false }; },
    async detail() { return { messages: [message], subject: 'Subject', more: false }; } };
  const model = InboxPageModel({ ...env, service });
  await model.methods.init();
  assert.equal(model.state.account.value, 'a'); assert.equal(model.state.folder.value, '');
  assert.equal(model.state.messages.value[0].state.expanded, true);
  model.methods.destroy(); model.destroy();
});

test('mail service keeps root-scoped URLs, read POST marker and body privacy options', async () => {
  const calls = [];
  const service = MailService('/secret', async (url, options) => { calls.push([url, options]); return { ok: true, json: async () => ({ is_read: true }) }; });
  await service.setRead('mail-1', true);
  assert.equal(calls[0][0], '/secret/flags/mail-1?read=1');
  assert.equal(calls[0][1].method, 'POST'); assert.equal(calls[0][1].headers['X-Jianxin-Request'], '1');
  assert.equal(service.body('mail-1', { images: false, text: true }), '/secret/body/mail-1?images=0&text=1');
});

test('jxui uses native controls and never renders disabled="false"', () => {
  const store = ControlCore({ label: '刷新', disabled: false });
  const button = Button({ store });
  assert.equal(button.state.as, 'button'); assert.equal(button.state.attributes.disabled, undefined);
  store.set({ disabled: true }); assert.equal(button.state.attributes.disabled, '');
  button.onUnmounted(); store.destroy();
});

test('input property updates and its mount subscription is cleaned up', () => {
  const store = ControlCore({ value: 'before' });
  const input = Input({ store, label: '搜索' }); const target = { value: '' };
  input.onMounted({ target }); assert.equal(target.value, 'before');
  store.set({ value: 'after' }); assert.equal(target.value, 'after');
  input.onUnmounted(); store.set({ value: 'later' }); assert.equal(target.value, 'after'); store.destroy();
});

test('page builds real Timeless VNodes from reusable jxui without a browser', () => {
  const page = InboxPageView(environment());
  assert.equal(page.t, 'view'); assert.ok(page.children.length);
  // This test does not mount the page or invoke any browser APIs.
});


test('message view builds a sandboxed frame using actual Timeless primitives', () => {
  const service = { body() { return '/secret/body/mail-1?images=0'; } };
  const core = messageCore(service);
  const view = MailMessage({ store: core, service });
  assert.equal(view.state.as, 'article');
  assert.equal(view.state.attributes.id, 'mail-mail-1');
  // Creation exercises lazy expanded content without mounting or opening a browser.
  assert.ok(view.children.length); view.onUnmounted(); core.destroy();
});

test('official Timeless icons resolve from the framework registry', () => {
  for (const name of ['mail', 'inbox', 'sent', 'drafts', 'archive', 'trash', 'spam', 'custom', 'unread', 'refresh', 'search', 'notification', 'attachment']) {
    const icon = Glyph(name).children[0];
    assert.equal(icon.t, 'icon');
    assert.equal(Timeless.getIconRegistry()[icon.state.name].tag, 'svg');
    icon.onUnmounted();
  }
});

test('notification history deduplicates failures, preserves read state, and persists explicit read/unread', () => {
  let saved;
  const storage = { getItem() { return saved; }, setItem(key, value) { saved = value; } };
  const core = NotificationCore({ storage, now: () => 1800000000000 });
  core.record('status', '状态更新失败', '稍后自动重试');
  assert.equal(core.state.unread, 1); core.toggle(); assert.equal(core.state.open, true);
  // Opening the history does not implicitly acknowledge errors.
  assert.equal(core.state.unread, 1); core.setRead('status', true);
  core.record('status', '状态更新失败', '稍后自动重试');
  assert.equal(core.state.entries.length, 1); assert.equal(core.state.entries[0].count, 2); assert.equal(core.state.unread, 0);
  core.setRead('status', false); assert.equal(core.state.unread, 1);
  core.record('list', '列表失败', '重试'); core.readAll(); assert.equal(core.state.unread, 0);
  const restored = NotificationCore({ storage }); assert.equal(restored.state.entries.length, 2); assert.equal(restored.state.unread, 0);
  const view = NotificationCenter({ store: restored }); assert.ok(view.children.length); view.onUnmounted();
  for (let i = 0; i < 110; i++) core.record(String(i), '失败', '重试');
  assert.equal(core.state.entries.length, 100);
  core.destroy(); restored.destroy();
});

test('agent scopes ignore stale history, never send on open, and clean up polling', async () => {
  const { AgentCore } = await import('../packages/jxui/agent-core.js');
  const old = deferred(); const writes = []; let polls = 0, cleared = 0;
  const timers = { setTimeout() { polls++; return 1; }, clearTimeout() { cleared++; } };
  const service = { status: account => account === '' ? old.promise : Promise.resolve({ configured: true, messages: [{ role: 'assistant', content: account }], trace: [], drafts: [], busy: true }), action: async (op, payload) => writes.push({ op, payload }) };
  const core = AgentCore({ service, timers });core.toggle();core.scope('b');await Promise.resolve();
  old.resolve({ configured: true, messages: [{ content: 'stale' }], busy: false });await Promise.resolve();await Promise.resolve();
  assert.equal(core.state.messages[0].content,'b');assert.equal(writes.length,0);assert.equal(polls,1);
  await core.stop();assert.equal(writes[0].op,'stop');assert.equal(writes[0].payload.account,'b');core.destroy();assert.ok(cleared);
});
test('agent uses selected context, rejects duplicate sends, and records errors', async () => {
  const { AgentCore } = await import('../packages/jxui/agent-core.js');
  const pending=deferred();const writes=[];const errors=[];
  const core=AgentCore({ service:{status:async()=>({configured:true,messages:[],trace:[],drafts:[],busy:false}),action:(op,payload)=>{writes.push({op,payload});return pending.promise;}},context:()=>({kind:'thread',id:'thread-1'}),notify:error=>errors.push(error) });
  await core.reload();const send=core.send('Summarize');core.send('Duplicate');assert.equal(writes.length,1);assert.equal(writes[0].payload.context.id,'thread-1');pending.resolve({ok:true});assert.equal(await send,true);
  core.destroy();
  const failed=AgentCore({service:{status:async()=>({configured:true,busy:false}),action:async()=>{throw new Error('Model unavailable');}},notify:error=>errors.push(error)});await failed.reload();assert.equal(await failed.send('Hi'),false);assert.equal(failed.state.loading,false);assert.equal(errors.at(-1),'Model unavailable');failed.destroy();
});
test('agent service uses root protection, same-origin credentials and POST marker', async () => {
  const { AgentService } = await import('../src/services.js');const calls=[];
  const service=AgentService('/secret',async(url,options)=>{calls.push({url,options});return{ok:true,json:async()=>({ok:true})};});
  await service.status('a');await service.action('chat',{account:'a',prompt:'Hi'});
  assert.equal(calls[0].url,'/secret/api/agent?account=a');assert.equal(calls[1].options.headers['X-Jianxin-Request'],'1');assert.equal(JSON.parse(calls[1].options.body).prompt,'Hi');assert.equal(calls[1].options.credentials,'same-origin');
});

test('all conversation messages expand and persist read state after their bodies load', async () => {
  const rows = [message, { ...message, id: 'mail-2' }].map(mail => ({ ...mail }));
  const service = {
    async mailboxes() { return { accounts: [{ id: 'a', folders: [] }], counts: { total: 2, unread: rows.filter(m => !m.is_read).length } }; },
    async threads() { return { threads: [], more: false }; },
    async detail() { return { thread: { id: 't1' }, messages: rows.map(mail => ({ ...mail })), more: false }; },
    async setRead(id, read) { rows.find(mail => mail.id === id).is_read = read; return { id, is_read: read }; },
  };
  const model = InboxPageModel({ ...environment('thread/t1'), service });
  await model.methods.init();
  assert.equal(model.state.messages.value.length, 2);
  assert.ok(model.state.messages.value.every(core => core.state.expanded));
  assert.ok(rows.every(mail => !mail.is_read));
  await Promise.all(model.state.messages.value.map(core => core.loaded(true)));
  assert.ok(rows.every(mail => mail.is_read));
  await model.methods.loadDetail();
  assert.ok(model.state.messages.value.every(core => core.state.is_read));
  model.methods.destroy();
});

test('read writes queue without overlapping and failures do not block later updates', async () => {
  const gate = deferred(); let active = 0, maximum = 0, calls = 0;
  const service = MailService('/secret', async () => {
    active++; maximum = Math.max(maximum, active); const call = ++calls;
    if (call === 1) await gate.promise;
    active--;
    return { ok: call !== 1, async json() { return call === 1 ? { error: 'offline' } : { is_read: true }; } };
  });
  const first = service.setRead('mail-1', true); const failure = assert.rejects(first, /offline/);
  const second = service.setRead('mail-2', true);
  await Promise.resolve(); assert.equal(calls, 1);
  gate.resolve(); await failure; assert.equal((await second).is_read, true);
  assert.equal(maximum, 1);
});

test('read updates have no loading or success text and refresh the list silently', async () => {
  const gate = deferred(); const core = messageCore({ async setRead() { return gate.promise; } });
  const update = core.setRead(true);
  assert.equal(core.state.pending, true); assert.equal(core.state.status, '');
  gate.resolve({ is_read: true }); await update;
  assert.equal(core.state.status, ''); assert.equal(core.state.is_read, true); core.destroy();
  const list = deferred(); const model = InboxPageModel({ ...environment(), service: {
    async threads() { return list.promise; }, async mailboxes() { return { accounts: [], counts: {} }; },
  } });
  model.state.loading.as(false);
  const refresh = model.methods.refresh({ silent: true });
  assert.equal(model.state.loading.value, false);
  list.resolve({ threads: [], more: false }); await refresh;
  assert.equal(model.state.loading.value, false); model.methods.destroy();
});
