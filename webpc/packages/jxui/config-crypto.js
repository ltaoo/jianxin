// Wire-compatible with mobile/tools/config_crypto.mjs and Android RemoteConfig.
export const CONFIG_LIMIT = 2 * 1024 * 1024;
const text = new TextDecoder('utf-8', { fatal: true });
const utf8 = new TextEncoder();
function decode(value) {
  if (typeof value !== 'string') throw new Error('加密格式不支持或文件损坏');
  const binary = atob(value), bytes = Uint8Array.from(binary, c => c.charCodeAt(0));
  if (btoa(binary) !== value) throw new Error('密文必须是完整的 Base64 内容');
  return bytes;
}

export async function decryptConfig(data, password, method = 'aes-256-gcm', crypto = globalThis.crypto) {
  if (data.byteLength > CONFIG_LIMIT) throw new Error('加密配置不能超过 2 MB');
  const passwordBytes = utf8.encode(password);
  if (!passwordBytes.length || passwordBytes.length > 1024) throw new Error('请输入解密密钥（最多 1024 字节）');
  if (!crypto?.subtle) throw new Error('浏览器不支持本机解密，请通过 localhost 或 HTTPS 打开页面');
  let salt, nonce, encrypted, aad, bits;
  try {
    if (method === 'legacy-json') {
      const e = JSON.parse(text.decode(data));
      if (e.format !== 'jianxin-config' || e.version !== 1 || e.kdf !== 'pbkdf2-sha256' || e.iterations !== 600000 || e.cipher !== 'aes-256-gcm') throw new Error();
      salt = decode(e.salt); nonce = decode(e.nonce);
      const ciphertext = decode(e.ciphertext), tag = decode(e.tag);
      if (salt.length !== 16 || nonce.length !== 12 || !ciphertext.length || tag.length !== 16) throw new Error();
      encrypted = new Uint8Array(ciphertext.length + 16); encrypted.set(ciphertext); encrypted.set(tag, ciphertext.length);
      aad = utf8.encode('jianxin-config:v1:pbkdf2-sha256:600000:aes-256-gcm'); bits = 256;
    } else {
      if (!['aes-256-gcm', 'aes-128-gcm'].includes(method)) throw new Error();
      const packed = decode(text.decode(data).replace(/[ \t\r\n]/g, ''));
      if (packed.length <= 28) throw new Error();
      nonce = packed.subarray(0, 12); encrypted = packed.subarray(12);
      salt = utf8.encode(`jianxin-config:password-key:v2:${method}`);
      aad = utf8.encode(`jianxin-config:v2:pbkdf2-sha256:600000:${method}`);
      bits = method === 'aes-256-gcm' ? 256 : 128;
    }
  } catch {
    passwordBytes.fill(0);
    throw new Error('密文格式无效，请选择生成文件时的加密方式');
  }
  let plaintext;
  try {
    const material = await crypto.subtle.importKey('raw', passwordBytes, 'PBKDF2', false, ['deriveKey']);
    const key = await crypto.subtle.deriveKey({ name: 'PBKDF2', salt, iterations: 600000, hash: 'SHA-256' },
      material, { name: 'AES-GCM', length: bits }, false, ['decrypt']);
    plaintext = new Uint8Array(await crypto.subtle.decrypt({ name: 'AES-GCM', iv: nonce, additionalData: aad, tagLength: 128 }, key, encrypted));
  } catch {
    throw new Error('解密失败：密钥、加密方式不匹配或文件已被修改');
  } finally { passwordBytes.fill(0); }
  try {
    const config = JSON.parse(text.decode(plaintext));
    if (!Array.isArray(config?.accounts) || config.accounts.length < 1 || config.accounts.length > 100) throw new Error();
    return config;
  } catch { throw new Error('解密后的内容不是有效邮箱配置，需要包含 1 至 100 个邮箱'); }
  finally { plaintext.fill(0); }
}
