export function MailService(root, request = fetch) {
  let flagQueue = Promise.resolve();
  async function json(path, options) {
    const response = await request(`${root}/${path}`, { credentials: 'same-origin', ...options });
    let result;
    try { result = await response.json(); }
    catch { throw new Error('邮件服务返回异常，请重试'); }
    if (!response.ok) throw Object.assign(new Error(result.error || '邮件读取失败，请重试'), { status: response.status });
    return result;
  }
  function setFlag(id, flag, value, signal) {
    const operation = flagQueue.then(() => {
      if (signal?.aborted) throw signal.reason || new DOMException('Aborted', 'AbortError');
      return json(`flags/${encodeURIComponent(id)}?${flag}=${value ? 1 : 0}`, {
        method: 'POST', headers: { 'X-Jianxin-Request': '1' }, signal,
      });
    });
    flagQueue = operation.catch(() => {});
    return operation;
  }
  return {
    mailboxes: signal => json('api/mailboxes', { signal }),
    configPresets: signal => json('api/config/presets', { signal }),
    configSource: (url, signal) => json('api/config/source', { method: 'POST', signal,
      headers: { 'Content-Type': 'application/json', 'X-Jianxin-Request': '1' }, body: JSON.stringify({ url }) }),
    configPreview: (config, signal) => json('api/config/preview', { method: 'POST', signal,
      headers: { 'Content-Type': 'application/json', 'X-Jianxin-Request': '1' }, body: JSON.stringify(config) }),
    configImport: config => json('api/config/import', { method: 'POST',
      headers: { 'Content-Type': 'application/json', 'X-Jianxin-Request': '1' }, body: JSON.stringify(config) }),
    reloadMailboxes: signal => json('api/mailboxes/reload', {
      method: 'POST', headers: { 'X-Jianxin-Request': '1' }, signal,
    }),
    startOAuth: (id, signal) => json(`api/mailboxes/oauth?${new URLSearchParams({ account: id })}`, {
      method: 'POST', headers: { 'X-Jianxin-Request': '1' }, signal,
    }),
    saveOAuthSettings: (account, settings) => json('api/mailboxes/oauth-settings', { method: 'POST',
      headers: { 'Content-Type': 'application/json', 'X-Jianxin-Request': '1' }, body: JSON.stringify({ account, ...settings }) }),
    oauthStatus: (id, signal) => json(`api/mailboxes/oauth?${new URLSearchParams({ account: id })}`, { signal }),
    threads: (params, signal) => json(`api/threads?${new URLSearchParams(params)}`, { signal }),
    detail: (kind, id, page, signal, folder = '') => json(`api/${kind}/${encodeURIComponent(id)}?${new URLSearchParams({ page, folder: kind === 'thread' ? folder : '' })}`, { signal }),
    metadata: (id, signal) => json(`api/message/${encodeURIComponent(id)}/metadata`, { signal }),
    prepareReply: (id, signal) => json(`api/reply/${encodeURIComponent(id)}`, { signal }),
    contacts: (q, signal) => json(`api/contacts?${new URLSearchParams({ q })}`, { signal }),
    sendReply: (id, reply) => json(`api/reply/${encodeURIComponent(id)}`, {
      method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Jianxin-Request': '1' }, body: JSON.stringify(reply),
    }),
    setRead: (id, read, signal) => setFlag(id, 'read', read, signal),
    setStarred: (id, starred, signal) => setFlag(id, 'starred', starred, signal),
    body: (id, { images, text }) => `${root}/body/${encodeURIComponent(id)}?images=${images ? 1 : 0}&text=${text ? 1 : 0}`,
  };
}

export function AgentService(root, request = fetch) {
  async function call(path, options) {
    const response = await request(`${root}/api/agent${path}`, { credentials: 'same-origin', ...options });
    const result = await response.json();
    if (!response.ok) throw new Error(result.error || '邮件助手请求失败');
    return result;
  }
  return { status: account => call(`?${new URLSearchParams({ account })}`),
    action: (operation, payload) => call(`/${operation}`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Jianxin-Request': '1' }, body: JSON.stringify(payload) }) };
}
