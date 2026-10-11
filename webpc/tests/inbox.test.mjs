import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import { ControlCore, MailboxCore, NotificationCore } from '../packages/jxui/core.js';
import { InboxPageModel, MessageCore } from '../src/pages/inbox/index.model.js';
import { MailService } from '../src/services.js';
import { Button, Input, formatDate, RelativeTime, Glyph, NotificationCenter, NavItem, MailboxGroup, ConversationRow } from '../packages/jxui/index.js';
import { MailMessage } from '../packages/jxui/mail-message.js';
import { InboxPageView } from '../src/pages/inbox/index.js';

// Execute the actual Timeless runtimes without a browser or DOM implementation.
for (const name of ['timeless.lite.umd.min.js', 'timeless.dom.umd.min.js']) {
  vm.runInThisContext(fs.readFileSync(new URL(`../vendor/${name}`, import.meta.url), 'utf8'));
}
Object.assign(globalThis, Timeless);
const deferred = () => { let resolve, reject; const promise = new Promise((r, j) => { resolve = r; reject = j; }); return { promise, resolve, reject }; };
function environment(path = '') {
  const location = { href: `http://127.0.0.1:8787/secret/${path}`, hash: '' };
  const history = { pushState(_, __, path) { location.href = new URL(path, location.href).href; }, replaceState(_, __, path) { this.pushState(_, __, path); } };
  return { root: '/secret', location, history };
}
const message = { id: 'mail-1', account_id: 'a', subject: 'Subject', is_read: false };

test('favorite cards unstar all pages, prevent duplicate writes and preserve failed cards for retry without navigating', async () => {
  const rows = [{ ...message, id: 'first', is_starred: true }, { ...message, id: 'second', is_starred: true }, { ...message, id: 'plain', is_starred: false }];
  const thread = { id: 't1', subject: 'Subject', account_id: 'a' };
  const writes = [], pages = []; let fail = true, gate = deferred();
  const service = {
    async mailboxes() { return { accounts: [], counts: { total: 3, unread: 3, starred: rows.filter(row => row.is_starred).length } }; },
    async threads() { return { threads: rows.some(row => row.is_starred) ? [thread] : [], more: false }; },
    async detail(kind, id, page) { assert.equal(kind, 'thread'); assert.equal(id, 't1'); pages.push(page); return { messages: page ? [rows[1]] : [rows[0], rows[2]], more: page === 0 }; },
    async setStarred(id, starred) {
      writes.push(id); assert.equal(starred, false); if (gate) await gate.promise;
      if (id === 'second' && fail) throw new Error('offline');
      rows.find(row => row.id === id).is_starred = false; return { id, is_starred: false };
    },
  };
  const model = InboxPageModel({ ...environment(), service }); await model.methods.init();
  const core = model.methods.conversation(thread);
  model.state.selected.as({ kind: 'message', id: 'first', thread_id: 't1' }); assert.equal(core.state.active, true);
  model.state.selected.as(null); assert.equal(core.state.active, false);
  const view = ConversationRow({ store: core, thread, favorite: true });
  const find = (node, name) => node?.state?.attributes?.n === name ? node : node?.children?.map(child => find(child, name)).find(Boolean);
  assert.equal(find(view, 'conversation-link').state.as, 'a');
  assert.equal(find(find(view, 'conversation-link'), 'favorite-more-toggle'), undefined);
  find(view, 'favorite-more-toggle').events.onClick({}); assert.ok(find(view, 'favorite-unstar'));
  const operation = core.unstar(); await core.unstar();
  assert.equal(core.state.pending, true); assert.equal(core.state.menuOpen, false);
  await model.methods.loadList({ silent: true }); assert.equal(model.methods.conversation(thread), core);
  gate.resolve(); await operation; gate = undefined;
  assert.deepEqual(writes, ['first', 'second']); assert.deepEqual(pages, [0, 1]);
  assert.equal(model.state.selected.value, null); assert.equal(model.state.threads.value.length, 1);
  assert.equal(model.ui.starred.state.count, 1); assert.match(core.state.error, /offline/);
  core.toggleMenu(); let focused = 0;
  view.events.onKeyDown({ key: 'Escape', preventDefault() {}, stopPropagation() {}, currentTarget: { querySelector: () => ({ focus() { focused++; } }) } });
  assert.equal(core.state.menuOpen, false); assert.equal(focused, 1);
  fail = false; await core.unstar();
  assert.deepEqual(writes, ['first', 'second', 'second']); assert.equal(model.ui.starred.state.count, 0);
  assert.equal(model.state.threads.value.length, 0); assert.equal(core.state.error, '');
  assert.ok(rows.every(row => !row.is_starred)); assert.ok(rows.every(row => !row.is_read));
  view.onUnmounted(); model.methods.destroy();
});
test('favorites navigate across mailboxes, restore URLs and refresh after successful toggles while failures preserve state', async () => {
  const row = { ...message, thread_id: 'thread-1', is_starred: false };
  let fail = false, gate;
  const calls = [];
  const env = environment();
  const model = InboxPageModel({ ...env, service: {
    async mailboxes() { return { accounts: [], counts: { total: 1, unread: 1, starred: Number(row.is_starred) } }; },
    async threads(query) { calls.push(query); return { threads: !Number(query.starred) || row.is_starred ? [{ id: 'thread-1' }] : [], more: false }; },
    async detail() { return { messages: [row], more: false }; },
    async setStarred(id, starred) { if (gate) await gate.promise; if (fail) throw new Error('offline'); row.is_starred = starred; return { id, is_starred: starred }; },
    body() { return '/secret/body/mail-1'; },
  } });
  await model.methods.init();
  model.methods.selectFolder('a', 'inbox', true);
  model.ui.starred.click();
  assert.equal(calls.at(-1).starred, '1'); assert.equal(calls.at(-1).unread, '0');
  assert.equal(calls.at(-1).account, ''); assert.equal(calls.at(-1).folder, '');
  assert.equal(new URL(env.location.href).searchParams.get('starred'), '1');
  model.methods.restore(); assert.equal(model.state.starred.value, true);
  model.methods.select({ id: 'thread-1' }); await model.methods.loadDetail();
  const core = model.state.messages.value[0];
  fail = true; await core.setStarred(true);
  assert.equal(core.state.is_starred, false); assert.match(core.state.starError, /offline/);
  fail = false; gate = deferred(); const toggle = core.setStarred(true);
  await core.setStarred(true); assert.equal(core.state.starPending, true);
  assert.equal(core.state.is_starred, true); assert.equal(model.ui.starred.state.count, 1);
  await model.methods.refreshStatuses(); assert.equal(model.ui.starred.state.count, 1);
  gate.resolve(); await toggle; gate = undefined; await model.methods.refresh({ silent: true });
  assert.equal(core.state.is_starred, true); assert.equal(core.state.starError, '');
  assert.equal(model.ui.starred.state.count, 1); assert.equal(model.state.threads.value.length, 1);
  const view = MailMessage({ store: core, service: model.services.mail });
  const find = (node, name) => node?.state?.attributes?.n === name ? node : node?.children?.map(child => find(child, name)).find(Boolean);
  assert.equal(find(view, 'mail-starred-toggle').state.attributes['aria-pressed'], 'true');
  const unstar = core.setStarred(false);
  assert.equal(find(view, 'mail-starred-toggle').state.attributes['aria-pressed'], 'false');
  assert.equal(model.ui.starred.state.count, 0);
  await unstar; await model.methods.refresh({ silent: true });
  assert.equal(model.ui.starred.state.count, 0); assert.equal(model.state.threads.value.length, 0);
  model.ui.all.click(); assert.equal(model.state.starred.value, false);
  view.onUnmounted(); model.methods.destroy(); model.destroy();
});

