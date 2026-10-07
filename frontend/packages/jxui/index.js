import icons from "../../vendor/timeless-icons.js";

// All views use the real Timeless primitives exposed by the vendored runtime.
function binding(store) {
  const state = ref(store.state);
  const events = [store.onStateChange(() => state.as(store.state))];
  const derived = [];
  return { state, derive(fn) { const value = Timeless.computed(state, fn); derived.push(value); return value; },
    cleanup() { for (const off of events) off?.(); derived.forEach(value => value.destroy()); state.destroy(); } };
}
export function Button({ store, variant = 'secondary', icon, class: extra = '', ...rest }, children) {
  const b = binding(store);
  return View({ ...rest, as: 'button',
    class: `jx-button jx-button--${variant} ${extra}`,
    attributes: { type: 'button', disabled: b.derive( s => s.disabled ? '' : undefined),
      'aria-label': b.derive( s => s.label), title: b.derive(s => s.label), ...rest.attributes },
    onClick(e) { store.click(e); }, onUnmounted: b.cleanup,
  }, [icon && Glyph(icon), children ?? b.derive( s => s.label)]);
}
export function Input({ store, label, placeholder = '', ...rest }) {
  const b = binding(store);
  let off = () => {};
  return View({ ...rest, as: 'input', class: `jx-input ${rest.class || ''}`,
    attributes: { type: 'search', 'aria-label': label, placeholder,
      value: b.derive( s => s.value), disabled: b.derive( s => s.disabled ? '' : undefined) },
    onInput(e) { store.input(e.target.value); },
    onKeyDown(e) { store.keydown(e); },
    onMounted({ target }) {
      const apply = () => { if (target.value !== store.state.value) target.value = store.state.value; };
      apply(); off = store.onStateChange(apply);
    }, onUnmounted() { off(); b.cleanup(); },
  });
}
export function Badge({}, children) { return View({ as: 'span', class: 'jx-badge' }, children); }
export function Avatar({ name }) {
  const tones = ['blue', 'green', 'violet', 'amber'];
  const hash = Array.from(name || '').reduce((value, letter) => value + letter.codePointAt(0), 0);
  return View({ class: `jx-avatar jx-avatar--${tones[hash % tones.length]}`, attributes: { 'aria-hidden': 'true' } }, [Array.from(name || '?')[0].toUpperCase()]); }
