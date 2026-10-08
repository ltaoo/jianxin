import { Glyph } from './index.js';

/** Floating, non-modal composer. Events delegate all draft behavior to ReplyCore. */
export function ReplyWindow({ store }) {
  const open = ref(store.state.open);
  const off = store.onStateChange(value => open.as(value.open));
  return View({ class: 'jx-reply-host', attributes: { n: 'reply-window-host' }, onUnmounted() { off(); open.destroy(); } },
    [Show({ when: open, ok: () => ComposeWindow({ store }) })]);
}

function ComposeWindow({ store }) {
  const state = ref(store.state), refs = [];
  const off = store.onStateChange(value => state.as(value));
  const pick = fn => { const value = Timeless.computed(state, fn); refs.push(value); return value; };
  const disabled = pick(s => s.loading || s.sending || s.locked ? '' : undefined);
  function action(name, label, click, icon) {
    return View({ as: 'button', class: 'jx-button jx-button--ghost', attributes: { n: name, type: 'button', 'aria-label': label, title: label, disabled },
      onClick: click }, icon ? [Glyph(icon)] : [label]);
  }
  function fieldInput(field, label, multiline = false) {
    return View({ as: multiline ? 'textarea' : 'input', class: multiline ? 'jx-compose-body' : 'jx-compose-subject', attributes: {
      n: `reply-${field}-input`, 'aria-label': label, type: multiline ? undefined : 'text', rows: multiline ? '8' : undefined,
      maxlength: multiline ? '64000' : '1000', disabled,
    }, onMounted({ target: host }) {
      const target = host.get$elm?.() || host;
      const apply = () => { if (target.value !== store.state[field]) target.value = store.state[field]; };
      const input = e => store.edit(field, e.target.value);
      apply(); const stop = store.onStateChange(apply); target.addEventListener('input', input);
      return () => { stop(); target.removeEventListener('input', input); };
    } });
  }
  function recipientField(field, label) {
    const isExpanded = s => s.recipientField === field && (!!s.contacts.length || s.contactLoading || !!s.contactError || !!s.queries[field].trim());
    const expanded = pick(isExpanded);
    return View({ class: 'jx-compose-recipient-row', attributes: { n: `reply-${field}-field` } }, [
      View({ as: 'label', class: 'jx-muted', attributes: { n: `reply-${field}-label`, for: `reply-${field}-input` } }, [label]),
      View({ class: 'jx-compose-recipients', attributes: { n: `reply-${field}-recipients` } }, [
        For({ each: pick(s => s[field]), render(recipient, index) {
          return View({ as: 'span', class: 'jx-recipient-chip', attributes: { n: `reply-${field}-chip`, title: recipient.email } }, [
            View({ as: 'span', attributes: { n: `reply-${field}-chip-name` } }, [recipient.name || recipient.email]),
            action(`reply-${field}-remove`, `移除 ${recipient.email}`, () => store.removeRecipient(field, index.value), 'close'),
          ]);
        } }),
        View({ as: 'input', class: 'jx-recipient-input', attributes: { n: `reply-${field}-input`, id: `reply-${field}-input`,
          type: 'text', autocomplete: 'off', placeholder: '姓名或邮箱地址', disabled, role: 'combobox', 'aria-autocomplete': 'list',
          'aria-expanded': pick(s => String(isExpanded(s))), 'aria-controls': `reply-${field}-list`,
          'aria-activedescendant': pick(s => s.recipientField === field && s.contactIndex >= 0 ? `reply-${field}-contact-${s.contactIndex}` : undefined),
        }, onKeyDown(event) { store.recipientKeydown(field, event); }, onFocus() { store.focusRecipient(field); }, onBlur() { store.blurRecipient(field); },
        onMounted({ target: host }) {
          const target = host.get$elm?.() || host;
          const apply = () => { if (target.value !== store.state.queries[field]) target.value = store.state.queries[field]; };
          const input = e => store.inputRecipient(field, e.target.value);
          apply(); const stop = store.onStateChange(apply); target.addEventListener('input', input);
          return () => { stop(); target.removeEventListener('input', input); };
        } }),
        Show({ when: expanded, ok: () => View({ class: 'jx-contact-popup', attributes: { n: `reply-${field}-suggestions` } }, [
          View({ as: 'ul', attributes: { n: `reply-${field}-contact-list`, id: `reply-${field}-list`, role: 'listbox', 'aria-label': `${label}搜索结果` } }, [
            For({ each: pick(s => s.contacts), render(contact, index) {
              const owned = [];
              const rowPick = fn => { const value = Timeless.combine({ state, index }, d => fn(d.state, d.index)); owned.push(value); return value; };
              return View({ as: 'li', onUnmounted() { owned.forEach(r => r.destroy()); }, attributes: { n: `reply-${field}-contact-option`, id: rowPick((s, i) => `reply-${field}-contact-${i}`), role: 'option',
                'aria-selected': rowPick((s, i) => String(s.contactIndex === i)) } }, [
                View({ as: 'button', class: rowPick((s, i) => `jx-contact-option ${s.contactIndex === i ? 'is-active' : ''}`), attributes: {
                  n: `reply-${field}-select-contact`, type: 'button', tabindex: '-1', 'aria-label': `${contact.name || ''} ${contact.email}`,
                }, onPointerDown(e) { e.preventDefault(); }, onClick() { store.chooseContact(field, contact); } }, [
                  View({ as: 'strong', attributes: { n: `reply-${field}-contact-name` } }, [contact.name || contact.email]),
                  View({ as: 'span', class: 'jx-muted', attributes: { n: `reply-${field}-contact-email` } }, [contact.email]),
                ]),
              ]);
            } }),
          ]),
          Show({ when: pick(s => s.contactLoading), ok: () => View({ as: 'p', class: 'jx-muted', attributes: { n: 'reply-contact-loading', role: 'status' } }, ['正在搜索联系人…']) }),
          Show({ when: pick(s => !s.contactLoading && !s.contactError && !s.contacts.length), ok: () => View({ as: 'p', class: 'jx-muted', attributes: { n: 'reply-contact-empty', role: 'status' } }, ['没有匹配的联系人，可直接输入邮箱地址。']) }),
          Show({ when: pick(s => s.contactError), ok: () => View({ as: 'p', class: 'jx-error', attributes: { n: 'reply-contact-error', role: 'status' } }, [pick(s => s.contactError)]) }),
        ]) }),
      ]),
    ]);
  }
  function bounds(target) {
    const rect = target.closest('dialog').getBoundingClientRect();
    return { left: rect.left, top: rect.top, width: rect.width, height: rect.height, viewportWidth: window.innerWidth, viewportHeight: window.innerHeight };
  }
  function dialog() {
    return View({ as: 'dialog', class: pick(s => `jx-reply-window ${s.dropActive ? 'is-dropping' : ''} ${s.maximized ? 'is-maximized' : ''} ${s.minimized ? 'is-minimized' : ''}`), attributes: {
      n: 'reply-window', open: '', 'aria-label': '回复邮件', 'aria-modal': 'false',
    }, style: pick(s => s.position ? { left: `${s.position.x}px`, top: `${s.position.y}px`, right: 'auto', bottom: 'auto' } : {}),
    onKeyDown(e) { if (e.key === 'Escape') { e.preventDefault(); store.requestClose(); } },
    onUnmounted() { off(); refs.forEach(r => r.destroy()); state.destroy(); },
    onMounted({ target: host }) {
      const target = host.get$elm?.() || host;
      const previous = document.activeElement;
      const focus = s => { if (!s.loading && document.activeElement === previous) target.querySelector('[data-n="reply-body-input"]')?.focus(); };
      focus(store.state); const stop = store.onStateChange(focus);
      const resize = () => store.fitWindow(bounds(target));
      const beforeUnload = event => { if (store.state.dirty && !store.state.sent) { event.preventDefault(); event.returnValue = ''; } };
      const dragover = event => { if (Array.from(event.dataTransfer?.types || []).includes('Files')) { event.preventDefault(); event.dataTransfer.dropEffect = 'copy'; store.dragFiles(true); } };
      const dragleave = event => { if (!target.contains(event.relatedTarget)) store.dragFiles(false); };
      const drop = event => { if (event.dataTransfer?.files?.length) { event.preventDefault(); store.addFiles(event.dataTransfer.files); } store.dragFiles(false); };
      for (const [name, fn] of Object.entries({ dragover, dragleave, drop })) target.addEventListener(name, fn);
      window.addEventListener('resize', resize); window.addEventListener('beforeunload', beforeUnload);
      return () => {
        stop(); for (const [name, fn] of Object.entries({ dragover, dragleave, drop })) target.removeEventListener(name, fn);
        window.removeEventListener('resize', resize); window.removeEventListener('beforeunload', beforeUnload); previous?.focus?.();
      };
    } }, [
      View({ class: 'jx-compose-header', attributes: { n: 'reply-window-header' } }, [
        View({ class: 'jx-compose-handle', attributes: { n: 'reply-window-drag-handle', tabindex: '0', role: 'button', 'aria-label': '拖动回复窗口，也可用方向键移动' },
          onPointerDown(e) { if (e.button === 0) { e.currentTarget.setPointerCapture(e.pointerId); store.startDrag({ ...bounds(e.currentTarget), x: e.clientX, y: e.clientY }); } },
          onPointerMove(e) { store.dragTo(e.clientX, e.clientY); }, onPointerUp() { store.endDrag(); },
          onKeyDown(e) { if (store.moveWindow(e.key, bounds(e.currentTarget))) e.preventDefault(); },
        }, [Glyph('reply'), View({ as: 'strong', class: 'jx-truncate', attributes: { n: 'reply-window-title', title: pick(s => s.subject) } }, [pick(s => s.subject || '回复邮件')])]),
        View({ as: 'button', class: 'jx-button jx-button--ghost jx-icon-button', attributes: { n: 'reply-window-minimize', type: 'button', 'aria-label': pick(s => s.minimized ? '恢复回复窗口' : '最小化回复窗口'), title: pick(s => s.minimized ? '恢复' : '最小化'), 'aria-expanded': pick(s => String(!s.minimized)) }, onClick() { store.toggleMinimize(); } }, ['−']),
        View({ as: 'button', class: 'jx-button jx-button--ghost jx-icon-button', attributes: { n: 'reply-window-maximize', type: 'button', 'aria-label': pick(s => s.maximized ? '恢复窗口大小' : '放大回复窗口'), title: pick(s => s.maximized ? '恢复大小' : '放大'), 'aria-pressed': pick(s => String(s.maximized)) }, onClick() { store.toggleMaximize(); } }, [Glyph('expand')]),
        View({ as: 'button', class: 'jx-button jx-button--ghost jx-icon-button', attributes: { n: 'reply-window-close', type: 'button', 'aria-label': '关闭回复窗口', disabled: pick(s => s.sending ? '' : undefined) },
          onClick() { store.requestClose(); } }, [Glyph('close')]),
      ]),
      View({ as: 'form', class: 'jx-compose-form', attributes: { n: 'reply-form', 'aria-label': '回复邮件内容' }, onMounted({ target: host }) {
        const target = host.get$elm?.() || host;
        const submit = e => { e.preventDefault(); store.send(); };
        target.addEventListener('submit', submit); return () => target.removeEventListener('submit', submit);
      } }, [
        View({ class: 'jx-compose-fields', attributes: { n: 'reply-envelope' } }, [
          View({ as: 'p', class: 'jx-compose-from jx-muted', attributes: { n: 'reply-from' } }, [pick(s => `发件人：${s.from || '正在读取…'}`)]),
          recipientField('to', '收件人'), recipientField('cc', '抄送'),
          View({ as: 'button', class: 'jx-compose-subject-toggle jx-muted', attributes: { n: 'reply-subject-toggle', type: 'button', disabled, 'aria-expanded': pick(s => String(s.subjectEditing)) }, onClick() { store.toggleSubject(); } }, [pick(s => s.subjectEditing ? '收起主题' : '修改主题')]),
          Show({ when: pick(s => s.subjectEditing), ok: () => View({ class: 'jx-compose-subject-row', attributes: { n: 'reply-subject-field' } }, [
            View({ as: 'span', class: 'jx-muted', attributes: { n: 'reply-subject-label' } }, ['主题']), fieldInput('subject', '主题'),
          ]) }),
        ]),
        View({ class: 'jx-compose-content', attributes: { n: 'reply-content' } }, [
          fieldInput('body', '回复正文', true),
          View({ as: 'ul', class: 'jx-compose-attachments', attributes: { n: 'reply-attachments', 'aria-label': '待发送附件' } }, [
            For({ each: pick(s => s.attachments), render(file) {
              return View({ as: 'li', class: 'jx-compose-attachment', attributes: { n: 'reply-attachment' } }, [Glyph('attachment'),
                View({ class: 'jx-compose-attachment-info', attributes: { n: 'reply-attachment-details' } }, [
                  View({ as: 'strong', attributes: { n: 'reply-attachment-name' } }, [file.filename]),
                  View({ as: 'span', class: file.status === 'failed' ? 'jx-error' : 'jx-muted', attributes: { n: 'reply-attachment-status' } },
                    [file.status === 'reading' ? '正在读取…' : file.error || `${(file.size / 1024).toFixed(1)} KB`]),
                ]), action('reply-remove-attachment', `移除附件 ${file.filename}`, () => store.removeFile(file.id), 'close'),
              ]);
            } }),
          ]),
        ]),
        View({ as: 'p', class: 'jx-error jx-compose-error', attributes: { n: 'reply-error', role: 'status', 'aria-live': 'polite' } }, [pick(s => s.error)]),
        Show({ when: pick(s => s.discardPending), ok: () => View({ class: 'jx-compose-discard', attributes: { n: 'reply-discard-confirmation', role: 'alert' } }, [
          View({ as: 'p', attributes: { n: 'reply-discard-prompt' } }, ['关闭窗口将丢弃这份回复及附件。']),
          View({ as: 'button', class: 'jx-button jx-button--ghost', attributes: { n: 'reply-keep-editing', type: 'button' }, onClick() { store.keepEditing(); } }, ['保留回复']),
          View({ as: 'button', class: 'jx-button jx-button--ghost', attributes: { n: 'reply-confirm-discard', type: 'button' }, onClick() { store.discard(); } }, ['丢弃回复']),
        ]) }),
        View({ class: 'jx-compose-footer', attributes: { n: 'reply-actions' } }, [
          View({ as: 'button', class: 'jx-button jx-button--primary', attributes: { n: 'reply-send', type: 'submit',
            disabled: pick(s => s.loading || s.sending || s.locked || s.attachments.some(a => a.status !== 'ready') ? '' : undefined) } },
            [pick(s => s.sending ? '正在发送…' : s.sent ? '已发送' : '发送')]),
          action('reply-add-attachment', '添加附件', e => e.currentTarget.closest('dialog').querySelector('[data-n="reply-file-input"]').click(), 'attachment'),
          View({ as: 'input', attributes: { n: 'reply-file-input', type: 'file', multiple: '', hidden: '', 'aria-label': '选择附件', disabled }, onMounted({ target: host }) {
            const target = host.get$elm?.() || host;
            const change = e => { store.addFiles(e.target.files); target.value = ''; };
            target.addEventListener('change', change); return () => target.removeEventListener('change', change);
          } }),
          View({ as: 'span', class: 'jx-muted jx-compose-hint', attributes: { n: 'reply-upload-hint' } }, ['可拖入附件 · 合计 20 MB']),
          View({ as: 'span', class: 'jx-muted', attributes: { n: 'reply-send-status', role: 'status', 'aria-live': 'polite' } }, [pick(s => s.sent ? '回复已发送' : s.loading ? '正在加载回复…' : '')]),
        ]),
      ]),
      Show({ when: pick(s => s.dropActive), ok: () => View({ class: 'jx-compose-drop-hint', attributes: { n: 'reply-drop-hint', 'aria-hidden': 'true' } }, [Glyph('attachment'), '松开以添加附件']) }),
    ]);
  }
  return dialog();
}
