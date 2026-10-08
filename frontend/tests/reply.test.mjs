import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import { ReplyCore, MAX_ATTACHMENT_BYTES } from '../packages/jxui/reply-core.js';
import { ReplyWindow } from '../packages/jxui/reply-window.js';
import { InboxPageModel } from '../src/pages/inbox/index.model.js';

for (const file of ['timeless.lite.umd.min.js', 'timeless.dom.umd.min.js']) vm.runInThisContext(fs.readFileSync(new URL(`../vendor/${file}`, import.meta.url), 'utf8'));
Object.assign(globalThis, Timeless);
const deferred = () => { let resolve; const promise = new Promise(r => { resolve = r; }); return { promise, resolve }; };
const prepared = { from: 'owner@example.com', to: [{ email: 'sender@example.com', name: 'Sender' }], cc: [], subject: 'Re: Meeting' };
const service = extra => ({ prepareReply: async () => prepared, contacts: async () => ({ contacts: [] }), sendReply: async () => ({ ok: true }), ...extra });
const timers = { setTimeout() { return 1; }, clearTimeout() {} };
const find = (node, name) => node?.state?.attributes?.n === name ? node : node?.children?.map(child => find(child, name)).find(Boolean);

test('reply inherits subject in its title, exposes explicit subject editing and preserves drafts across window sizes', async () => {
  const core = ReplyCore({ timers, service: service() }); await core.open('mail-1');
  const view = ReplyWindow({ store: core });
  assert.equal(find(view, 'reply-window-title').state.attributes.title, 'Re: Meeting');
  assert.equal(find(view, 'reply-subject-field'), undefined);
  core.edit('body', 'Thanks'); find(view, 'reply-subject-toggle').events.onClick({});
  assert.ok(find(view, 'reply-subject-field')); core.edit('subject', 'Re: Updated meeting');
  assert.equal(find(view, 'reply-window-title').state.attributes.title, 'Re: Updated meeting');
  find(view, 'reply-window-maximize').events.onClick({});
  assert.equal(core.state.maximized, true); assert.equal(core.state.position, null);
  find(view, 'reply-window-minimize').events.onClick({}); assert.equal(core.state.minimized, true);
  find(view, 'reply-window-minimize').events.onClick({}); assert.equal(core.state.minimized, false);
  assert.equal(core.state.body, 'Thanks'); assert.equal(core.state.subject, 'Re: Updated meeting');
  find(view, 'reply-window-maximize').events.onClick({}); assert.equal(core.state.maximized, false);
  find(view, 'reply-window').onUnmounted(); view.onUnmounted(); core.destroy();
});

test('recipient search ignores stale results; keyboard choice, manual entry and deduplication feed To/Cc', async () => {
  const old = deferred(); let count = 0;
  const contact = { name: 'Alice', email: 'alice@example.com' };
  const core = ReplyCore({ timers, service: service({ contacts: async () => ++count === 1 ? old.promise : { contacts: [contact] } }) });
  await core.open('mail-1');
  core.inputRecipient('to', 'stale'); const pending = core.searchContacts('to');
  core.inputRecipient('to', 'alice'); await core.searchContacts('to');
  old.resolve({ contacts: [{ name: 'Old', email: 'old@example.com' }] }); await pending;
  assert.deepEqual(core.state.contacts, [contact]);
  core.recipientKeydown('to', { key: 'ArrowDown', preventDefault() {} });
  core.recipientKeydown('to', { key: 'Enter', preventDefault() {} });
  assert.equal(core.state.to[1].email, contact.email); assert.equal(core.state.queries.to, '');
  core.inputRecipient('cc', 'copy@example.com, COPY@example.com; other@example.com');
  assert.equal(core.commitRecipients('cc'), true); assert.equal(core.state.cc.length, 2);
  core.inputRecipient('to', 'not an email'); assert.equal(core.commitRecipients('to'), false);
  assert.match(core.state.error, /完整邮箱/);
  core.inputRecipient('to', ''); core.removeRecipient('to', 0);
  assert.deepEqual(core.state.to, [contact]); core.destroy();
});

