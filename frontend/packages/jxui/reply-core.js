import { ControlCore } from './core.js';

export const MAX_ATTACHMENT_BYTES = 20 * 1024 * 1024;
const validEmail = email => /^[^\s<>@,;]+@[^\s<>@,;]+$/.test(email) && email.length <= 320;
async function fileBase64(file) {
  const bytes = new Uint8Array(await file.arrayBuffer());
  let binary = '';
  for (let i = 0; i < bytes.length; i += 8192) binary += String.fromCharCode(...bytes.subarray(i, i + 8192));
  return btoa(binary);
}

/** One page-owned reply survives navigation; the View holds no draft state. */
export function ReplyCore({ service, replied = () => {}, timers = globalThis, readFile = fileBase64 }) {
  const initial = () => ({ open: false, messageId: '', from: '', to: [], cc: [], subject: '', body: '',
    queries: { to: '', cc: '' }, recipientField: '', contacts: [], contactIndex: -1, contactLoading: false,
    contactError: '', attachments: [], loading: false, sending: false, locked: false, sent: false,
    error: '', dirty: false, discardPending: false, position: null, dropActive: false, subjectEditing: false, maximized: false, minimized: false });
  const core = ControlCore(initial());
  const destroy = core.destroy.bind(core);
  let destroyed = false, request, contactRequest, contactTimer, generation = 0, fileId = 0, drag;
  const editable = () => !destroyed && core.state.open && !core.state.loading && !core.state.sending && !core.state.locked;
  function stopSearch() { timers.clearTimeout(contactTimer); contactRequest?.abort(); contactRequest = undefined; }
  function clamp(value, maximum) { return Math.max(0, Math.min(value, Math.max(0, maximum))); }
  function recipientsChanged(field, next) {
    core.set({ [field]: next, dirty: true, error: '', discardPending: false });
  }
  return Object.assign(core, {
    async open(messageId) {
      if (destroyed) return;
      if (core.state.open) { if (messageId !== core.state.messageId) core.set({ error: '请先发送或关闭当前回复，再回复另一封邮件。' }); return; }
      const current = ++generation;
      request = new AbortController(); core.set({ ...initial(), open: true, messageId, loading: true });
      try {
        const reply = await service.prepareReply(messageId, request.signal);
        if (!destroyed && current === generation) core.set({ from: reply.from, to: reply.to, cc: reply.cc || [], subject: reply.subject });
      } catch (error) {
        if (!destroyed && current === generation && error.name !== 'AbortError') core.set({ error: error.message });
      } finally { if (!destroyed && current === generation) core.set({ loading: false }); }
    },
    edit(field, value) {
      if (editable() && ['subject', 'body'].includes(field)) core.set({ [field]: value, dirty: true, error: '', discardPending: false });
    },
    toggleSubject() { if (editable()) core.set({ subjectEditing: !core.state.subjectEditing }); },
    toggleMaximize() { drag = undefined; core.set({ maximized: !core.state.maximized, minimized: false, position: null }); },
    toggleMinimize() { drag = undefined; core.set({ minimized: !core.state.minimized }); },
    focusRecipient(field) {
      if (!editable() || !['to', 'cc'].includes(field)) return;
      core.set({ recipientField: field, contactIndex: -1 }); this.searchContacts(field);
    },
    inputRecipient(field, value) {
      if (!editable()) return;
      core.set({ queries: { ...core.state.queries, [field]: value }, recipientField: field, dirty: true, discardPending: false });
      if (/[,;]\s*$/.test(value) && this.commitRecipients(field)) return;
      stopSearch(); core.set({ contacts: [], contactIndex: -1 });
      contactTimer = timers.setTimeout(() => this.searchContacts(field), 200);
    },
    async searchContacts(field) {
      if (!editable() || core.state.recipientField !== field) return;
      stopSearch(); const current = contactRequest = new AbortController();
      const query = core.state.queries[field].trim();
      if (new TextEncoder().encode(query).length > 256) { core.set({ contacts: [], contactLoading: false }); return; }
      core.set({ contactLoading: true, contactError: '' });
      try {
        const result = await service.contacts(query, current.signal);
        if (destroyed || contactRequest !== current || core.state.recipientField !== field) return;
        core.set({ contacts: result.contacts.filter(contact => validEmail(contact.email) &&
          !core.state[field].some(a => a.email.toLowerCase() === contact.email.toLowerCase())), contactIndex: -1 });
      } catch (error) {
        if (!destroyed && contactRequest === current && error.name !== 'AbortError') core.set({ contactError: '联系人搜索失败，可以直接输入邮箱地址。', contacts: [] });
      } finally { if (!destroyed && contactRequest === current) core.set({ contactLoading: false }); }
    },
    commitRecipients(field) {
      if (!editable()) return false;
      const query = core.state.queries[field].trim();
      if (!query) return true;
      const emails = query.split(/[\s,;]+/).filter(Boolean);
      if (!emails.every(validEmail)) { core.set({ error: '请输入完整邮箱地址，多个地址用逗号分隔，或从搜索结果选择联系人。' }); return false; }
      const next = [...core.state[field]];
      for (const email of emails) if (!next.some(a => a.email.toLowerCase() === email.toLowerCase())) next.push({ name: null, email });
      if (next.length + core.state[field === 'to' ? 'cc' : 'to'].length > 100) { core.set({ error: '收件人和抄送合计最多 100 人。' }); return false; }
      stopSearch(); recipientsChanged(field, next);
      core.set({ queries: { ...core.state.queries, [field]: '' }, contacts: [], contactIndex: -1, contactLoading: false });
      return true;
    },
    chooseContact(field, contact) {
      if (!editable() || !validEmail(contact.email)) return;
      if (core.state.to.length + core.state.cc.length >= 100) { core.set({ error: '收件人和抄送合计最多 100 人。' }); return; }
      if (!core.state[field].some(a => a.email.toLowerCase() === contact.email.toLowerCase())) recipientsChanged(field, [...core.state[field], contact]);
      stopSearch(); core.set({ queries: { ...core.state.queries, [field]: '' }, contacts: [], contactIndex: -1, contactLoading: false });
    },
    removeRecipient(field, index) { if (editable()) recipientsChanged(field, core.state[field].filter((_, i) => i !== index)); },
    blurRecipient(field) { if (editable()) this.commitRecipients(field); this.hideContacts(); },
    hideContacts() { stopSearch(); core.set({ recipientField: '', contacts: [], contactIndex: -1, contactLoading: false }); },
    recipientKeydown(field, event) {
      if (!editable() || event.isComposing) return;
      const { contacts, contactIndex, queries } = core.state;
      if (['ArrowDown', 'ArrowUp'].includes(event.key) && contacts.length) {
        event.preventDefault(); core.set({ contactIndex: clamp(contactIndex + (event.key === 'ArrowDown' ? 1 : -1), contacts.length - 1) });
      } else if (['Enter', 'Tab', ',', ';'].includes(event.key)) {
        if (event.key !== 'Tab') event.preventDefault();
        if (contactIndex >= 0 && contacts[contactIndex]) this.chooseContact(field, contacts[contactIndex]); else this.commitRecipients(field);
      } else if (event.key === 'Escape') {
        if (core.state.recipientField === field) { event.preventDefault(); event.stopPropagation(); this.hideContacts(); }
      } else if (event.key === 'Backspace' && !queries[field]) this.removeRecipient(field, core.state[field].length - 1);
    },
    async addFiles(files) {
      if (!editable()) return;
      const selected = Array.from(files);
      if (!selected.length) return;
      if (core.state.attachments.length + selected.length > 20 ||
          core.state.attachments.reduce((sum, a) => sum + a.size, 0) + selected.reduce((sum, f) => sum + f.size, 0) > MAX_ATTACHMENT_BYTES) {
        core.set({ error: '附件最多 20 个，合计不超过 20 MB。' }); return;
      }
      const current = generation;
      const added = selected.map(file => ({ id: ++fileId, filename: file.name, size: file.size, status: 'reading', data: '', error: '' }));
      core.set({ attachments: [...core.state.attachments, ...added], dirty: true, error: '', discardPending: false, dropActive: false });
      await Promise.all(added.map(async (attachment, i) => {
        try {
          const data = await readFile(selected[i]);
          if (!destroyed && current === generation) core.set({ attachments: core.state.attachments.map(a => a.id === attachment.id ? { ...a, data, status: 'ready' } : a) });
        } catch {
          if (!destroyed && current === generation) core.set({ attachments: core.state.attachments.map(a => a.id === attachment.id ? { ...a, status: 'failed', error: '读取失败，请移除后重新添加' } : a) });
        }
      }));
    },
    removeFile(id) { if (editable()) core.set({ attachments: core.state.attachments.filter(a => a.id !== id), dirty: true, error: '' }); },
    dragFiles(active) { if (editable()) core.set({ dropActive: active }); },
    startDrag({ x, y, left, top, width, height, viewportWidth, viewportHeight }) {
      if (core.state.maximized) return;
      drag = { x, y, left, top, width, height, viewportWidth, viewportHeight };
    },
    dragTo(x, y) {
      if (drag) core.set({ position: { x: clamp(drag.left + x - drag.x, drag.viewportWidth - drag.width), y: clamp(drag.top + y - drag.y, drag.viewportHeight - drag.height) } });
    },
    endDrag() { drag = undefined; },
    fitWindow({ left, top, width, height, viewportWidth, viewportHeight }) {
      if (core.state.position) core.set({ position: { x: clamp(left, viewportWidth - width), y: clamp(top, viewportHeight - height) } });
    },
    moveWindow(key, bounds) {
      if (core.state.maximized) return false;
      const offsets = { ArrowLeft: [-16, 0], ArrowRight: [16, 0], ArrowUp: [0, -16], ArrowDown: [0, 16] };
      if (!offsets[key]) return false;
      const [x, y] = offsets[key];
      this.fitWindow({ ...bounds, left: bounds.left + x, top: bounds.top + y });
      if (!core.state.position) { core.set({ position: { x: clamp(bounds.left + x, bounds.viewportWidth - bounds.width), y: clamp(bounds.top + y, bounds.viewportHeight - bounds.height) } }); }
      return true;
    },
    requestClose() {
      if (core.state.sending) return;
      if (core.state.dirty && !core.state.sent) core.set({ discardPending: true }); else this.discard();
    },
    keepEditing() { core.set({ discardPending: false }); },
    discard() {
      if (core.state.sending) return;
      ++generation; request?.abort(); stopSearch(); drag = undefined; core.set(initial());
    },
    async send() {
      if (!editable() || !this.commitRecipients('to') || !this.commitRecipients('cc')) return;
      const draft = core.state;
      if (!draft.to.length) { core.set({ error: '请添加至少一个收件人。' }); return; }
      if ((!draft.body.trim() && !draft.attachments.length) || draft.attachments.some(a => a.status !== 'ready')) {
        core.set({ error: '请填写正文或添加附件，并等待附件读取完成。' }); return;
      }
      core.set({ sending: true, error: '', discardPending: false }); this.hideContacts();
      try {
        await service.sendReply(draft.messageId, { to: draft.to, cc: draft.cc, subject: draft.subject, body: draft.body,
          attachments: draft.attachments.map(a => ({ filename: a.filename, data: a.data })) });
        if (!destroyed) { core.set({ sent: true, locked: true }); replied(); }
      } catch (error) {
        if (!destroyed) core.set({ error: error.status ? error.message : '发送结果未知，内容已保留。请先检查已发送邮件和发件队列，避免重复发送。', locked: !error.status || error.status >= 500 });
      } finally { if (!destroyed) core.set({ sending: false }); }
    },
    destroy() { destroyed = true; ++generation; request?.abort(); stopSearch(); destroy(); },
  });
}
