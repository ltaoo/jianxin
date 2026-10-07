import { Button, Glyph } from './index.js';
import { ControlCore } from './core.js';
export function AgentPanel({ store }) {
  const state = ref(store.state), refs = [], controls = [];
  const off = store.onStateChange(value => state.as(value));
  const derive = fn => { const r = Timeless.computed(state, fn); refs.push(r); return r; };
  function button(label, action, variant = 'ghost', disabled = () => false) {
    const c = ControlCore({ label, disabled: disabled(store.state) }, { click: action });
    const off = store.onStateChange(s => c.set({ disabled: disabled(s) }));
    return View({ onUnmounted() { off(); c.destroy(); } }, [Button({ store: c, variant })]);
  }
  let prompt = '', promptTarget;
  async function send() {
    if (!prompt.trim() || store.state.busy || store.state.loading || !store.state.configured) return;
    const previous = prompt;
    if (await store.send(previous)) { if (prompt === previous) { prompt = ''; if (promptTarget) promptTarget.value = ''; } }
  }
  function editor(value, label, change) {
    return View({ as: 'textarea', class: 'jx-agent-editor', attributes: { 'aria-label': label, rows: '4', maxlength: '16000' }, onInput(e) { change(e.target.value); }, onMounted({ target }) { target.value = value; if (label === '给邮件助手发送消息') promptTarget = target; },
      onKeyDown(e) { if (label === '给邮件助手发送消息' && e.key === 'Enter' && !e.shiftKey && !e.isComposing) { e.preventDefault(); send(); } } });
  }
  function draftView(d) {
    if (d.kind === 'read') {
      const approve = ControlCore({ label: d.read ? '确认标为已读' : '确认标为未读', disabled: store.state.busy }, { click: () => store.confirmRead(d.id) });
      const off = store.onStateChange(s => approve.set({ disabled: s.busy || s.loading }));
      return View({ class: 'jx-agent-draft', onUnmounted() { off(); approve.destroy(); } }, [View({ as: 'strong' }, ['邮件状态变更']), d.subject, Button({ store: approve, variant: 'primary' }), button('取消', () => store.discard(d.id), 'ghost', s => s.busy || s.loading)]);
    }
    let subject = d.subject, body = d.body;
    const save = ControlCore({ label: '确认保存草稿', disabled: store.state.busy }, { click: () => { save.set({ disabled: true }); store.saveDraft(d.id, subject, body); } });
    const off = store.onStateChange(s => save.set({ disabled: s.busy || s.loading }));
    return View({ class: 'jx-agent-draft', onUnmounted() { off(); save.destroy(); } }, [View({ as: 'strong' }, ['回复草稿']), View({ class: 'jx-muted' }, [`收件人：${d.to}`]),
      View({ as: 'input', class: 'jx-input', attributes: { 'aria-label': '草稿主题', maxlength: '240' }, onMounted({ target }) { target.value = subject; }, onInput(e) { subject = e.target.value; } }),
      editor(body, '草稿正文', value => body = value), Button({ store: save, variant: 'primary' }), button('放弃草稿', () => store.discard(d.id), 'ghost', s => s.busy || s.loading), View({ class: 'jx-muted' }, ['确认后保存至草稿箱，不会发送。'])]);
  }
  return View({ as: 'aside', class: derive(s => `jx-agent-panel ${s.open ? 'is-open' : ''}`), attributes: { 'aria-label': '邮件 Agent' }, onUnmounted() { off(); refs.forEach(r => r.destroy()); controls.forEach(c => c.destroy()); state.destroy(); } }, [
    View({ class: 'jx-agent-heading jx-row' }, [Glyph('agent'), View({ as: 'strong' }, ['邮件助手']), View({ class: 'jx-spacer' }), button('关闭助手', () => store.toggle())]),
    View({ class: 'jx-row jx-agent-tabs' }, [button('Agent', () => store.tab('agent')), button('MCP', () => store.tab('mcp'))]),
    View({ class: 'jx-muted jx-agent-scope' }, [derive(s => s.account ? '仅使用当前邮箱' : '使用已配置的邮箱')]),
    Show({ when: derive(s => s.tab === 'mcp'), ok: () => View({ class: 'jx-agent-scroll' }, [View({ as: 'h3' }, ['连接外部 Agent']), View({ as: 'p' }, ['通过现有 stdio MCP 接口读取邮件、搜索对话。']), View({ as: 'pre' }, [derive(s => JSON.stringify({ mcpServers: { jianxin: s.mcp || { command: 'jianxin', args: ['agent'] } } }, null, 2))]), View({ as: 'p', class: 'jx-muted' }, [derive(s => (s.tools || []).join(' · '))]), View({ class: 'jx-muted' }, ['在支持 MCP 的客户端中使用以上配置。账户写操作仍使用 MCP 的确认机制。'])]) }),
    View({ class: derive(s => `jx-agent-chat ${s.tab === 'agent' ? '' : 'is-hidden'}`) }, [
      View({ class: 'jx-agent-scroll', attributes: { 'aria-live': 'polite' } }, [
        Show({ when: derive(s => !s.configured), ok: () => View({ class: 'jx-agent-setup' }, [View({ as: 'h3' }, ['配置你的模型']), View({ as: 'p' }, ['在配置文件 general.agent 中设置 model、base_url 和 api_key，然后重启 Web 服务。支持 OpenAI 兼容接口与本机模型。'])]) }),
        Show({ when: derive(s => !s.messages.length && s.configured), ok: () => View({ class: 'jx-agent-suggestions' }, ['从邮件开始', button('总结未读邮件', () => store.send('搜索当前邮箱的未读邮件，概述需要关注的事项。')), button('最近有哪些重要邮件？', () => store.send('查看最近邮件，列出需要关注的事项。')), button('为当前对话起草回复', () => store.send('读取当前选中的完整对话，拟定礼貌的回复草稿。信息不足时先问我。'))]) }),
        For({ each: derive(s => s.messages.filter(m => m.content)), render: m => View({ class: `jx-agent-message jx-agent-message--${m.role}` }, [View({ class: 'jx-muted' }, [m.role === 'user' ? '你' : '邮件助手']), View({ class: 'jx-agent-content' }, [m.content])]) }),
        Show({ when: derive(s => s.stream), ok: () => View({ class: 'jx-agent-message' }, [View({ class: 'jx-muted' }, ['邮件助手']), View({ class: 'jx-agent-content' }, [derive(s => s.stream)])]) }),
        For({ each: derive(s => s.trace), render: t => View({ class: 'jx-agent-tool jx-row' }, [Glyph('check'), t]) }),
        For({ each: derive(s => s.busy ? [] : s.drafts), render: draftView }),
        Show({ when: derive(s => s.busy || s.loading), ok: () => View({ class: 'jx-agent-tool', attributes: { role: 'status' } }, ['正在处理…']) }),
        Show({ when: derive(s => s.error), ok: () => View({ class: 'jx-agent-error', attributes: { role: 'alert' } }, [derive(s => s.error)]) }),
      ]),
      View({ class: 'jx-agent-compose' }, [editor('', '给邮件助手发送消息', value => prompt = value),
        View({ class: 'jx-row' }, [button('发送', send, 'primary', s => !s.configured || s.busy || s.loading), button('停止', () => store.stop(), 'ghost', s => !s.busy), button('清空会话', () => store.clear(), 'ghost', s => s.busy || s.loading)]),
        View({ class: 'jx-muted' }, ['按需读取邮件并发送给你配置的模型。请检查生成的内容。'])]),
    ]),
  ]);
}
