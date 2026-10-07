export function MailService(root, request = fetch) {
  let readQueue = Promise.resolve();
  async function json(path, options) {
    const response = await request(`${root}/${path}`, { credentials: 'same-origin', ...options });
    let result;
    try { result = await response.json(); }
    catch { throw new Error('邮件服务返回异常，请重试'); }
    if (!response.ok) throw new Error(result.error || '邮件读取失败，请重试');
    return result;
  }
  return {
    mailboxes: signal => json('api/mailboxes', { signal }),
    threads: (params, signal) => json(`api/threads?${new URLSearchParams(params)}`, { signal }),
    detail: (kind, id, page, signal) => json(`api/${kind}/${encodeURIComponent(id)}?page=${page}`, { signal }),
    setRead(id, read, signal) {
      const operation = readQueue.then(() => {
        if (signal?.aborted) throw signal.reason || new DOMException('Aborted', 'AbortError');
        return json(`flags/${encodeURIComponent(id)}?read=${read ? 1 : 0}`, {
          method: 'POST', headers: { 'X-Jianxin-Request': '1' }, signal,
        });
      });
      readQueue = operation.catch(() => {});
      return operation;
    },
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
