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
    onMounted({ target: host }) {
      const target = host.get$elm?.() || host;
      const apply = () => { if (target.value !== store.state.value) target.value = store.state.value; };
      const input = event => store.input(event.target.value);
      const keydown = event => store.keydown(event);
      apply(); off = store.onStateChange(apply);
      // The vendored View runtime does not forward native input events.
      target.addEventListener('input', input);
      target.addEventListener('keydown', keydown);
      const stop = off;
      off = () => { stop(); target.removeEventListener('input', input); target.removeEventListener('keydown', keydown); };
    }, onUnmounted() { off(); b.cleanup(); },
  });
}
export function SidebarMoreMenu({ store, reload, root = '' }) {
  const b = binding(store), refresh = binding(reload);
  const refreshLabel = refresh.derive(s => s.label), refreshDisabled = refresh.derive(s => s.disabled ? '' : undefined);
  let target, detach = () => {};
  const dismiss = () => { target?.querySelector('[data-n="sidebar-more-toggle"]')?.focus(); store.close(); };
  return View({ class: 'jx-sidebar-more', attributes: { n: 'sidebar-more' },
    onMounted({ target: host }) {
      target = host.get$elm?.() || host;
      const outside = event => { if (!target.contains(event.target)) store.close(); };
      const keydown = event => {
        if (!store.state.open) return;
        if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); dismiss(); }
        if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) {
          const items = Array.from(target.querySelectorAll('.jx-mail-menu-action')).filter(item => !item.disabled);
          if (!items.length) return;
          event.preventDefault();
          const index = items.indexOf(document.activeElement);
          const next = event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1
            : index < 0 ? (event.key === 'ArrowDown' ? 0 : items.length - 1)
              : (index + (event.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length;
          items[next].focus();
        }
      };
      document.addEventListener('pointerdown', outside); document.addEventListener('focusin', outside);
      target.addEventListener('keydown', keydown);
      detach = () => { document.removeEventListener('pointerdown', outside); document.removeEventListener('focusin', outside); target.removeEventListener('keydown', keydown); };
    }, onUnmounted() { detach(); b.cleanup(); refresh.cleanup(); } }, [
    Button({ store, variant: 'ghost', icon: 'more', attributes: { n: 'sidebar-more-toggle',
      'aria-expanded': b.derive(s => String(s.open)), 'aria-controls': 'sidebar-more-panel' } }),
    Show({ when: b.derive(s => s.open), ok: () => View({ as: 'nav', class: 'jx-mail-menu jx-sidebar-more-menu',
      attributes: { n: 'sidebar-more-panel', id: 'sidebar-more-panel', 'aria-label': '更多邮箱操作' },
      onMounted({ target: host }) { (host.get$elm?.() || host).querySelector('button:not(:disabled), a')?.focus(); } }, [
      View({ as: 'button', class: 'jx-mail-menu-action', attributes: { n: 'refresh-mailbox-config', type: 'button',
        disabled: refreshDisabled, 'aria-label': refreshLabel },
        onClick(event) { dismiss(); reload.click(event); } }, [Glyph('refresh'), refreshLabel]),
      View({ as: 'a', class: 'jx-mail-menu-action', attributes: { n: 'sync-logs-navigation', href: `${root}/logs` },
        onClick: dismiss }, [Glyph('clock'), '同步日志']),
    ]) }),
  ]);
}
export function Badge({}, children) { return View({ as: 'span', class: 'jx-badge', attributes: { n: 'count-badge' } }, children); }
export function Avatar({ name }) {
  const tones = ['blue', 'green', 'violet', 'amber'];
  const hash = Array.from(name || '').reduce((value, letter) => value + letter.codePointAt(0), 0);
  return View({ class: `jx-avatar jx-avatar--${tones[hash % tones.length]}`, attributes: { n: 'sender-avatar', 'aria-hidden': 'true' } }, [Array.from(name || '?')[0].toUpperCase()]); }