test('attachments reserve limits while reading, support removal and preserve errors', async () => {
  const pending = deferred(); const core = ReplyCore({ timers, service: service(), readFile: () => pending.promise });
  await core.open('mail-1');
  const reading = core.addFiles([{ name: 'note.txt', size: 10 }]);
  assert.equal(core.state.attachments[0].status, 'reading');
  await core.addFiles([{ name: 'oversized.bin', size: MAX_ATTACHMENT_BYTES }]);
  assert.equal(core.state.attachments.length, 1); assert.match(core.state.error, /20 MB/);
  await core.send(); assert.equal(core.state.sending, false);
  core.removeFile(core.state.attachments[0].id); pending.resolve('aGk='); await reading;
  assert.equal(core.state.attachments.length, 0); core.destroy();
  const failed = ReplyCore({ service: service(), readFile: async () => { throw new Error('disk'); } });
  await failed.open('mail-1'); await failed.addFiles([{ name: 'note.txt', size: 2 }]);
  assert.equal(failed.state.attachments[0].status, 'failed'); failed.destroy();
});

test('native file bytes encode completely across chunks, and discarded reads cannot alter a new reply', async () => {
  const bytes = Uint8Array.from({ length: 20000 }, (_, i) => i % 256);
  const core = ReplyCore({ service: service() }); await core.open('mail-1');
  await core.addFiles([{ name: 'binary.bin', size: bytes.length, arrayBuffer: async () => bytes.buffer }]);
  assert.deepEqual(Buffer.from(core.state.attachments[0].data, 'base64'), Buffer.from(bytes)); core.destroy();
  const gate = deferred(); const changing = ReplyCore({ service: service(), readFile: () => gate.promise });
  await changing.open('mail-1'); const reading = changing.addFiles([{ name: 'old.txt', size: 2 }]);
  changing.discard(); await changing.open('mail-2'); gate.resolve('aGk='); await reading;
  assert.equal(changing.state.attachments.length, 0); changing.destroy();
});

test('floating composer sends edited recipients, Cc and bytes once and preserves uncertain results', async () => {
  const gate = deferred(); const calls = []; let refreshed = 0;
  const core = ReplyCore({ service: service({ sendReply: (id, payload) => { calls.push({ id, payload }); return gate.promise; } }),
    readFile: async () => 'aGk=', replied() { refreshed++; } });
  await core.open('mail-1'); core.removeRecipient('to', 0); core.inputRecipient('to', 'edited@example.com'); core.commitRecipients('to');
  core.inputRecipient('cc', 'copy@example.com'); core.commitRecipients('cc');
  core.edit('body', 'Thanks'); await core.addFiles([{ name: 'note.txt', size: 2 }]);
  const view = ReplyWindow({ store: core }); assert.equal(find(view, 'reply-window').state.as, 'dialog');
  assert.equal(find(view, 'reply-to-input').state.attributes.role, 'combobox');
  assert.equal(find(view, 'reply-file-input').state.attributes.multiple, '');
  const listeners = new Map(), target = { value: '', addEventListener: (name, listener) => listeners.set(name, listener), removeEventListener: name => listeners.delete(name) };
  const body = find(view, 'reply-body-input'); body.onMounted({ target: { get$elm: () => target } }); assert.equal(target.value, 'Thanks');
  listeners.get('input')({ target: { value: 'Edited body' } }); assert.equal(core.state.body, 'Edited body');
  const sending = core.send(); await core.send(); core.edit('body', 'Overwrite');
  assert.equal(calls.length, 1); assert.equal(core.state.body, 'Edited body');
  assert.equal(find(view, 'reply-send').state.attributes.disabled, '');
  assert.deepEqual(calls[0], { id: 'mail-1', payload: { to: [{ name: null, email: 'edited@example.com' }], cc: [{ name: null, email: 'copy@example.com' }],
    subject: prepared.subject, body: 'Edited body', attachments: [{ filename: 'note.txt', data: 'aGk=' }] } });
  gate.resolve({ ok: true }); await sending; assert.equal(core.state.sent, true); assert.equal(refreshed, 1);
  body.onUnmounted(); assert.equal(listeners.size, 0); view.onUnmounted(); core.destroy();
  for (const status of [400, 502, undefined]) {
    let attempts = 0;
    const failed = ReplyCore({ service: service({ sendReply: async () => { attempts++; throw Object.assign(new Error('failed'), { status }); } }), readFile: async () => 'aGk=' });
    await failed.open('mail-1'); failed.edit('body', 'Retain'); await failed.addFiles([{ name: 'note.txt', size: 2 }]);
    await failed.send(); assert.equal(failed.state.body, 'Retain'); assert.equal(failed.state.attachments.length, 1);
    assert.equal(failed.state.locked, status !== 400); await failed.send(); assert.equal(attempts, status === 400 ? 2 : 1); failed.destroy();
  }
});

