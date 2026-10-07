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
  function BodyFrame() {
    let cleanup = () => {};
    return View({ as: 'iframe', class: 'jx-mail-body', attributes: {
      title: '邮件正文', src: frameUrl, scrolling: 'no',
      sandbox: 'allow-same-origin allow-popups allow-popups-to-escape-sandbox', referrerpolicy: 'no-referrer',
    }, onMounted({ target: frame }) {
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
  return View({ as: 'article', attributes: { id: `mail-${store.state.id}` }, class: 'jx-mail-message', onUnmounted() {
    events.forEach(off => off()); derived.forEach(value => value.destroy()); state.destroy();
  } }, [
    View({ as: 'button', class: 'jx-message-header', attributes: { type: 'button',
      'aria-expanded': pick( s => String(s.expanded)), 'aria-controls': `body-${store.state.id}` },
      onClick() { store.toggle(); },
    }, [Avatar({ name: store.state.sender }), View({ class: 'jx-message-identity' }, [
      View({ as: 'strong', class: 'jx-truncate' }, [store.state.sender || '未知发件人']),
      View({ as: 'span', class: 'jx-muted jx-truncate', attributes: { title: store.state.recipient } }, [`收件人：${store.state.recipient || '未提供'}`]),
    ]), RelativeTime({ timestamp: store.state.timestamp }),
      View({ as: 'span', class: pick( s => `jx-expand ${s.expanded ? 'is-open' : ''}`) }, [Glyph('next')])]),
    Show({ when: pick( s => s.expanded), ok: () => View({ attributes: { id: `body-${store.state.id}` } }, [
      View({ class: 'jx-message-meta' }, [View({ as: 'span', class: 'jx-muted' }, [Glyph('check'), readLabel]),
        View({ as: 'button', class: 'jx-button jx-button--ghost', attributes: { type: 'button', disabled: disabled },
          onClick() { store.setRead(!store.state.is_read); },
        }, [actionLabel]),
        View({ as: 'span', class: statusClass, attributes: { role: 'status', 'aria-live': 'polite' } }, [statusLabel]),
      ]),
      BodyFrame(),
      store.state.attachment_count > 0 && View({ class: 'jx-attachment-note' }, [Glyph('attachment'), `${store.state.attachment_count} 个附件 · 当前阅读器暂不支持下载`]),
    ]) }),
  ]);
}