const iconNames = {
  agent: 'bot', mail: 'inbox', all: 'file-stack', inbox: 'inbox', sent: 'upload', drafts: 'pen-line',
  archive: 'box', trash: 'trash-2', spam: 'circle-alert', custom: 'folder',
  unread: 'message-square-more', starred: 'star', refresh: 'refresh-cw', back: 'arrow-left', close: 'x',
  search: 'search', menu: 'menu', next: 'chevron-right', prev: 'chevron-left',
  check: 'check', image: 'image', text: 'file-text', attachment: 'paperclip',
  notification: 'bell', clock: 'clock-3', server: 'server', user: 'user', privacy: 'eye', loading: 'loader-circle',
  more: 'ellipsis-vertical', reply: 'undo-2', expand: 'maximize',
};
let iconsRegistered = false;
export function Glyph(name) {
  // Register the selected official ASN definitions in the actual Timeless Icon registry.
  if (!iconsRegistered) { Timeless.registerIcons(icons); iconsRegistered = true; }
  return View({ as: 'span', class: `jx-glyph jx-glyph--${name}`, attributes: { n: `icon-${name}`, 'aria-hidden': 'true' } },
    [Timeless.Icon({ name: iconNames[name] || 'folder', size: 20 })]);
}
export function NavItem({ store, icon = 'custom', count = 0, showCount = false }) {
  const b = binding(store);
  const countLabel = b.derive(s => String(s.count ?? count));
  const countVisible = b.derive(s => showCount || (s.count ?? count) > 0);
  return View({ as: 'button', class: b.derive( s => `jx-nav-item ${s.active ? 'is-active' : ''}`),
    attributes: { n: `mail-navigation-${icon}-button`, type: 'button', 'aria-current': b.derive( s => s.active ? 'page' : undefined) },
    onClick(e) { store.click(e); }, onUnmounted: b.cleanup,
  }, [Glyph(icon), View({ as: 'span', class: 'jx-truncate', attributes: { n: 'mail-navigation-label', title: store.state.label } }, [b.derive( s => s.label)]), Show({ when: countVisible, ok: () => Badge({}, [countLabel]) })]);
}
export function Panel({ kind, id, ...props }, children) { return View({ ...props, attributes: { ...props.attributes, id }, class: `jx-panel jx-panel--${kind} ${props.class || ''}` }, children); }
export function Toolbar(props = {}, children) { return View({ ...props, class: `jx-toolbar ${props.class || ''}` }, children); }
export function EmptyState({ title, description, icon = 'mail' }, children = []) {
  return View({ class: 'jx-empty', attributes: { role: 'status' } }, [Glyph(icon),
    View({ as: 'h2' }, [title]), View({ as: 'p' }, [description]), ...children]);
}
export function ConversationRow({ store, thread, mailbox, root = '', favorite = false }) {
  const b = binding(store);
  const account = mailbox ? binding(mailbox) : b;
  const platformIcon = account.derive(s => s.platformIcon || '');
  const link = View({ as: 'a', class: b.derive( s => `jx-conversation ${s.active ? 'is-selected' : ''} ${thread.unread_count ? 'is-unread' : ''}`),
    attributes: { n: 'conversation-link', href: store.state.href, 'aria-current': b.derive( s => s.active ? 'true' : undefined), title: thread.subject },
    onClick(e) { if (!e.metaKey && !e.ctrlKey && !e.shiftKey && e.button === 0) { e.preventDefault(); store.click(e); } },
    onUnmounted() { if (!favorite) { if (mailbox) account.cleanup(); b.cleanup(); } },
  }, [Avatar({ name: thread.sender || thread.account_email }), View({ as: 'span', class: 'jx-unread-dot', attributes: { n: 'conversation-unread-indicator', 'aria-label': thread.unread_count ? '未读' : '已读' } }),
    View({ class: 'jx-conversation-content', attributes: { n: 'conversation-summary' } }, [
      View({ class: 'jx-row', attributes: { n: 'conversation-sender-row' } }, [View({ as: 'span', class: 'jx-sender jx-truncate', attributes: { n: 'conversation-sender' } }, [thread.sender || thread.account_email]),
        thread.message_count > 1 ? Badge({}, [String(thread.message_count)]) : undefined,
        RelativeTime({ timestamp: thread.last_message_at })]),
      View({ class: 'jx-subject jx-truncate', attributes: { n: 'conversation-subject' } }, [thread.subject || '（无主题）']),
      View({ class: 'jx-snippet jx-truncate', attributes: { n: 'conversation-snippet' } }, [thread.snippet || '（无邮件摘要）']),
      View({ class: 'jx-conversation-account jx-truncate', attributes: { n: 'conversation-mailbox' } }, [
        Show({ when: platformIcon, ok: () => View({ as: 'img', class: 'jx-conversation-favicon', attributes: {
          n: 'conversation-mailbox-favicon', src: account.derive(s => `${root}/assets/mail-providers/${s.platformIcon}.svg`), alt: '', 'aria-hidden': 'true',
        } }) }),
        Show({ when: account.derive(s => !s.platformIcon), ok: () => Glyph('mail') }),
        View({ as: 'span', class: 'jx-truncate', attributes: { n: 'conversation-mailbox-email', title: thread.account_email } }, [thread.account_email]),
      ]),
    ])]);
  if (!favorite) return link;
  let detach = () => {};
  return View({ class: 'jx-conversation-card', attributes: { n: 'favorite-conversation-card' },
    onMounted({ target: host }) {
      const target = host.get$elm?.() || host;
      const outside = event => { if (!target.querySelector('[data-n="favorite-more-controls"]')?.contains(event.target)) store.closeMenu(); };
      document.addEventListener('pointerdown', outside); detach = () => document.removeEventListener('pointerdown', outside);
    }, onKeyDown(event) {
      if (event.key === 'Escape' && store.state.menuOpen) { event.preventDefault(); event.stopPropagation(); store.closeMenu(); event.currentTarget.querySelector('[data-n="favorite-more-toggle"]')?.focus(); }
    }, onUnmounted() { detach(); if (mailbox) account.cleanup(); b.cleanup(); } }, [
    link,
    View({ class: 'jx-favorite-more', attributes: { n: 'favorite-more-controls' } }, [
      View({ as: 'button', class: 'jx-button jx-button--ghost jx-icon-button', attributes: { n: 'favorite-more-toggle', type: 'button', title: '更多', 'aria-label': `更多操作：${thread.subject || '无主题'}`,
        'aria-expanded': b.derive(s => String(s.menuOpen)), 'aria-controls': `favorite-more-${thread.id}` }, onClick() { store.toggleMenu(); } }, [Glyph('more')]),
      Show({ when: b.derive(s => s.menuOpen), ok: () => View({ as: 'nav', class: 'jx-mail-menu', attributes: { n: 'favorite-more-panel', id: `favorite-more-${thread.id}`, 'aria-label': '收藏操作' },
        onMounted({ target: host }) { (host.get$elm?.() || host).querySelector('button')?.focus(); } }, [
        View({ as: 'button', class: 'jx-mail-menu-action', attributes: { n: 'favorite-unstar', type: 'button', disabled: b.derive(s => s.pending ? '' : undefined) },
          onClick(event) { event.currentTarget.closest('[data-n="favorite-more-controls"]').querySelector('[data-n="favorite-more-toggle"]').focus(); store.unstar(); } }, [Glyph('starred'), b.derive(s => s.pending ? '正在取消收藏…' : '取消收藏')]),
      ]) }),
    ]),
    Show({ when: b.derive(s => !!s.error || s.pending), ok: () => View({ as: 'p', class: b.derive(s => `jx-favorite-status ${s.error ? 'jx-error' : 'jx-muted'}`), attributes: { n: 'favorite-unstar-status', role: 'status', 'aria-live': 'polite' } }, [b.derive(s => s.error || '正在取消收藏…')]) }),
  ]);
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
    n: 'relative-time', title: valid ? date.toLocaleString('zh-CN') : '未知时间',
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
export function MailboxGroup({ store, root = '' }, folders) {
  const b = binding(store);
  const expanded = b.derive(s => s.expanded);
  const email = b.derive(s => s.email);
  const problem = b.derive(s => s.problem);
  const opened = b.derive(s => s.statusOpen);
  const icon = b.derive(s => s.platformIcon);
  const items = b.derive(s => s.folders || []);
  const menuOpen = b.derive(s => s.menuOpen);
  const refreshing = b.derive(s => s.refreshBusy);
  const refreshError = b.derive(s => s.refreshError);
  const refreshLabel = b.derive(s => s.refreshBusy ? '正在刷新…' : '刷新');
  const refreshDisabled = b.derive(s => s.refreshBusy ? '' : undefined);
  function oauthField(field, label) {
    return View({ as: 'label', class: 'jx-import-field' }, [View({ as: 'span' }, [label]),
      View({ as: 'input', class: 'jx-input', attributes: { n: `mailbox-oauth-${field}`, type: field === 'token_endpoint' ? 'url' : 'text',
        disabled: b.derive(s => s.oauth.busy ? '' : undefined), autocomplete: 'off', maxlength: field === 'client_id' ? '256' : '4096' },
        onMounted({ target: host }) {
          const target = host.get$elm?.() || host;
          const apply = () => { if (target.value !== store.state.oauthDraft[field]) target.value = store.state.oauthDraft[field]; };
          const input = event => store.editOAuth(field, event.target.value);
          apply(); const stop = store.onStateChange(apply); target.addEventListener('input', input);
          return () => { stop(); target.removeEventListener('input', input); };
        } }),
    ]);
  }
  let detach = () => {};
  return View({ as: 'section', class: 'jx-mailbox-group',
    attributes: { n: 'mailbox-group', 'aria-label': email },
    onMounted({ target: host }) {
      const target = host.get$elm?.() || host;
      function outside(event) {
        if (!target.querySelector('.jx-mailbox-icon')?.contains(event.target) && !target.querySelector('.jx-mailbox-status-popover')?.contains(event.target)) store.closeStatus();
        if (!target.querySelector('[data-n="mailbox-more-controls"]')?.contains?.(event.target)) store.closeMenu();
      }
      function escape(event) {
        if (event.key === 'Escape' && store.state.menuOpen) {
          event.preventDefault(); store.closeMenu(); target.querySelector('[data-n="mailbox-more-toggle"]')?.focus();
        }
        if (event.key === 'Escape' && store.state.statusOpen) {
          store.closeStatus(); target.querySelector('[data-n="mailbox-status-trigger"]')?.focus();
        }
      }
      const blur = event => { if (!target.querySelector('[data-n="mailbox-more-controls"]')?.contains?.(event.target)) store.closeMenu(); };
      document.addEventListener('keydown', escape); document.addEventListener('pointerdown', outside);
      document.addEventListener('focusin', blur);
      detach = () => { document.removeEventListener('keydown', escape); document.removeEventListener('pointerdown', outside); document.removeEventListener('focusin', blur); };
    }, onUnmounted() { detach(); b.cleanup(); },
  }, [View({ class: 'jx-mailbox-header', attributes: { n: 'mailbox-header' } }, [
    View({ as: 'button', class: b.derive(s => `jx-mailbox-toggle ${s.expanded ? 'is-open' : ''}`),
      attributes: { n: 'mailbox-folders-toggle', type: 'button', 'aria-label': b.derive(s => `${s.expanded ? '折叠' : '展开'} ${s.email} 的文件夹`),
        'aria-expanded': b.derive(s => String(s.expanded)), 'aria-controls': `folders-${store.state.id}` },
      onClick() { store.toggle(); },
    }, [Glyph('next')]),
    View({ class: 'jx-mailbox-name', attributes: { n: 'mailbox-name' } }, [
      View({ as: 'span', class: 'jx-mailbox-icon', attributes: { n: 'mailbox-platform-icon' } }, [
        Show({ when: icon, ok: () => View({ as: 'img', class: 'jx-mailbox-favicon', attributes: {
          n: 'mailbox-platform-favicon', src: b.derive(s => `${root}/assets/mail-providers/${s.platformIcon}.svg`), alt: '', 'aria-hidden': 'true',
        } }) }),
        Show({ when: b.derive(s => !s.platformIcon), ok: () => Glyph('mail') }),
        Show({ when: problem, ok: () => View({ as: 'button', class: 'jx-mailbox-status-trigger', attributes: {
          n: 'mailbox-status-trigger', type: 'button', 'aria-label': b.derive(s => `${s.email}：${s.problem}，查看详情`),
          'aria-expanded': b.derive(s => String(s.statusOpen)), 'aria-controls': `mailbox-status-${store.state.id}`,
        }, onClick() { store.toggleStatus(); } }, [View({ as: 'span',
          class: b.derive(s => `jx-mailbox-status-badge jx-mailbox-status-badge--${s.status?.tone}`),
          attributes: { n: 'mailbox-status-badge', 'aria-hidden': 'true' },
        }, ['!'])]) }),
      ]),
      View({ as: 'button', class: b.derive(s => `jx-mailbox-identity ${s.active ? 'is-active' : ''}`),
        attributes: { n: 'select-mailbox', type: 'button', title: email, 'aria-current': b.derive(s => s.active ? 'true' : undefined) },
        onClick(e) { store.closeStatus(); store.closeMenu(); store.click(e); },
      }, [View({ as: 'strong', class: 'jx-truncate', attributes: { n: 'mailbox-email' } }, [email])]),
    ]),
    View({ class: 'jx-mailbox-more', attributes: { n: 'mailbox-more-controls' }, onKeyDown(event) {
      if (['ArrowDown', 'ArrowUp'].includes(event.key)) {
        event.preventDefault();
        if (!store.state.menuOpen) store.toggleMenu();
        else event.currentTarget.querySelector('[data-n="refresh-mailbox"]:not(:disabled)')?.focus();
      }
    } }, [
      View({ as: 'button', class: 'jx-button jx-button--ghost jx-icon-button', attributes: {
        n: 'mailbox-more-toggle', type: 'button', title: '更多', 'aria-label': b.derive(s => `${s.email} 的更多操作`),
        'aria-expanded': b.derive(s => String(s.menuOpen)), 'aria-controls': `mailbox-more-${store.state.id}`,
      }, onClick() { store.toggleMenu(); } }, [Glyph('more')]),
      Show({ when: menuOpen, ok: () => View({ as: 'nav', class: 'jx-mail-menu jx-mailbox-more-menu',
        attributes: { n: 'mailbox-more-panel', id: `mailbox-more-${store.state.id}`, 'aria-label': `${store.state.email} 的更多操作` },
        onMounted({ target: host }) { (host.get$elm?.() || host).querySelector('button:not(:disabled)')?.focus(); } }, [
        View({ as: 'button', class: 'jx-mail-menu-action', attributes: { n: 'refresh-mailbox', type: 'button', disabled: refreshDisabled },
          onClick(event) { event.currentTarget.closest('[data-n="mailbox-more-controls"]').querySelector('[data-n="mailbox-more-toggle"]').focus(); store.refresh(); } }, [Glyph('refresh'), refreshLabel]),
      ]) }),
    ]),
    Show({ when: opened, ok: () => View({ as: 'div', class: 'jx-mailbox-status-popover',
      attributes: { n: 'mailbox-status-details', id: `mailbox-status-${store.state.id}`, role: 'status', 'aria-live': 'polite' },
    }, [View({ as: 'p', attributes: { n: 'mailbox-problem-message' } }, [problem]),
      Show({ when: b.derive(s => s.status?.state === 'oauth_required'), ok: () => View({ class: 'jx-mailbox-oauth', attributes: { n: 'mailbox-oauth-login' } }, [
        Show({ when: b.derive(s => !!s.oauth_config?.needs_setup), ok: () => View({ attributes: { n: 'mailbox-oauth-setup' } }, [
          View({ as: 'p', class: 'jx-muted' }, ['邮箱已添加。请填写授权应用信息，再手动开始登录。']),
          oauthField('client_id', 'OAuth 应用 ID（Client ID）'), oauthField('token_endpoint', 'HTTPS 令牌端点'),
          View({ as: 'button', class: 'jx-button jx-button--secondary', attributes: { n: 'save-mailbox-oauth-settings', type: 'button',
            disabled: b.derive(s => s.oauth.busy || !s.oauthDraft.client_id.trim() || !s.oauthDraft.token_endpoint.trim() ? '' : undefined) },
            onClick() { store.saveOAuthSettings(); } }, [b.derive(s => s.oauth.state === 'saving' ? '正在保存…' : '保存授权设置')]),
        ]) }),
        View({ as: 'button', class: 'jx-button jx-button--primary', attributes: {
          n: 'start-mailbox-oauth', type: 'button', disabled: b.derive(s => s.oauth.busy || s.oauth_config?.needs_setup ? '' : undefined),
        }, onClick() { store.startOAuth(); } }, [b.derive(s => s.oauth.busy ? '等待授权完成…' : '开始 OAuth 登录')]),
        Show({ when: b.derive(s => !!s.oauth.verification_uri), ok: () => View({ class: 'jx-mailbox-oauth-prompt', attributes: { n: 'mailbox-oauth-prompt' } }, [
          View({ as: 'p', attributes: { n: 'mailbox-oauth-code-instructions' } }, ['在授权页面输入：']),
          View({ as: 'code', class: 'jx-mailbox-oauth-code', attributes: { n: 'mailbox-oauth-user-code' } }, [b.derive(s => s.oauth.user_code)]),
          View({ as: 'a', class: 'jx-button jx-button--secondary', attributes: {
            n: 'open-mailbox-oauth-page', href: b.derive(s => s.oauth.verification_uri), target: '_blank', rel: 'noopener noreferrer',
          } }, ['打开授权页面']),
        ]) }),
        Show({ when: b.derive(s => !!s.oauth.error), ok: () => View({ as: 'p', class: 'jx-error', attributes: { n: 'mailbox-oauth-error' } }, [b.derive(s => s.oauth.error)]) }),
        Show({ when: b.derive(s => s.oauth.state === 'complete' && !s.oauth.error), ok: () => View({ as: 'p', attributes: { n: 'mailbox-oauth-complete' } }, ['授权成功，正在更新邮箱状态。']) }),
      ]) }),
    ]) }),
  ]), Show({ when: refreshing, ok: () => View({ as: 'p', class: 'jx-sidebar-refresh-status jx-muted', attributes: { n: 'mailbox-refresh-progress', role: 'status' } }, ['正在刷新邮箱…']) }),
    Show({ when: refreshError, ok: () => View({ as: 'p', class: 'jx-sidebar-refresh-status jx-error', attributes: { n: 'mailbox-refresh-error', role: 'status' } }, [refreshError]) }),
    Show({ when: expanded, ok: () => View({ class: 'jx-mailbox-folders',
    attributes: { n: 'mailbox-folders', id: `folders-${store.state.id}` },
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
            View({ class: 'jx-row' }, [RelativeTime({ timestamp: item.timestamp }), item.count > 1 ? Badge({}, [`${item.count} 次`]) : undefined,
              View({ as: 'button', class: 'jx-button jx-button--ghost', attributes: { type: 'button' }, onClick() { store.setRead(item.id, !item.read); } }, [item.read ? '标为未读' : '标为已读']),
            ]),
          ]),
        ]);
      } }), Show({ when: empty, ok: () => EmptyState({ icon: 'notification', title: '暂无通知', description: '邮箱状态和邮件操作的错误会记录在这里。' }) })]),
    ]) }),
  ]);
}