test('optimistic starred counts survive polling, roll back only the failed write and restore cancelled changes', async () => {
  const rows = ['one', 'two'].map(id => ({ ...message, id, thread_id: 't1', is_starred: false }));
  const gates = new Map(); let writes = 0;
  const model = InboxPageModel({ ...environment('thread/t1'), service: {
    async mailboxes() { return { accounts: [], counts: { total: 2, unread: 2, starred: rows.filter(row => row.is_starred).length } }; },
    async threads() { return { threads: [{ id: 't1' }], more: false }; },
    async detail() { return { thread: { id: 't1' }, messages: rows.map(row => ({ ...row })), more: false }; },
    async setStarred(id, starred, signal) {
      writes++; const gate = deferred(); gates.set(id, gate);
      signal.addEventListener('abort', () => gate.reject(new DOMException('Aborted', 'AbortError')), { once: true });
      await gate.promise; rows.find(row => row.id === id).is_starred = starred;
      return { id, is_starred: starred };
    },
    body: () => '/secret/body/fixture',
  } });
  await model.methods.init();
  const [first, second] = model.state.messages.value;
  const one = first.setStarred(true), two = second.setStarred(true);
  assert.equal(model.ui.starred.state.count, 2);
  await second.setStarred(false); assert.equal(writes, 2);
  await model.methods.refresh({ silent: true }); assert.equal(model.ui.starred.state.count, 2);
  gates.get('one').reject(new Error('offline')); await one;
  assert.equal(first.state.is_starred, false); assert.match(first.state.starError, /offline/);
  assert.equal(second.state.is_starred, true); assert.equal(model.ui.starred.state.count, 1);
  await model.methods.refreshStatuses(); assert.equal(model.ui.starred.state.count, 1);
  gates.get('two').resolve(); await two; await model.methods.refresh({ silent: true });
  assert.equal(model.ui.starred.state.count, 1); assert.equal(model.ui.unread.state.count, 2);
  const view = MailMessage({ store: second, service: model.services.mail });
  const find = node => node?.state?.attributes?.n === 'mail-starred-toggle' ? node : node?.children?.map(find).find(Boolean);
  const cancel = second.setStarred(false);
  assert.equal(second.state.is_starred, false); assert.equal(model.ui.starred.state.count, 0);
  assert.equal(find(view).state.attributes['aria-pressed'], 'false'); assert.equal(find(view).state.attributes.disabled, '');
  await model.methods.refreshStatuses(); assert.equal(model.ui.starred.state.count, 0);
  model.methods.close(); await cancel;
  assert.equal(model.ui.starred.state.count, 1); assert.equal(model.state.selected.value, null);
  view.onUnmounted(); model.methods.destroy(); model.destroy();
});

test('starred writes share the protected serialized queue with read writes', async () => {
  const pending = deferred(), calls = [];
  const service = MailService('/secret', async (url, options) => {
    calls.push({ url, options });
    if (calls.length === 1) await pending.promise;
    return { ok: true, json: async () => ({ is_starred: true }) };
  });
  const read = service.setRead('mail-1', true), star = service.setStarred('mail-1', true);
  await Promise.resolve(); assert.equal(calls.length, 1);
  pending.resolve(); await Promise.all([read, star]);
  assert.equal(calls[1].url, '/secret/flags/mail-1?starred=1');
  assert.equal(calls[1].options.method, 'POST'); assert.equal(calls[1].options.headers['X-Jianxin-Request'], '1');
});

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

