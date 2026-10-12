export function mailboxDraft(preset = {}) {
  return { preset: preset.key || '', platform: preset.platform || '', provider: preset.provider || 'generic-imap',
    email: '', display_name: '', password: '', username: '', oauth: preset.key === 'outlook', client_id: '',
    token_endpoint: 'https://login.microsoftonline.com/consumers/oauth2/v2.0/token', advanced: preset.key === 'custom',
    ...Object.fromEntries(['imap', 'smtp'].flatMap(protocol => [
      [`${protocol}_host`, preset[protocol]?.host || ''], [`${protocol}_port`, String(preset[protocol]?.port || (protocol === 'imap' ? 993 : 465))],
      [`${protocol}_tls`, preset[protocol]?.tls || 'tls'], [`${protocol}_username`, ''],
    ])) };
}

export function mailboxConfig(draft) {
  const email = draft.email.trim();
  if (!/^[^@\s]+@[^@\s]+$/.test(email)) throw new Error('请输入有效的邮箱地址');
  if (!draft.oauth && !draft.password) throw new Error('请输入密码或客户端授权码');
  if ([draft.username, draft.imap_username, draft.smtp_username].some(value => /[\r\n\0]/.test(value)) || draft.password.includes('\0')) throw new Error('登录信息不能包含换行或空字符');
  const auth = { type: draft.oauth ? 'oauth2' : 'app_password' };
  if (draft.username.trim()) auth.username = draft.username.trim();
  if (draft.oauth) {
    if (draft.password) auth.access_token = draft.password;
    if (draft.client_id.trim()) auth.client_id = draft.client_id.trim();
    if (draft.token_endpoint.trim()) {
      let url;
      try { url = new URL(draft.token_endpoint.trim()); } catch {}
      if (!url || url.protocol !== 'https:' || url.username || url.password || url.hash || url.search || !url.pathname.endsWith('/token')) {
        throw new Error('OAuth 令牌端点需要是以 /token 结尾的 HTTPS URL');
      }
      auth.token_endpoint = url.href;
    }
  } else auth.password = draft.password;
  const account = { email, auth, provider: draft.oauth ? draft.provider : 'generic-imap' };
  if (draft.display_name.trim()) account.display_name = draft.display_name.trim();
  if (draft.platform) account.platform = draft.platform;
  for (const protocol of ['imap', 'smtp']) {
    const host = draft[`${protocol}_host`].trim(), port = draft[`${protocol}_port`];
    let address;
    try { address = new URL(`https://${host}`); } catch {}
    if (!host || /\s|:\d+$/.test(host) || !address || address.username || address.password || address.port || address.pathname !== '/' || address.search || address.hash || host.includes('/')) {
      throw new Error(`请输入有效的 ${protocol.toUpperCase()} 服务器地址，不要包含协议或端口`);
    }
    if (!/^\d+$/.test(String(port)) || Number(port) < 1 || Number(port) > 65535) throw new Error(`${protocol.toUpperCase()} 端口需要在 1 至 65535 之间`);
    const tls = draft[`${protocol}_tls`];
    if (!['tls', 'start_tls'].includes(tls)) throw new Error('请选择 TLS 或 STARTTLS');
    account[protocol] = { host, port: Number(port), tls };
    if (draft[`${protocol}_username`].trim()) account[protocol].username = draft[`${protocol}_username`].trim();
  }
  return { accounts: [account] };
}
