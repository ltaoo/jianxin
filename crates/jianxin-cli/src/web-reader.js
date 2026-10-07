// Trusted outer-page code only. Email scripts remain disabled by sandbox/CSP.
for (const frame of document.querySelectorAll('iframe[title="邮件正文"]')) {
  const item = frame.closest('.mail-item');
  const details = frame.closest('details');
  const button = item.querySelector('[data-read-button]');
  const state = item.querySelector('[data-read-state]');
  const status = item.querySelector('[data-read-status]');
  let autoAttempted = false;
  let busy = false;
  const setRead = async (read) => {
    if (busy) return;
    busy = true;
    autoAttempted = true;
    button.disabled = true;
    status.classList.remove('error');
    status.textContent = '正在同步到邮箱服务器…';
    try {
      const response = await fetch(`${item.dataset.readUrl}?read=${read ? 1 : 0}`, {
        method: 'POST', headers: {'X-Jianxin-Request': '1'}, credentials: 'same-origin'
      });
      const result = await response.json();
      if (!response.ok) throw new Error(result.error || '同步失败，请重试');
      item.dataset.read = result.is_read ? '1' : '0';
      state.textContent = result.is_read ? '已读' : '未读';
      status.textContent = '已同步到邮箱服务器';
      const count = document.querySelector('[data-thread-unread]');
      if (count && result.thread_unread !== null) count.textContent = result.thread_unread;
    } catch (error) {
      status.classList.add('error');
      status.textContent = `同步失败：${error.message}。当前状态未更改，可点击按钮重试。`;
    } finally {
      busy = false;
      button.disabled = false;
      button.textContent = item.dataset.read === '1' ? '标为未读' : '标为已读';
    }
  };
  const autoRead = () => {
    if (!document.hidden && (!details || details.open) && !autoAttempted && item.dataset.read === '0') setRead(true);
  };
  button.addEventListener('click', () => setRead(item.dataset.read !== '1'));
  document.addEventListener('visibilitychange', () => {
    if (frame.contentDocument?.getElementById('jianxin-mail-content')) autoRead();
  });
  let observer;
  let pending = false;
  const resize = () => {
    if (pending) return;
    pending = true;
    requestAnimationFrame(() => {
      pending = false;
      const content = frame.contentDocument?.getElementById('jianxin-mail-content');
      if (!content) return;
      const height = Math.ceil(Math.max(content.scrollHeight, content.getBoundingClientRect().height)) + 32;
      const next = `${Math.max(1, Math.min(height, 2000000))}px`;
      if (frame.style.height !== next) frame.style.height = next;
    });
  };
  const watch = () => {
    observer?.disconnect();
    const content = frame.contentDocument?.getElementById('jianxin-mail-content');
    if (!content) return;
    observer = new ResizeObserver(resize);
    observer.observe(content);
    frame.contentDocument.addEventListener('load', resize, true);
    resize();
    autoRead();
  };
  const expand = () => {
    if (details && !details.open) return;
    if (!frame.hasAttribute('src')) frame.src = frame.dataset.src;
    else if (frame.contentDocument?.getElementById('jianxin-mail-content')) autoRead();
  };
  if (details) {
    details.addEventListener('toggle', expand);
    if (location.hash === `#${details.id}`) details.open = true;
  }
  frame.addEventListener('load', watch);
  window.addEventListener('resize', resize);
  if (frame.contentDocument?.readyState === 'complete') watch();
  expand();
}