test('webmail header shows sender email and both reply entries; more actions close and restore focus', async () => {
  let replies = 0;
  const core = MessageCore({ ...message, sender: 'Sender', sender_email: 'sender@example.com', recipient: 'owner@example.com' }, {
    service: { body: () => '/body/mail-1', metadata: async () => ({ id: 'mail-1', from: [], to: [], cc: [] }), setRead: async (id, read) => ({ is_read: read }) },
    options: {}, expanded: true, changed() {}, reply() { replies++; },
  });
  const view = MailMessage({ store: core, service: { body: () => '/body/mail-1' } });
  const find = (node, name) => node?.state?.attributes?.n === name ? node : node?.children?.map(child => find(child, name)).find(Boolean);
  assert.equal(find(view, 'mail-message-sender-email').state.attributes.title, 'sender@example.com');
  assert.equal(find(view, 'mail-read-toggle'), undefined);
  core.toggleMenu(); assert.equal(find(view, 'mail-more-toggle').state.attributes['aria-expanded'], 'true');
  let focused = 0;
  view.events.onKeyDown({ key: 'Escape', preventDefault() {}, stopPropagation() {}, currentTarget: { querySelector: () => ({ focus() { focused++; } }) } });
  assert.equal(core.state.menuOpen, false); assert.equal(focused, 1);
  core.toggleMenu(); find(view, 'mail-read-toggle').events.onClick({});
  assert.equal(core.state.menuOpen, false); assert.equal(core.state.is_read, true);
  core.toggleMenu(); find(view, 'mail-metadata-toggle').events.onClick({});
  assert.equal(core.state.infoPanel, 'metadata'); assert.equal(core.state.menuOpen, false);
  find(view, 'mail-reply-open').events.onClick({}); find(view, 'mail-reply-bottom').events.onClick({});
  assert.equal(replies, 2);
  await new Promise(resolve => setTimeout(resolve, 0));
  view.onUnmounted(); core.destroy();
});

test('reader navigation follows the loaded conversation order, handles boundaries and supports message deep links', async () => {
  const model = InboxPageModel({ ...environment(), service: { async detail(kind, id) { return { thread: { id }, messages: [], more: false }; } } });
  model.state.threads.as([{ id: 'first' }, { id: 'second' }]);
  assert.equal(model.ui.nextConversation.state.disabled, true);
  model.state.selected.as({ kind: 'thread', id: 'first' });
  assert.equal(model.state.conversationPosition.value, '第 1 个会话，共 2 个');
  assert.equal(model.ui.previousConversation.state.disabled, true);
  model.ui.nextConversation.click();
  assert.equal(model.state.selected.value.id, 'second'); assert.equal(model.ui.nextConversation.state.disabled, true);
  model.methods.navigateConversation(1); assert.equal(model.state.selected.value.id, 'second');
  model.state.selected.as({ kind: 'message', id: 'mail-1', thread_id: 'second' });
  model.ui.previousConversation.click(); assert.equal(model.state.selected.value.id, 'first');
  model.ui.readerWidth.click(); assert.equal(model.state.readerWide.value, true); assert.equal(model.ui.readerWidth.state.label, '恢复分栏');
  model.ui.readerWidth.click(); assert.equal(model.state.readerWide.value, false);
  model.state.selected.as({ kind: 'thread', id: 'absent' });
  assert.equal(model.state.conversationPosition.value, ''); assert.equal(model.ui.previousConversation.state.disabled, true);
  assert.equal(model.ui.nextConversation.state.disabled, true);
  await new Promise(resolve => setTimeout(resolve, 0)); model.methods.destroy();
});

test('folder detail requests preserve folder membership while direct message URLs keep their exact identity', async () => {
  const calls = [];
  const service = MailService('/secret', async url => { calls.push(url); return { ok: true, json: async () => ({}) }; });
  await service.detail('thread', 't1', 2, undefined, 'inbox');
  assert.equal(new URL(calls[0], 'http://localhost').searchParams.get('folder'), 'inbox');
  assert.equal(new URL(calls[0], 'http://localhost').searchParams.get('page'), '2');
  await service.detail('message', 'm1', 0, undefined, 'inbox');
  assert.equal(new URL(calls[1], 'http://localhost').searchParams.get('folder'), '');
  const model = InboxPageModel({ ...environment(), service: { async detail(kind, id, page, signal, folder) { assert.equal(folder, 'inbox'); return { thread: { id }, messages: [], more: false }; } } });
  model.state.folder.as('inbox'); model.state.selected.as({ kind: 'thread', id: 't1' });
  await model.methods.loadDetail();
  model.state.threads.as([{ id: 't1', unread_count: 1 }]);
  model.state.detail.as({ id: 't1', unread_count: 1 });
  model.methods.readChanged({ id: 'sent-reply', thread_id: 't1', folder_id: 'sent', is_read: true }, false);
  assert.equal(model.state.threads.value[0].unread_count, 1);
  assert.equal(model.state.detail.value.unread_count, 1);
  model.methods.destroy();
});

test('sender and metadata details share one cached read-only request without expanding the message', async () => {
  const gate = deferred(); let requests = 0, reads = 0;
  const core = messageCore({ body: () => '/body/mail-1', async metadata(id) { requests++; assert.equal(id, 'mail-1'); return gate.promise; },
    async setRead() { reads++; } }, false);
  const view = MailMessage({ store: core, service: { body: () => '/body/mail-1' } });
  const find = (node, name) => node?.state?.attributes?.n === name ? node : node?.children?.map(child => find(child, name)).find(Boolean);
  const sender = find(view, 'mail-message-sender');
  assert.equal(sender.state.as, 'button');
  assert.equal(find(view, 'mail-message-header').state.as, 'div');
  sender.events.onClick({}); assert.equal(core.state.infoPanel, 'sender'); assert.equal(core.state.expanded, false);
  assert.equal(sender.state.attributes['aria-expanded'], 'true');
  core.toggleInfo('metadata'); assert.equal(requests, 1);
  gate.resolve({ id: 'mail-1', thread_id: 'thread-1', message_id_header: '<original@example.com>', from: [{ name: 'Sender', email: 'sender@example.com' }], reply_to: [] });
  await new Promise(r => setTimeout(r, 0));
  assert.ok(find(view, 'mail-metadata-id-value')); assert.ok(find(view, 'mail-metadata-thread_id-value'));
  core.toggleInfo('sender'); assert.ok(find(view, 'mail-sender-email')); assert.equal(reads, 0);
  assert.equal(core.state.metadata.from[0].email, 'sender@example.com');
  core.closeInfo(); await core.toggleInfo('metadata'); assert.equal(requests, 1);
  let focused = false;
  find(view, 'mail-info-close').events.onClick({ currentTarget: { closest: () => ({ querySelector: () => ({ focus() { focused = true; } }) }) } });
  assert.equal(core.state.infoPanel, ''); assert.equal(focused, true);
  view.onUnmounted(); core.destroy();
});