test('drag stays in viewport and closing a dirty composer requires discard', async () => {
  const core = ReplyCore({ service: service() }); await core.open('mail-1');
  const bounds = { left: 200, top: 100, width: 400, height: 300, viewportWidth: 1000, viewportHeight: 800 };
  core.startDrag({ ...bounds, x: 220, y: 120 }); core.dragTo(-1000, 2000);
  assert.deepEqual(core.state.position, { x: 0, y: 500 }); core.endDrag(); core.dragTo(0, 0);
  assert.deepEqual(core.state.position, { x: 0, y: 500 });
  core.fitWindow({ ...bounds, left: 900, top: 700, viewportWidth: 500, viewportHeight: 400 });
  assert.deepEqual(core.state.position, { x: 100, y: 100 });
  core.edit('body', 'Keep this'); core.requestClose(); assert.equal(core.state.open, true); assert.equal(core.state.discardPending, true);
  await core.open('mail-2'); assert.equal(core.state.messageId, 'mail-1'); assert.equal(core.state.body, 'Keep this');
  core.keepEditing(); assert.equal(core.state.discardPending, false); core.discard(); assert.equal(core.state.open, false); core.destroy();
});

test('recipient view resolves reactive row indexes and exposes keyboard selection through ARIA', async () => {
  const core = ReplyCore({ service: service(), timers }); await core.open('mail-1');
  let view = ReplyWindow({ store: core });
  find(view, 'reply-to-remove').events.onClick({}); assert.equal(core.state.to.length, 0);
  find(view, 'reply-window').onUnmounted(); view.onUnmounted();
  core.set({ recipientField: 'to', queries: { to: 'alice', cc: '' }, contacts: [{ name: 'Alice', email: 'alice@example.com' }], contactIndex: 0 });
  view = ReplyWindow({ store: core });
  assert.equal(find(view, 'reply-to-contact-option').state.attributes.id, 'reply-to-contact-0');
  assert.equal(find(view, 'reply-to-contact-option').state.attributes['aria-selected'], 'true');
  find(view, 'reply-to-select-contact').events.onClick({}); assert.equal(core.state.to[0].email, 'alice@example.com');
  find(view, 'reply-window').onUnmounted(); view.onUnmounted(); core.destroy();
});

test('page-owned reply survives closing its source message and mail navigation', async () => {
  const location = { href: 'http://127.0.0.1:8787/' }, history = { pushState() {}, replaceState() {} };
  const model = InboxPageModel({ root: '', location, history, timers, service: service({ mailboxes: async () => ({ accounts: [] }),
    threads: async () => ({ threads: [], more: false }), detail: async () => ({ messages: [{ id: 'mail-1' }], more: false }) }) });
  model.methods.select({ id: 'thread-1' }); await new Promise(r => setTimeout(r, 0));
  await model.state.messages.value[0].openReply(); model.ui.reply.edit('body', 'Keep across navigation'); model.methods.close();
  assert.equal(model.ui.reply.state.open, true); assert.equal(model.ui.reply.state.body, 'Keep across navigation'); model.methods.destroy();
});