const iconNames = {
  agent: 'bot', mail: 'inbox', all: 'file-stack', inbox: 'inbox', sent: 'upload', drafts: 'pen-line',
  archive: 'box', trash: 'trash-2', spam: 'circle-alert', custom: 'folder',
  unread: 'message-square-more', refresh: 'refresh-cw', back: 'arrow-left', close: 'x',
  search: 'search', menu: 'menu', next: 'chevron-right', prev: 'chevron-left',
  check: 'check', image: 'image', text: 'file-text', attachment: 'paperclip',
  notification: 'message-square-more', clock: 'clock-3', server: 'server', user: 'user', privacy: 'eye', loading: 'loader-circle',
};
let iconsRegistered = false;
export function Glyph(name) {
  // Register the selected official ASN definitions in the actual Timeless Icon registry.
  if (!iconsRegistered) { Timeless.registerIcons(icons); iconsRegistered = true; }
  return View({ as: 'span', class: `jx-glyph jx-glyph--${name}`, attributes: { 'aria-hidden': 'true' } },
    [Timeless.Icon({ name: iconNames[name] || 'folder', size: 20 })]);
}
export function NavItem({ store, icon = 'custom', count = 0, showCount = false }) {
  const b = binding(store);
  const countLabel = b.derive(s => String(s.count ?? count));
  const countVisible = b.derive(s => showCount || (s.count ?? count) > 0);
  return View({ as: 'button', class: b.derive( s => `jx-nav-item ${s.active ? 'is-active' : ''}`),
    attributes: { type: 'button', 'aria-current': b.derive( s => s.active ? 'page' : undefined) },
    onClick(e) { store.click(e); }, onUnmounted: b.cleanup,
  }, [Glyph(icon), View({ as: 'span', class: 'jx-truncate', attributes: { title: store.state.label } }, [b.derive( s => s.label)]), Show({ when: countVisible, ok: () => Badge({}, [countLabel]) })]);
}
export function Panel({ kind, id, ...props }, children) { return View({ ...props, attributes: { ...props.attributes, id }, class: `jx-panel jx-panel--${kind} ${props.class || ''}` }, children); }
export function Toolbar(props = {}, children) { return View({ ...props, class: `jx-toolbar ${props.class || ''}` }, children); }
export function EmptyState({ title, description, icon = 'mail' }, children = []) {
  return View({ class: 'jx-empty', attributes: { role: 'status' } }, [Glyph(icon),
    View({ as: 'h2' }, [title]), View({ as: 'p' }, [description]), ...children]);
}
export function ConversationRow({ store, thread }) {
  const b = binding(store);
  return View({ as: 'a', class: b.derive( s => `jx-conversation ${s.active ? 'is-selected' : ''} ${thread.unread_count ? 'is-unread' : ''}`),
    attributes: { href: store.state.href, 'aria-current': b.derive( s => s.active ? 'true' : undefined), title: thread.subject },
    onClick(e) { if (!e.metaKey && !e.ctrlKey && !e.shiftKey && e.button === 0) { e.preventDefault(); store.click(e); } },
    onUnmounted: b.cleanup,
  }, [Avatar({ name: thread.sender || thread.account_email }), View({ as: 'span', class: 'jx-unread-dot', attributes: { 'aria-label': thread.unread_count ? '未读' : '已读' } }),
    View({ class: 'jx-conversation-content' }, [
      View({ class: 'jx-row' }, [View({ as: 'span', class: 'jx-sender jx-truncate' }, [thread.sender || thread.account_email]),
        thread.message_count > 1 && Badge({}, [String(thread.message_count)]),
        RelativeTime({ timestamp: thread.last_message_at })]),
      View({ class: 'jx-subject jx-truncate' }, [thread.subject || '（无主题）']),
      View({ class: 'jx-snippet jx-truncate' }, [thread.snippet || '（无邮件摘要）']),
      View({ class: 'jx-conversation-account jx-truncate' }, [Glyph('mail'), thread.account_email]),
    ])]);
}
export function formatDate(ts, now = Date.now()) {
  if (!Number.isFinite(ts) || ts <= 0 || !Number.isFinite(new Date(ts * 1000).getTime())) return '未知时间';
  const seconds = Math.floor((now - ts * 1000) / 1000);
  if (Math.abs(seconds) < 60) return '刚刚';
  const formatter = new Intl.RelativeTimeFormat('zh-CN', { numeric: 'auto' });
  const elapsed = Math.abs(seconds);
  const units = [[31536000, 'year'], [2592000, 'month'], [86400, 'day'], [3600, 'hour'], [60, 'minute']];
  const [size, unit] = units.find(([size]) => elapsed >= size);
  return formatter.format((seconds < 0 ? 1 : -1) * Math.floor(elapsed / size), unit);
}

export function RelativeTime({ timestamp }) {
  const label = ref(formatDate(timestamp));
  const date = new Date(timestamp * 1000);
  const valid = Number.isFinite(timestamp) && timestamp > 0 && Number.isFinite(date.getTime());
  let timer;
  return View({ as: 'time', class: 'jx-date', attributes: {
    title: valid ? date.toLocaleString('zh-CN') : '未知时间',
    datetime: valid ? date.toISOString() : undefined,
  }, onMounted() {
    label.as(formatDate(timestamp));
    timer = setInterval(() => label.as(formatDate(timestamp)), 30000);
  }, onUnmounted() { clearInterval(timer); label.destroy(); } }, [label]);
}

export function Select({ store, label, options, id }) {
  const b = binding(store);
  let offState = () => {}, offOptions = () => {};
  return View({ as: 'select', class: 'jx-select', attributes: { id, 'aria-label': label },
    onChange(e) { store.input(e.target.value); },
    onMounted({ target }) {
      const apply = () => { target.value = store.state.value; };
      apply(); offState = store.onStateChange(apply);
      offOptions = options.subscribe({ onChange() { queueMicrotask(apply); } });
    }, onUnmounted() { offState(); offOptions(); b.cleanup(); },
  }, [View({ as: 'option', attributes: { value: '' } }, ['所有邮箱']),
    For({ each: options, render(option) { return View({ as: 'option', attributes: { value: option.id } }, [option.email]); } }),
  ]);
}