test('metadata failures remain retryable and destroying a message aborts the details request', async () => {
  let attempts = 0;
  const core = messageCore({ async metadata() { if (++attempts === 1) throw new Error('offline'); return { id: 'mail-1', from: [] }; } });
  await core.toggleInfo('metadata'); assert.equal(core.state.metadataError, 'offline');
  await core.loadMetadata(); assert.equal(core.state.metadata.id, 'mail-1'); assert.equal(core.state.metadataError, ''); core.destroy();
  const gate = deferred(); let signal;
  const pending = messageCore({ metadata(id, next) { signal = next; return gate.promise; } });
  const operation = pending.toggleInfo('sender'); pending.destroy(); assert.equal(signal.aborted, true);
  gate.resolve({ id: 'mail-1' }); await operation; assert.equal(pending.state.metadata, null);
});

test('metadata uses a native modal with Escape/backdrop dismissal and cleans up dialog listeners', () => {
  const core = messageCore({ body: () => '/body/mail-1' });
  core.set({ infoPanel: 'metadata', metadata: { id: 'mail-1', from: [] } });
  const view = MailMessage({ store: core, service: { body: () => '/body/mail-1' } });
  const find = (node, name) => node?.state?.attributes?.n === name ? node : node?.children?.map(child => find(child, name)).find(Boolean);
  const modal = find(view, 'mail-metadata-dialog');
  assert.equal(modal.state.as, 'dialog'); assert.equal(modal.state.attributes['aria-modal'], 'true');
  const listeners = new Map(); let shown = 0, closed = 0, focused = 0;
  const target = { open: false, addEventListener(name, listener) { listeners.set(name, listener); }, removeEventListener(name) { listeners.delete(name); },
    showModal() { this.open = true; shown++; }, close() { this.open = false; closed++; },
    getBoundingClientRect: () => ({ left: 100, right: 500, top: 100, bottom: 500 }),
    closest: () => ({ querySelector: () => ({ focus() { focused++; } }) }),
  };
  // Mount the dialog with Timeless DOM's wrapper shape; children need no real browser.
  const heading = modal.children[0]; const mounted = heading.onMounted; heading.onMounted = undefined;
  modal.onMounted({ target: { get$elm: () => target } }); heading.onMounted = mounted;
  assert.equal(shown, 1);
  listeners.get('click')({ target, currentTarget: target, clientX: 200, clientY: 200 }); assert.equal(core.state.infoPanel, 'metadata');
  listeners.get('click')({ target, currentTarget: target, clientX: 10, clientY: 10 });
  assert.equal(core.state.infoPanel, ''); assert.equal(focused, 1);
  assert.equal(closed, 1); assert.equal(listeners.size, 0);
  core.set({ infoPanel: 'metadata' });
  const reopened = find(view, 'mail-metadata-dialog');
  const reopenedHeading = reopened.children[0]; const reopenedMounted = reopenedHeading.onMounted; reopenedHeading.onMounted = undefined;
  reopened.onMounted({ target: { get$elm: () => target } }); reopenedHeading.onMounted = reopenedMounted;
  assert.equal(shown, 2);
  let prevented = false;
  listeners.get('cancel')({ currentTarget: target, preventDefault() { prevented = true; } });
  assert.equal(prevented, true); assert.equal(core.state.infoPanel, ''); assert.equal(focused, 2);
  assert.equal(closed, 2); assert.equal(listeners.size, 0);
  view.onUnmounted(); core.destroy();
});

