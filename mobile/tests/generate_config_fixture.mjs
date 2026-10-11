// Synthetic interoperability vectors; contains no real account credentials.
import { mkdirSync, writeFileSync } from 'node:fs';
import { encryptConfig, encryptLegacyConfig } from '../tools/config_crypto.mjs';

const password = '测试密码🔑-fixture-only';
const plaintext = JSON.stringify({
  general: { sync_interval: 60 },
  accounts: [
    { id: 'qq', email: 'sample@qq.com', display_name: '测试邮箱', auth: { type: 'app_password', password: 'sample-authorization-code' } },
    { id: 'custom', email: 'sample@example.com', auth: { type: 'password', username: 'shared-login', password: 'sample-password' },
      imap: { host: 'imap.example.com', port: 143, tls: 'starttls', username: 'imap-login' },
      smtp: { host: 'smtp.example.com', port: 587, tls: 'starttls', username: 'smtp-login' },
      folders: { sent: 'Sent Items' } }
  ]
});
const bytes = Buffer.from(plaintext);
const nonce = Buffer.from('101112131415161718191a1b', 'hex');
const fixture = { password, plaintext,
  envelope: JSON.parse(encryptLegacyConfig(bytes, password, Buffer.from('000102030405060708090a0b0c0d0e0f', 'hex'), nonce)),
  'aes-256-gcm': encryptConfig(bytes, password, 'aes-256-gcm', nonce).toString(),
  'aes-128-gcm': encryptConfig(bytes, password, 'aes-128-gcm', nonce).toString()
};
mkdirSync(new URL('fixtures/', import.meta.url), { recursive: true });
writeFileSync(new URL('fixtures/config.json', import.meta.url), JSON.stringify(fixture, null, 2) + '\n');