/** A mailbox is always visible; collapsing only hides its own folder children. */
export function MailboxGroup({ store }, folders) {
  const b = binding(store);
  const expanded = b.derive(s => s.expanded);
  const email = b.derive(s => s.email);
  const status = b.derive(s => s.status?.label || '未同步');
  const items = b.derive(s => s.folders || []);
  return View({ as: 'section', class: 'jx-mailbox-group',
    attributes: { 'aria-label': email }, onUnmounted: b.cleanup,
  }, [View({ class: 'jx-mailbox-header' }, [
    View({ as: 'button', class: b.derive(s => `jx-mailbox-toggle ${s.expanded ? 'is-open' : ''}`),
      attributes: { type: 'button', 'aria-label': b.derive(s => `${s.expanded ? '折叠' : '展开'} ${s.email} 的文件夹`),
        'aria-expanded': b.derive(s => String(s.expanded)), 'aria-controls': `folders-${store.state.id}` },
      onClick() { store.toggle(); },
    }, [Glyph('next')]),
    View({ as: 'button', class: b.derive(s => `jx-mailbox-identity ${s.active ? 'is-active' : ''}`),
      attributes: { type: 'button', title: email, 'aria-current': b.derive(s => s.active ? 'true' : undefined) },
      onClick(e) { store.click(e); },
    }, [View({ class: 'jx-mailbox-name' }, [Glyph('mail'), View({ as: 'strong', class: 'jx-truncate' }, [email])]),
      View({ class: b.derive(s => `jx-mailbox-status jx-mailbox-status--${s.status?.tone || 'warning'}`),
        attributes: { role: 'status', 'aria-live': 'polite' },
      }, [View({ as: 'span', class: 'jx-status-dot', attributes: { 'aria-hidden': 'true' } }), status]),
    ]),
  ]), Show({ when: expanded, ok: () => View({ class: 'jx-mailbox-folders',
    attributes: { id: `folders-${store.state.id}` },
  }, [For({ each: items, render(folder) { return folders(folder, store.state.id); } })]) })]);
}

/** Non-modal activity history with explicit per-item read/unread controls. */
export function NotificationCenter({ store }) {
  const b = binding(store);
  const opened = b.derive(s => s.open);
  const entries = b.derive(s => s.entries);
  const hasUnread = b.derive(s => s.unread > 0);
  const unreadLabel = b.derive(s => String(s.unread));
  const readAllDisabled = b.derive(s => s.unread ? undefined : '');
  const empty = b.derive(s => !s.entries.length);
  let detach = () => {};
  return View({ class: 'jx-notification-center', onMounted({ target }) {
    function outside(event) { if (!target.contains(event.target)) store.close(); }
    function escape(event) { if (event.key === 'Escape' && store.state.open) { store.close(); target.querySelector('button')?.focus(); } }
    document.addEventListener('pointerdown', outside); document.addEventListener('keydown', escape);
    detach = () => { document.removeEventListener('pointerdown', outside); document.removeEventListener('keydown', escape); };
  }, onUnmounted() { detach(); b.cleanup(); } }, [
    View({ as: 'button', class: b.derive(s => `jx-button jx-button--ghost jx-notification-trigger ${s.unread ? 'has-unread' : ''}`), attributes: {
      type: 'button', 'aria-label': b.derive(s => `通知中心，${s.unread} 条未读`), 'aria-expanded': b.derive(s => String(s.open)), 'aria-controls': 'notification-history', title: '通知中心',
    }, onClick() { store.toggle(); } }, [Glyph('notification'), Show({ when: hasUnread, ok: () => Badge({}, [unreadLabel]) })]),
    Show({ when: opened, ok: () => View({ as: 'section', class: 'jx-notification-panel', attributes: { id: 'notification-history', 'aria-label': '通知中心' } }, [
      View({ class: 'jx-notification-heading' }, [View({ as: 'strong' }, ['通知中心']),
        View({ as: 'button', class: 'jx-button jx-button--ghost', attributes: { type: 'button', disabled: readAllDisabled }, onClick() { store.readAll(); } }, [Glyph('check'), '全部已读']),
      ]),
      View({ class: 'jx-notification-list' }, [For({ each: entries, render(item) {
        return View({ as: 'article', class: `jx-notification-item ${item.read ? 'is-read' : 'is-unread'}` }, [
          View({ class: 'jx-notification-symbol' }, [Glyph('spam')]),
          View({ class: 'jx-notification-content' }, [View({ as: 'strong' }, [item.title]), View({ as: 'p' }, [item.message]),
            View({ class: 'jx-row' }, [RelativeTime({ timestamp: item.timestamp }), item.count > 1 && Badge({}, [`${item.count} 次`]),
              View({ as: 'button', class: 'jx-button jx-button--ghost', attributes: { type: 'button' }, onClick() { store.setRead(item.id, !item.read); } }, [item.read ? '标为未读' : '标为已读']),
            ]),
          ]),
        ]);
      } }), Show({ when: empty, ok: () => EmptyState({ icon: 'notification', title: '暂无通知', description: '邮箱状态和邮件操作的错误会记录在这里。' }) })]),
    ]) }),
  ]);
}