test('read failures preserve state and allow explicit retry', async () => {
  let attempts = 0;
  const core = messageCore({ async setRead() { if (++attempts === 1) throw new Error('连接失败'); return { is_read: true }; } });
  await core.loaded(true);
  assert.equal(core.state.is_read, false); assert.equal(core.state.pending, false); assert.equal(core.state.error, '操作失败：连接失败');
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
  const write = core.setRead(true); assert.equal(core.state.is_read, true);
  core.destroy(); assert.equal(signal.aborted, true);
  pending.resolve({ is_read: true }); await write; assert.equal(core.state.is_read, true);
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

test('mailbox platform overrides transport and config refresh updates only the existing group', async () => {
  let account = { id: 'a', email: 'a@example.com', provider: 'google', folders: [] };
  const model = InboxPageModel({ ...environment(), service: { async mailboxes() { return { accounts: [account] }; } } });
  await model.methods.loadMailboxes();
  const core = model.state.accountGroups.value[0]; core.toggle();
  assert.equal(core.state.platformIcon, 'gmail');
  account = { ...account, platform: ' QQ ' }; await model.methods.loadMailboxes();
  assert.equal(model.state.accountGroups.value[0], core); assert.equal(core.state.platformIcon, 'qq');
  assert.equal(core.state.expanded, false);
  for (const [platform, provider, expected] of [[undefined, 'microsoft', 'outlook'], ['', 'google', 'gmail'], [' ', 'google', 'gmail'], [undefined, 'generic-imap', ''], ['163', 'generic-imap', 'netease163'], ['aliyun', 'google', 'aliyun'], ['unknown', 'google', ''], ['__proto__', 'google', '']]) {
    core.updateAccount({ ...account, platform, provider }); assert.equal(core.state.platformIcon, expected);
  }
  model.methods.destroy(); model.destroy();
});

test('conversation mailbox favicons share sidebar accounts, react to platform changes and retain a generic fallback', async () => {
  let accounts = [
    { id: 'a', email: 'a@custom.example', platform: 'qq', provider: 'google', folders: [] },
    { id: 'b', email: 'b@custom.example', provider: 'microsoft', folders: [] },
    { id: 'c', email: 'c@custom.example', provider: 'generic-imap', folders: [] },
  ];
  const model = InboxPageModel({ ...environment(), service: { async mailboxes() { return { accounts }; } } });
  await model.methods.loadMailboxes();
  const find = (node, name) => node?.state?.attributes?.n === name ? node : node?.children?.map(child => find(child, name)).find(Boolean);
  const views = accounts.map(account => {
    const store = ControlCore({ href: '/secret/thread/t', active: true });
    const view = ConversationRow({ store, mailbox: model.methods.mailbox(account.id), root: '/secret',
      thread: { id: 't', account_id: account.id, account_email: account.email, subject: 'Subject' } });
    return { store, view };
  });
  assert.equal(find(views[0].view, 'conversation-mailbox-favicon').state.attributes.src, '/secret/assets/mail-providers/qq.svg');
  assert.equal(find(views[1].view, 'conversation-mailbox-favicon').state.attributes.src, '/secret/assets/mail-providers/outlook.svg');
  assert.equal(find(views[2].view, 'conversation-mailbox-favicon'), undefined);
  assert.equal(find(views[0].view, 'conversation-mailbox-email').state.attributes.title, 'a@custom.example');
  accounts = accounts.map(account => account.id === 'a' ? { ...account, platform: '163' } : account);
  await model.methods.loadMailboxes();
  const image = find(views[0].view, 'conversation-mailbox-favicon');
  assert.equal(image.state.attributes.src, '/secret/assets/mail-providers/netease163.svg');
  assert.equal(views[0].view.state.attributes['aria-current'], 'true');
  for (const { view, store } of views) { view.onUnmounted(); store.destroy(); }
  model.methods.mailbox('a').updateAccount({ ...accounts[0], platform: 'gmail' });
  assert.equal(image.state.attributes.src, '/secret/assets/mail-providers/netease163.svg');
  model.methods.destroy(); model.destroy();
});

test('mailbox badges open safe details without selecting an account and recover automatically', () => {
  let selected = 0;
  const account = { id: 'a', email: 'a@qq.com', platform: 'qq', folders: [] };
  const core = MailboxCore(account, { click() { selected++; } });
  for (const tone of ['success', 'neutral']) {
    core.updateAccount({ ...account, status: { tone, label: '已同步' } });
    core.toggleStatus(); assert.equal(core.state.problem, ''); assert.equal(core.state.statusOpen, false);
  }
  for (const [state, tone, label] of [['not_synced', 'warning', '未同步'], ['oauth_required', 'warning', '等待 OAuth 登录'], ['missing_credentials', 'error', '缺少凭据：配置客户端授权码'], ['login_failed', 'error', '登录失败：更新客户端授权码'], ['sync_failed', 'error', '同步失败']]) {
    const failed = { ...account, status: { state, tone, label } };
    core.updateAccount(failed); assert.equal(core.state.problem, label); assert.equal(core.state.statusOpen, false);
    core.toggleStatus(); assert.equal(core.state.statusOpen, true); assert.equal(selected, 0);
    core.updateAccount(failed); assert.equal(core.state.statusOpen, true);
    core.closeStatus(); assert.equal(core.state.statusOpen, false);
  }
  core.toggleStatus(); core.updateAccount({ ...account, status: { tone: 'success', state: 'synced', label: '已同步' } });
  assert.equal(core.state.problem, ''); assert.equal(core.state.statusOpen, false);
  core.destroy();
});

test('mailbox status view closes on outside click or Escape and removes document listeners', () => {
  const previous = globalThis.document, listeners = new Map(); let focused = false;
  globalThis.document = { addEventListener(name, callback) { listeners.set(name, callback); }, removeEventListener(name) { listeners.delete(name); } };
  const icon = {}, popup = {};
  const core = MailboxCore({ id: 'a', email: 'a@qq.com', platform: 'qq', folders: [], status: { tone: 'error', label: '缺少凭据' } });
  const view = MailboxGroup({ store: core, root: '/secret' }, () => null);
  try {
    view.onMounted({ target: { querySelector(selector) {
      if (selector === '.jx-mailbox-icon') return { contains: target => target === icon };
      if (selector === '.jx-mailbox-status-popover') return { contains: target => target === popup };
      return { focus() { focused = true; } };
    } } });
    core.toggleStatus(); listeners.get('pointerdown')({ target: icon }); assert.equal(core.state.statusOpen, true);
    listeners.get('pointerdown')({ target: popup }); assert.equal(core.state.statusOpen, true);
    listeners.get('keydown')({ key: 'Escape' }); assert.equal(core.state.statusOpen, false); assert.equal(focused, true);
    core.toggleStatus(); listeners.get('pointerdown')({ target: {} }); assert.equal(core.state.statusOpen, false);
  } finally { view.onUnmounted(); core.destroy(); globalThis.document = previous; }
  assert.equal(listeners.size, 0);
});

test('mailbox VNodes hide healthy labels and bind favicon and problem controls reactively', () => {
  let selected = 0;
  const account = { id: 'a', email: 'a@qq.com', platform: 'qq', provider: 'google', folders: [] };
  const core = MailboxCore({ ...account, status: { tone: 'success', label: '已同步' } }, { click() { selected++; } });
  const view = MailboxGroup({ store: core, root: '/secret' }, () => null);
  const find = (node, name) => node?.state?.attributes?.n === name ? node : node?.children?.map(child => find(child, name)).find(Boolean);
  assert.equal(find(view, 'mailbox-status-trigger'), undefined);
  assert.equal(find(view, 'mailbox-status-details'), undefined);
  assert.equal(find(view, 'mailbox-platform-favicon').state.attributes.src, '/secret/assets/mail-providers/qq.svg');
  core.updateAccount({ ...account, platform: 'outlook', status: { tone: 'warning', label: '等待 OAuth 登录' } });
  assert.equal(find(view, 'mailbox-platform-favicon').state.attributes.src, '/secret/assets/mail-providers/outlook.svg');
  const trigger = find(view, 'mailbox-status-trigger');
  assert.equal(trigger.state.as, 'button'); assert.match(trigger.state.attributes['aria-label'], /等待 OAuth 登录/);
  trigger.events.onClick({}); assert.equal(selected, 0); assert.equal(trigger.state.attributes['aria-expanded'], 'true');
  assert.equal(find(view, 'mailbox-status-details').state.attributes.role, 'status');
  core.updateAccount({ ...account, status: { tone: 'success', label: '已同步' } });
  assert.equal(find(view, 'mailbox-status-trigger'), undefined); assert.equal(find(view, 'mailbox-status-details'), undefined);
  view.onUnmounted(); core.destroy();
});

test('OAuth mailbox button starts one login, shows the device code, reloads on completion and cleans up polling', async () => {
  const account = { id: 'a', email: 'a@outlook.com', provider: 'microsoft', folders: [], status: { state: 'oauth_required', tone: 'warning', label: '等待 OAuth 登录' } };
  const tasks = []; let starts = 0, reloads = 0, signal;
  let result = { state: 'pending', busy: true, user_code: 'ABCD-EFGH', verification_uri: 'https://microsoft.com/devicelogin' };
  const model = InboxPageModel({ ...environment(), timers: { setTimeout(callback) { tasks.push(callback); return callback; }, clearTimeout(callback) { const i = tasks.indexOf(callback); if (i >= 0) tasks.splice(i, 1); } }, service: {
    async mailboxes() { return { accounts: [account] }; }, async threads() { return { threads: [], more: false }; },
    async startOAuth(id, s) { starts++; signal = s; return { state: 'starting', busy: true }; },
    async oauthStatus() { return result; },
    async reloadMailboxes() { reloads++; return { accounts: [{ ...account, status: { state: 'not_synced', tone: 'warning', label: '未同步' } }] }; },
  } });
  await model.methods.init();
  const core = model.state.accountGroups.value[0]; core.toggleStatus();
  const view = MailboxGroup({ store: core, root: '/secret' }, () => null);
  const find = (node, name) => node?.state?.attributes?.n === name ? node : node?.children?.map(child => find(child, name)).find(Boolean);
  const button = find(view, 'start-mailbox-oauth'); assert.equal(button.state.as, 'button');
  const start = button.events.onClick({}); core.startOAuth(); await start; await Promise.resolve();
  assert.equal(starts, 1); assert.equal(button.state.attributes.disabled, '');
  await tasks.shift()();
  assert.equal(core.state.oauth.user_code, 'ABCD-EFGH');
  assert.equal(find(view, 'open-mailbox-oauth-page').state.attributes.href, 'https://microsoft.com/devicelogin');
  assert.equal(find(view, 'open-mailbox-oauth-page').state.attributes.rel, 'noopener noreferrer');
  result = { state: 'complete', busy: false }; await tasks.shift()();
  assert.equal(reloads, 1); assert.equal(core.state.oauth.state, 'complete'); assert.equal(core.state.status.state, 'not_synced');
  assert.equal(core.state.statusOpen, false); assert.equal(tasks.length, 0);
  core.updateAccount(account); await core.startOAuth(); assert.equal(starts, 2);
  view.onUnmounted(); model.methods.destroy(); model.destroy();
  assert.equal(signal.aborted, true); assert.equal(tasks.length, 0);
});

test('OAuth startup errors can retry and unsafe authorization URLs never become links', async () => {
  const account = { id: 'a', email: 'a@outlook.com', folders: [], status: { state: 'oauth_required', tone: 'warning', label: '等待 OAuth 登录' } };
  let attempts = 0;
  const model = InboxPageModel({ ...environment(), service: {
    async mailboxes() { return { accounts: [account] }; },
    async startOAuth() { if (++attempts === 1) throw new Error('请检查 OAuth 配置'); return { state: 'pending', busy: true, verification_uri: 'javascript:alert(1)' }; },
  } });
  await model.methods.loadMailboxes(); const core = model.state.accountGroups.value[0];
  await core.startOAuth(); assert.match(core.state.oauth.error, /OAuth 配置/); assert.equal(core.state.oauth.busy, false);
  await core.startOAuth(); assert.match(core.state.oauth.error, /HTTPS/); assert.equal(core.state.oauth.verification_uri, undefined);
  model.methods.destroy(); model.destroy();
});

test('pending OAuth accounts remain visible, accept missing application settings, and authorize only on a manual click', async () => {
  const endpoint = 'https://login.microsoftonline.com/consumers/oauth2/v2.0/token';
  let account = { id: 'pending', email: 'pending@outlook.com', folders: [], provider: 'microsoft',
    status: { state: 'oauth_required', tone: 'warning', label: '等待 OAuth 登录' },
    oauth_config: { client_id: '', token_endpoint: endpoint, needs_setup: true } };
  let saves = 0, starts = 0;
  const model = InboxPageModel({ ...environment(), service: {
    async mailboxes() { return { accounts: [account] }; }, async threads() { return { threads: [], more: false }; },
    async saveOAuthSettings(id, settings) {
      saves++; assert.equal(id, 'pending'); assert.equal(settings.client_id, 'application-id');
      account = { ...account, oauth_config: { ...settings, needs_setup: false } };
      return { saved: true };
    },
    async startOAuth() { starts++; return { state: 'failed', busy: false, error: 'fixture-no-network' }; },
  } });
  await model.methods.init(); const core = model.state.accountGroups.value[0];
  assert.equal(core.state.email, 'pending@outlook.com'); assert.equal(starts, 0);
  core.toggleStatus();
  const view = MailboxGroup({ store: core, root: '/secret' }, () => null);
  const find = (node, name) => node?.state?.attributes?.n === name ? node : node?.children?.map(child => find(child, name)).find(Boolean);
  assert.ok(find(view, 'mailbox-oauth-setup')); assert.equal(find(view, 'start-mailbox-oauth').state.attributes.disabled, '');
  await core.startOAuth(); assert.equal(starts, 0);
  core.editOAuth('client_id', 'application-id'); await model.methods.refreshStatuses();
  assert.equal(core.state.oauthDraft.client_id, 'application-id');
  await core.saveOAuthSettings(); assert.equal(saves, 1); assert.equal(starts, 0);
  assert.equal(core.state.oauth_config.needs_setup, false);
  assert.equal(find(view, 'start-mailbox-oauth').state.attributes.disabled, undefined);
  await core.startOAuth(); assert.equal(starts, 1);
  view.onUnmounted(); model.methods.destroy(); model.destroy();
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

test('mailbox config refresh updates groups, preserves selection, resets removed accounts and retains state on failure', async () => {
  const account = id => ({ id, email: `${id}@example.com`, folders: [{ id: `${id}-inbox` }] });
  let accounts = [account('a')], next = [...accounts, account('b')], fail = false, calls = 0, reads = 0;
  let daemon = { status: 'restarted' };
  const gate = deferred();
  const env = environment();
  const model = InboxPageModel({ ...env, service: {
    async mailboxes() { reads++; return { accounts }; },
    async reloadMailboxes() { calls++; await gate.promise; if (fail) throw new Error('配置无效'); accounts = next; return { accounts, daemon }; },
    async threads() { return { threads: [], more: false }; },
  } });
  await model.methods.init(); model.methods.selectFolder('a', 'a-inbox');
  const core = model.state.accountGroups.value[0]; core.toggle();
  const pending = model.methods.reloadMailboxes();
  assert.equal(model.ui.reloadMailboxes.state.disabled, true);
  await model.methods.reloadMailboxes(); await model.methods.refreshStatuses();
  assert.equal(calls, 1); assert.equal(reads, 1);
  gate.resolve(); await pending;
  assert.deepEqual(model.state.accounts.value.map(a => a.id), ['a', 'b']);
  assert.equal(model.state.accountGroups.value[0], core); assert.equal(core.state.expanded, false);
  assert.equal(model.state.account.value, 'a'); assert.equal(model.state.folder.value, 'a-inbox');
  assert.match(model.state.configStatus.value, /后台收信已重启/);
  next = [account('b')]; model.state.selected.as({ kind: 'thread', id: 'old-thread' });
  await model.methods.reloadMailboxes();
  assert.equal(model.state.account.value, ''); assert.equal(model.state.folder.value, '');
  assert.equal(model.state.selected.value, null); assert.equal(new URL(env.location.href).searchParams.has('account'), false);
  const groups = model.state.accountGroups.value; fail = true;
  await model.methods.reloadMailboxes();
  assert.equal(model.state.accountGroups.value, groups); assert.equal(model.state.error.value, '配置无效');
  assert.equal(model.ui.notifications.state.entries[0].id, 'mailbox-config');
  assert.equal(model.ui.reloadMailboxes.state.disabled, false);
  fail = false; daemon = { status: 'failed', error: '邮箱配置已刷新，但后台收信重启失败' };
  await model.methods.reloadMailboxes();
  assert.equal(model.state.accountGroups.value, groups);
  assert.equal(model.state.configStatus.value, daemon.error);
  assert.equal(model.state.error.value, daemon.error);
  daemon = { status: 'not_running' }; await model.methods.reloadMailboxes();
  assert.match(model.state.configStatus.value, /后台收信服务未运行/);
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
  await service.reloadMailboxes();
  assert.equal(calls[1][0], '/secret/api/mailboxes/reload'); assert.equal(calls[1][1].method, 'POST');
  assert.equal(calls[1][1].headers['X-Jianxin-Request'], '1'); assert.equal(calls[1][1].credentials, 'same-origin');
  assert.equal(service.body('mail-1', { images: false, text: true }), '/secret/body/mail-1?images=0&text=1');
  await service.startOAuth('account-1');
  assert.equal(calls.at(-1)[0], '/secret/api/mailboxes/oauth?account=account-1');
  assert.equal(calls.at(-1)[1].method, 'POST'); assert.equal(calls.at(-1)[1].headers['X-Jianxin-Request'], '1');
  await service.oauthStatus('account-1'); assert.equal(calls.at(-1)[1].method, undefined);
});

test('reply service uses protected JSON POST and retains HTTP status on failure', async () => {
  const calls = [];
  const service = MailService('/secret', async (url, options) => { calls.push({ url, options }); return { ok: true, json: async () => ({ ok: true }) }; });
  await service.prepareReply('mail-1'); await service.sendReply('mail-1', { subject: 'Re: Subject', body: 'Thanks' });
  assert.equal(calls[0].url, '/secret/api/reply/mail-1'); assert.equal(calls[1].url, calls[0].url);
  assert.equal(calls[1].options.method, 'POST'); assert.equal(calls[1].options.credentials, 'same-origin');
  assert.equal(calls[1].options.headers['X-Jianxin-Request'], '1');
  assert.equal(calls[1].options.headers['Content-Type'], 'application/json');
  assert.deepEqual(JSON.parse(calls[1].options.body), { subject: 'Re: Subject', body: 'Thanks' });
  const failed = MailService('', async () => ({ ok: false, status: 502, json: async () => ({ error: 'offline' }) }));
  await assert.rejects(failed.sendReply('mail-1', {}), error => error.status === 502 && error.message === 'offline');
});

test('metadata service is a root-scoped GET with same-origin credentials', async () => {
  let call;
  const service = MailService('/secret', async (url, options) => { call = { url, options }; return { ok: true, json: async () => ({ id: 'mail-1' }) }; });
  await service.metadata('mail-1');
  assert.equal(call.url, '/secret/api/message/mail-1/metadata');
  assert.equal(call.options.method, undefined); assert.equal(call.options.credentials, 'same-origin');
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
  const find = node => node?.state?.attributes?.n === 'refresh-mailbox-config' ? node : node?.children?.map(find).find(Boolean);
  const reload = find(page);
  assert.equal(reload.state.as, 'button'); assert.equal(reload.state.attributes['aria-label'], '刷新邮箱配置');
  const navigation = page.children[0].children[1].children;
  assert.deepEqual(navigation.slice(0, 3).map(node => node.state.attributes.n), ['mail-navigation-all', 'mail-navigation-unread', 'mail-navigation-starred']);
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

test('mail iframe resolves the Timeless host before measuring full content and cleans up observers', () => {
  const saved = Object.fromEntries(['window', 'document', 'ResizeObserver', 'requestAnimationFrame', 'cancelAnimationFrame'].map(key => [key, globalThis[key]]));
  let queued, observed, disconnected = false, visible = false;
  const listeners = new Map();
  const content = { scrollHeight: 2400, getBoundingClientRect: () => ({ height: 2200 }),
    addEventListener(name, fn) { listeners.set(name, fn); }, removeEventListener(name) { listeners.delete(name); } };
  const frame = { style: {}, contentDocument: { getElementById: () => content }, addEventListener() {}, removeEventListener() {} };
  try {
    Object.assign(globalThis, { window: { addEventListener() {}, removeEventListener() {} }, document: { hidden: false, addEventListener() {}, removeEventListener() {} },
      ResizeObserver: class { constructor(callback) { this.callback = callback; } observe(node) { observed = node; } disconnect() { disconnected = true; } },
      requestAnimationFrame(callback) { queued = callback; return 1; }, cancelAnimationFrame() {} });
    const core = messageCore({ body: () => '/secret/body/mail-1' }); core.loaded = value => { visible = value; };
    const view = MailMessage({ store: core, service: { body: () => '/secret/body/mail-1' } });
    const find = node => node?.state?.attributes?.n === 'mail-body-frame' ? node : node?.children?.map(find).find(Boolean);
    const body = find(view);
    body.onMounted({ target: { get$elm: () => frame } });
    queued(); assert.equal(frame.style.height, '2432px'); assert.equal(observed, content); assert.equal(visible, true);
    content.scrollHeight = 3600; listeners.get('load')(); queued(); assert.equal(frame.style.height, '3632px');
    body.onUnmounted(); assert.equal(disconnected, true); assert.equal(listeners.size, 0);
    view.onUnmounted(); core.destroy();
  } finally { for (const [key, value] of Object.entries(saved)) if (value === undefined) delete globalThis[key]; else globalThis[key] = value; }
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
  assert.equal(core.state.pending, true); assert.equal(core.state.status, ''); assert.equal(core.state.is_read, true);
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

test('optimistic read counts survive refreshes and roll back only the failed message', async () => {
  const rows = [message, { ...message, id: 'mail-2' }].map(mail => ({ ...mail, thread_id: 't1', folder_id: 'inbox' }));
  const gates = new Map(); let writes = 0, lists = 0;
  const unread = () => rows.filter(mail => !mail.is_read).length;
  const model = InboxPageModel({ ...environment('thread/t1'), service: {
    async mailboxes() { return { accounts: [
      { id: 'a', folders: [{ id: 'inbox', unread_count: unread() }, { id: 'archive', unread_count: 3 }] },
      { id: 'b', folders: [{ id: 'other', unread_count: 4 }] },
    ], counts: { total: 9, unread: unread() + 7 } }; },
    async threads() { lists++; return { threads: [{ id: 't1', unread_count: unread() }], more: false }; },
    async detail() { return { thread: { id: 't1', unread_count: unread() }, messages: rows.map(mail => ({ ...mail })), more: false }; },
    async setRead(id, read) {
      writes++; const gate = deferred(); gates.set(id, gate); await gate.promise;
      rows.find(mail => mail.id === id).is_read = read;
      return { id, is_read: read, thread_unread: unread() };
    },
  } });
  await model.methods.init(); model.state.account.as('a');
  const [first, second] = model.state.messages.value;
  function check(count) {
    assert.equal(model.ui.unread.state.count, count + 7);
    assert.equal(model.ui.all.state.count, 9);
    assert.equal(model.state.detail.value.unread_count, count);
    assert.equal(model.state.threads.value[0].unread_count, count);
    assert.equal(model.methods.mailbox('a').state.folders[0].unread_count, count);
    assert.equal(model.state.accounts.value[0].folders[0].unread_count, count);
    assert.equal(model.state.folders.value[0].unread_count, count);
    assert.equal(model.methods.mailbox('a').state.folders[1].unread_count, 3);
    assert.equal(model.methods.mailbox('b').state.folders[0].unread_count, 4);
  }
  const one = first.setRead(true); check(1); assert.equal(first.state.is_read, true);
  await first.setRead(true); assert.equal(writes, 1);
  const two = second.loaded(true); check(0); assert.equal(second.state.is_read, true);
  await model.methods.refresh({ silent: true }); check(0);
  const before = lists;
  gates.get('mail-1').resolve(); await one;
  check(0); assert.equal(lists, before); // The first server count must not overwrite the second optimistic write.
  gates.get('mail-2').reject(new Error('连接失败')); await two;
  check(1); assert.equal(first.state.is_read, true); assert.equal(second.state.is_read, false);
  assert.equal(second.state.error, '操作失败：连接失败');
  assert.match(model.ui.notifications.state.entries[0].title, /操作失败/);
  const markUnread = first.setRead(false); check(2); assert.equal(first.state.is_read, false);
  gates.get('mail-1').reject(new Error('连接失败')); await markUnread;
  check(1); assert.equal(first.state.is_read, true);
  const retry = second.setRead(true); check(0); assert.equal(second.state.error, '');
  gates.get('mail-2').resolve(); await retry; check(0);
  model.methods.destroy();
});

test('a cancelled read restores counts after leaving the reader without changing another conversation', async () => {
  const gate = deferred();
  const model = InboxPageModel({ ...environment('thread/t1'), service: {
    async mailboxes() { return { accounts: [{ id: 'a', folders: [{ id: 'inbox', unread_count: 1 }] }], counts: { total: 1, unread: 1 } }; },
    async threads() { return { threads: [{ id: 't1', unread_count: 1 }], more: false }; },
    async detail() { return { thread: { id: 't1', unread_count: 1 }, messages: [{ ...message, thread_id: 't1', folder_id: 'inbox' }], more: false }; },
    setRead(id, read, signal) {
      signal.addEventListener('abort', () => gate.reject(new DOMException('Aborted', 'AbortError')));
      return gate.promise;
    },
  } });
  await model.methods.init();
  const write = model.state.messages.value[0].setRead(true);
  assert.equal(model.ui.unread.state.count, 0);
  model.methods.close(); model.state.detail.as({ id: 'other', unread_count: 5 });
  await write;
  assert.equal(model.ui.unread.state.count, 1);
  assert.equal(model.methods.mailbox('a').state.folders[0].unread_count, 1);
  assert.equal(model.state.detail.value.unread_count, 5);
  assert.equal(model.ui.notifications.state.entries.length, 0);
  model.methods.destroy();
});
