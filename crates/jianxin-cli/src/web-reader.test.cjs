// Non-browser checks: a minimal DOM stub exercises our own reader logic.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');

function message(id, open) {
  const button = {disabled: false, events: {}, addEventListener(name, fn) {this.events[name] = fn;}};
  const status = {textContent: '', classList: {add() {}, remove() {}}};
  const state = {textContent: '未读'};
  const details = {id: `mail-${id}`, open, events: {}, addEventListener(name, fn) {this.events[name] = fn;}};
  const item = {
    dataset: {read: '0', readUrl: `/secret/flags/${id}`},
    querySelector(selector) {return {'[data-read-button]': button, '[data-read-state]': state, '[data-read-status]': status}[selector];}
  };
  const content = {scrollHeight: 900, getBoundingClientRect() {return {height: this.scrollHeight};}};
  const frame = {
    loaded: open, dataset: {src: `/secret/body/${id}`}, style: {}, events: {},
    closest(selector) {return selector === 'details' ? details : item;},
    hasAttribute() {return this.loaded;},
    addEventListener(name, fn) {this.events[name] = fn;},
    set src(value) {this.loaded = true; this.events.load();}
  };
  frame.contentDocument = {readyState: 'complete', getElementById() {return frame.loaded ? content : null;}, addEventListener() {}};
  return {id, frame, details, item, button, status, state, content};
}

async function run() {
  const first = message('first', true);
  const second = message('second', false);
  const messages = [first, second];
  const writes = [];
  const read = new Set();
  const count = {textContent: 2};
  let fail = false;
  const context = {
    document: {hidden: false, querySelectorAll: () => messages.map(m => m.frame), querySelector: () => count, addEventListener() {}},
    location: {hash: ''}, window: {addEventListener() {}},
    requestAnimationFrame: fn => fn(),
    ResizeObserver: class {observe() {} disconnect() {}},
    fetch: async (url, options) => {
      assert.equal(options.method, 'POST');
      assert.equal(options.headers['X-Jianxin-Request'], '1');
      writes.push(url);
      if (fail) return {ok: false, json: async () => ({error: 'network unavailable'})};
      const parsed = new URL(url, 'http://localhost');
      const id = parsed.pathname.split('/').at(-1);
      const target = parsed.searchParams.get('read') === '1';
      if (target) read.add(id); else read.delete(id);
      return {ok: true, json: async () => ({is_read: target, thread_unread: 2 - read.size})};
    }
  };
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, 'web-reader.js'), 'utf8'), context);
  const settle = () => new Promise(resolve => setImmediate(resolve));
  await settle();
  assert.equal(writes.length, 1);
  assert.equal(first.item.dataset.read, '1');
  assert.equal(second.frame.loaded, false);
  assert.equal(second.item.dataset.read, '0');
  assert.equal(count.textContent, 1);
  first.button.events.click();
  await settle();
  assert.equal(first.item.dataset.read, '0');
  first.frame.events.load();
  await settle();
  assert.equal(writes.length, 2, 'manual unread must not be overwritten automatically');
  second.details.open = true;
  second.details.events.toggle();
  await settle();
  assert.equal(second.frame.loaded, true);
  assert.equal(second.item.dataset.read, '1');
  fail = true;
  first.button.events.click();
  await settle();
  assert.equal(first.item.dataset.read, '0', 'failed remote write preserves previous state');
  assert.match(first.status.textContent, /同步失败/);
  assert.equal(first.button.disabled, false);
  first.content.scrollHeight = 1800;
  first.frame.events.load();
  assert.equal(first.frame.style.height, '1832px', 'image loading still resizes without inner scrolling');
  console.log('Reader auto-read, collapsed replies, manual unread, failure and resize checks passed');
}
run().catch(error => {console.error(error); process.exitCode = 1;});
