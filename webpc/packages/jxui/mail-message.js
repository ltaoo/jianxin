import { Avatar, Glyph, RelativeTime } from './index.js';

/** One message, with lazy body loading and isolated email HTML. */
export function MailMessage({ store, service }) {
  const state = ref(store.state);
  const events = [store.onStateChange(() => state.as(store.state))];
  const derived = [];
  const pick = fn => { const value = Timeless.computed(state, fn); derived.push(value); return value; };
  const frameUrl = pick( s => service.body(s.id, s.options));
  const readLabel = pick(s => s.is_read ? '已读' : '未读');
  const disabled = pick(s => s.pending ? '' : undefined);
  const actionLabel = pick(s => s.is_read ? '标为未读' : '标为已读');
  const statusClass = pick(s => s.error ? 'jx-error' : 'jx-muted');
  const statusLabel = pick(s => s.error || s.status);
  let detachMenu = () => {};
  function closeInfo(event) {
    const selector = store.state.infoPanel === 'sender' ? 'mail-message-sender' : 'mail-more-toggle';
    const trigger = event.currentTarget.closest('article')?.querySelector(`[data-n="${selector}"]`);
    store.closeInfo(); trigger?.focus();
  }
  function infoValue(key) {
    return pick(s => {
      const value = s.metadata?.[key];
      if (value == null || value === '' || (Array.isArray(value) && !value.length)) return '未提供';
      if (Array.isArray(value)) return value.join('\n');
      if (['sent_at', 'received_at'].includes(key)) {
        const date = new Date(value * 1000);
        return Number.isFinite(date.getTime()) ? date.toLocaleString('zh-CN', { hour12: false }) : '未提供';
      }
      return key === 'size' ? `${value} 字节` : String(value);
    });
  }
  function addressList(field, label) {
    return View({ attributes: { n: `mail-info-${field}` } }, [
      View({ as: 'h3', attributes: { n: `mail-info-${field}-title` } }, [label]),
      For({ each: pick(s => s.metadata?.[field] || []), render(address) {
        return View({ class: 'jx-sender-address', attributes: { n: 'mail-sender-address' } }, [
          View({ as: 'strong', attributes: { n: 'mail-sender-name' } }, [address.name || '未提供姓名']),
          View({ as: 'code', attributes: { n: 'mail-sender-email' } }, [address.email || '未提供邮箱地址']),
        ]);
      } }),
      Show({ when: pick(s => !!s.metadata && !(s.metadata[field]?.length)), ok: () => View({ as: 'p', class: 'jx-muted', attributes: { n: 'mail-sender-empty' } }, ['未提供邮箱地址']) }),
    ]);
  }
  function InfoPanel(modal = false) {
    const fields = [['id', 'id（本地邮件记录）'], ['thread_id', 'threadid（本地对话）'], ['message_id_header', 'Message-ID（邮件头）'],
      ['remote_id', 'remote_id（服务器邮件 ID / IMAP UID）'], ['immutable_id', 'immutable_id'],
      ['account_email', '所属邮箱'], ['account_id', 'account_id'], ['provider', '邮件服务类型'], ['folder_name', '文件夹'], ['folder_id', 'folder_id'],
      ['in_reply_to', 'In-Reply-To'], ['references', 'References'], ['server_thread_key', '服务器对话标识'], ['thread_index_root', 'Thread-Index root'],
      ['sent_at', '发送时间'], ['received_at', '接收时间'], ['size', '邮件大小']];
    return View({ as: modal ? 'dialog' : 'div', class: modal ? 'jx-mail-info jx-mail-metadata-dialog' : 'jx-mail-info', attributes: {
      n: modal ? 'mail-metadata-dialog' : 'mail-info-panel', id: `info-${store.state.id}`, role: modal ? undefined : 'region',
      'aria-label': modal ? '邮件元数据' : '发件人详情', 'aria-modal': modal ? 'true' : undefined,
    }, onMounted({ target }) {
      if (!modal) return;
      const dialog = target.get$elm?.() || target;
      const cancel = event => { event.preventDefault(); closeInfo(event); };
      const backdrop = event => {
        if (event.target !== dialog) return;
        const rect = dialog.getBoundingClientRect();
        if (event.clientX < rect.left || event.clientX > rect.right || event.clientY < rect.top || event.clientY > rect.bottom) closeInfo(event);
      };
      dialog.addEventListener('cancel', cancel); dialog.addEventListener('click', backdrop);
      dialog.showModal();
      return () => { dialog.removeEventListener('cancel', cancel); dialog.removeEventListener('click', backdrop); if (dialog.open) dialog.close(); };
    } }, [
      View({ class: 'jx-row', attributes: { n: 'mail-info-heading' } }, [
        View({ as: 'strong', attributes: { n: 'mail-info-title' } }, [pick(s => s.infoPanel === 'sender' ? '发件人详情' : '邮件元数据')]),
        View({ class: 'jx-spacer', attributes: { n: 'mail-info-heading-spacer' } }),
        View({ as: 'button', class: 'jx-button jx-button--ghost', attributes: { n: 'mail-info-close', type: 'button', 'aria-label': '关闭邮件详情' }, onClick: closeInfo }, ['关闭']),
      ]),
      Show({ when: pick(s => s.metadataPending), ok: () => View({ as: 'p', class: 'jx-muted', attributes: { n: 'mail-info-loading', role: 'status' } }, ['正在读取邮件详情…']) }),
      Show({ when: pick(s => !!s.metadataError), ok: () => View({ attributes: { n: 'mail-info-failure' } }, [
        View({ as: 'p', class: 'jx-error', attributes: { n: 'mail-info-error', role: 'status' } }, [pick(s => s.metadataError)]),
        View({ as: 'button', class: 'jx-button jx-button--ghost', attributes: { n: 'mail-info-retry', type: 'button' }, onClick() { store.loadMetadata(); } }, ['重新加载']),
      ]) }),
      Show({ when: pick(s => !!s.metadata && s.infoPanel === 'sender'), ok: () => View({ attributes: { n: 'mail-sender-details' } }, [
        addressList('from', '发件人'),
        addressList('to', '收件人'),
        Show({ when: pick(s => !!s.metadata?.cc?.length), ok: () => addressList('cc', '抄送') }),
        Show({ when: pick(s => !!s.metadata?.reply_to?.length), ok: () => addressList('reply_to', 'Reply-To（回复地址）') }),
      ]) }),
      Show({ when: pick(s => !!s.metadata && s.infoPanel === 'metadata'), ok: () => View({ attributes: { n: 'mail-metadata-details' } }, [
        View({ as: 'p', class: 'jx-muted', attributes: { n: 'mail-metadata-help' } }, ['id 区分本地记录，Message-ID 用于核对邮件身份，threadid 表示所属对话。']),
        View({ as: 'dl', class: 'jx-mail-metadata', attributes: { n: 'mail-metadata-fields' } }, fields.flatMap(([key, label]) => [
          View({ as: 'dt', attributes: { n: `mail-metadata-${key}-label` } }, [label]),
          View({ as: 'dd', attributes: { n: `mail-metadata-${key}` } }, [View({ as: 'code', attributes: { n: `mail-metadata-${key}-value` } }, [infoValue(key)])]),
        ])),
      ]) }),
    ]);
  }
  function BodyFrame() {
    let cleanup = () => {};
    return View({ as: 'iframe', class: 'jx-mail-body', attributes: {
      n: 'mail-body-frame', title: '邮件正文', src: frameUrl, scrolling: 'no',
      sandbox: 'allow-same-origin allow-popups allow-popups-to-escape-sandbox', referrerpolicy: 'no-referrer',
    }, onMounted({ target }) {
      const frame = target.get$elm?.() || target;
      let observer, content, animation;
      function resize() {
        if (animation) return;
        animation = requestAnimationFrame(() => {
          animation = undefined;
          if (!content) return;
          const height = Math.ceil(Math.max(content.scrollHeight, content.getBoundingClientRect().height)) + 32;
          frame.style.height = `${Math.max(1, height)}px`;
        });
      }
      function visible() { if (content) store.loaded(!document.hidden); }
      function detach() {
        observer?.disconnect(); content?.removeEventListener('load', resize, true);
        content = undefined;
      }
      function loaded() {
        detach();
        try { content = frame.contentDocument?.getElementById('jianxin-mail-content'); }
        catch { return; }
        if (!content) return;
        if (typeof ResizeObserver !== 'undefined') { observer = new ResizeObserver(resize); observer.observe(content); }
        content.addEventListener('load', resize, true); resize(); visible();
      }
      frame.addEventListener('load', loaded);
      window.addEventListener('resize', resize); document.addEventListener('visibilitychange', visible);
      loaded();
      cleanup = () => { detach(); frame.removeEventListener('load', loaded);
        window.removeEventListener('resize', resize); document.removeEventListener('visibilitychange', visible);
        if (animation) cancelAnimationFrame(animation); };
    }, onUnmounted() { cleanup(); } });
  }
  function menuAction(name, label, icon, click, disabled) {
    return View({ as: 'button', class: 'jx-mail-menu-action', attributes: { n: name, type: 'button', disabled },
      onClick(e) { store.closeMenu(); click(e); } }, [Glyph(icon), label]);
  }
  return View({ as: 'article', attributes: { n: 'mail-message', id: `mail-${store.state.id}` }, class: 'jx-mail-message',
    onMounted({ target }) {
      const article = target.get$elm?.() || target;
      const outside = event => { if (!article.querySelector('[data-n="mail-more-controls"]')?.contains(event.target)) store.closeMenu(); };
      document.addEventListener('pointerdown', outside);
      detachMenu = () => document.removeEventListener('pointerdown', outside);
    }, onKeyDown(e) {
      if (e.key !== 'Escape') return;
      if (store.state.menuOpen) { e.preventDefault(); e.stopPropagation(); store.closeMenu(); e.currentTarget.querySelector('[data-n="mail-more-toggle"]')?.focus(); }
      else if (store.state.infoPanel === 'sender') { e.preventDefault(); closeInfo(e); }
    }, onUnmounted() {
      detachMenu(); events.forEach(off => off()); derived.forEach(value => value.destroy()); state.destroy();
    } }, [
    View({ class: 'jx-message-header', attributes: { n: 'mail-message-header' } }, [
      Avatar({ name: store.state.sender }),
      View({ class: 'jx-message-identity', attributes: { n: 'mail-message-identity' } }, [
        View({ as: 'button', class: 'jx-sender-details-button', attributes: { n: 'mail-message-sender', type: 'button',
          'aria-label': `查看发件人 ${store.state.sender || '未知发件人'} 的详情`, 'aria-expanded': pick(s => String(s.infoPanel === 'sender')), 'aria-controls': `info-${store.state.id}` },
          onClick() { store.toggleInfo('sender'); } }, [
          View({ as: 'strong', class: 'jx-truncate', attributes: { n: 'mail-message-sender-name' } }, [store.state.sender || '未知发件人']),
          store.state.sender_email && store.state.sender_email !== store.state.sender ? View({ as: 'span', class: 'jx-muted jx-truncate', attributes: { n: 'mail-message-sender-email', title: store.state.sender_email } }, [`<${store.state.sender_email}>`]) : undefined,
        ]),
        View({ as: 'button', class: 'jx-recipient-details-button jx-muted jx-truncate', attributes: { n: 'mail-message-recipient', type: 'button', title: store.state.recipient,
          'aria-label': '查看收件人详情', 'aria-expanded': pick(s => String(s.infoPanel === 'sender')), 'aria-controls': `info-${store.state.id}` }, onClick() { store.toggleInfo('sender'); } }, [`发送至 ${store.state.recipient || '未提供'}`, Glyph('next')]),
      ]),
      View({ class: 'jx-message-tools', attributes: { n: 'mail-message-tools' } }, [
        RelativeTime({ timestamp: store.state.timestamp }),
        View({ as: 'button', class: pick(s => `jx-button jx-button--ghost jx-icon-button jx-mail-star ${s.is_starred ? 'is-starred' : ''}`), attributes: { n: 'mail-starred-toggle', type: 'button',
          disabled: pick(s => s.starPending ? '' : undefined), 'aria-pressed': pick(s => String(!!s.is_starred)), 'aria-label': pick(s => s.is_starred ? '取消收藏' : '收藏'), title: pick(s => s.is_starred ? '取消收藏' : '收藏') },
          onClick() { store.setStarred(!store.state.is_starred); } }, [Glyph('starred')]),
        View({ as: 'button', class: 'jx-button jx-button--ghost jx-icon-button', attributes: { n: 'mail-reply-open', type: 'button', 'aria-label': '回复', title: '回复' }, onClick() { store.openReply(); } }, [Glyph('reply')]),
        View({ class: 'jx-mail-more', attributes: { n: 'mail-more-controls' } }, [
          View({ as: 'button', class: 'jx-button jx-button--ghost jx-icon-button', attributes: { n: 'mail-more-toggle', type: 'button', 'aria-label': '更多邮件操作', title: '更多',
            'aria-expanded': pick(s => String(s.menuOpen)), 'aria-controls': `more-${store.state.id}` }, onClick(e) { store.toggleMenu(); if (store.state.menuOpen) e.currentTarget.parentElement.querySelector('[data-n="mail-more-panel"] button')?.focus(); } }, [Glyph('more')]),
          Show({ when: pick(s => s.menuOpen), ok: () => View({ as: 'nav', class: 'jx-mail-menu', attributes: { n: 'mail-more-panel', id: `more-${store.state.id}`, 'aria-label': '邮件操作' } }, [
            menuAction('mail-menu-reply', '回复', 'reply', () => store.openReply()),
            menuAction('mail-read-toggle', actionLabel, 'unread', () => store.setRead(!store.state.is_read), disabled),
            View({ as: 'hr', class: 'jx-mail-menu-divider', attributes: { n: 'mail-more-divider' } }),
            menuAction('mail-menu-sender', '发件人详情', 'user', () => store.toggleInfo('sender')),
            menuAction('mail-metadata-toggle', '邮件元数据', 'text', () => store.toggleInfo('metadata')),
          ]) }),
        ]),
        View({ as: 'button', class: 'jx-button jx-button--ghost jx-icon-button', attributes: { n: 'mail-message-toggle', type: 'button',
          'aria-expanded': pick(s => String(s.expanded)), 'aria-controls': `body-${store.state.id}`, 'aria-label': pick(s => s.expanded ? '折叠邮件' : '展开邮件') }, onClick() { store.toggle(); } },
          [View({ as: 'span', class: pick(s => `jx-expand ${s.expanded ? 'is-open' : ''}`), attributes: { n: 'mail-message-expand-icon' } }, [Glyph('next')])]),
      ]),
    ]),
    Show({ when: pick(s => s.infoPanel === 'sender'), ok: () => InfoPanel() }),
    Show({ when: pick(s => s.infoPanel === 'metadata'), ok: () => InfoPanel(true) }),
    Show({ when: pick(s => s.expanded), ok: () => View({ attributes: { n: 'mail-message-content', id: `body-${store.state.id}` } }, [
      View({ class: 'jx-message-body-wrap', attributes: { n: 'mail-message-body' } }, [BodyFrame()]),
      store.state.attachment_count > 0 ? View({ class: 'jx-attachment-note', attributes: { n: 'mail-attachment-note' } }, [Glyph('attachment'), `${store.state.attachment_count} 个附件 · 当前阅读器暂不支持下载`]) : undefined,
      View({ class: 'jx-message-footer', attributes: { n: 'mail-message-actions' } }, [
        View({ as: 'button', class: 'jx-button jx-button--secondary jx-reply-pill', attributes: { n: 'mail-reply-bottom', type: 'button' }, onClick() { store.openReply(); } }, [Glyph('reply'), '回复']),
        View({ as: 'span', class: 'jx-muted', attributes: { n: 'mail-read-state' } }, [readLabel]),
        View({ as: 'span', class: statusClass, attributes: { n: 'mail-read-status', role: 'status', 'aria-live': 'polite' } }, [statusLabel]),
        View({ as: 'span', class: 'jx-error', attributes: { n: 'mail-starred-status', role: 'status', 'aria-live': 'polite' } }, [pick(s => s.starError)]),
      ]),
    ]) }),
  ]);
}
