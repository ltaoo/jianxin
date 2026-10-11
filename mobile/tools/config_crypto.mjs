#!/usr/bin/env node
// Desktop publishing helper. Only Node built-ins; nothing is bundled into the apps.
import { pbkdf2Sync, randomBytes, createCipheriv, createDecipheriv } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { Writable } from 'node:stream';

export const AAD = Buffer.from('jianxin-config:v1:pbkdf2-sha256:600000:aes-256-gcm');
export const LIMIT = 2 * 1024 * 1024;
export function encryptLegacyConfig(plaintext, password, salt = randomBytes(16), nonce = randomBytes(12)) {
  if (!password || Buffer.byteLength(password) > 1024) throw new Error('解密密码不能为空，最多 1024 字节');
  if (!plaintext.length || plaintext.length > 1400000) throw new Error('原始配置不能为空，最多 1.4 MB');
  const config = JSON.parse(plaintext.toString('utf8'));
  if (!Array.isArray(config.accounts) || config.accounts.length < 1 || config.accounts.length > 100) throw new Error('配置需要包含 1 至 100 个账号');
  const key = pbkdf2Sync(Buffer.from(password, 'utf8'), salt, 600000, 32, 'sha256');
  try {
    const cipher = createCipheriv('aes-256-gcm', key, nonce);
    cipher.setAAD(AAD);
    const ciphertext = Buffer.concat([cipher.update(plaintext), cipher.final()]);
    return Buffer.from(JSON.stringify({ format: 'jianxin-config', version: 1, kdf: 'pbkdf2-sha256', iterations: 600000, cipher: 'aes-256-gcm',
      salt: salt.toString('base64'), nonce: nonce.toString('base64'), ciphertext: ciphertext.toString('base64'), tag: cipher.getAuthTag().toString('base64') }) + '\n');
  } finally { key.fill(0); }
}
export function decryptLegacyConfig(encrypted, password) {
  if (encrypted.length > LIMIT) throw new Error('加密配置不能超过 2 MB');
  const e = JSON.parse(encrypted.toString('utf8'));
  if (e.format !== 'jianxin-config' || e.version !== 1 || e.kdf !== 'pbkdf2-sha256' || e.iterations !== 600000 || e.cipher !== 'aes-256-gcm') throw new Error('不支持的加密格式');
  const salt = Buffer.from(e.salt, 'base64'), nonce = Buffer.from(e.nonce, 'base64'), tag = Buffer.from(e.tag, 'base64');
  if (salt.length !== 16 || nonce.length !== 12 || tag.length !== 16) throw new Error('加密配置损坏');
  const key = pbkdf2Sync(Buffer.from(password, 'utf8'), salt, 600000, 32, 'sha256');
  try {
    const cipher = createDecipheriv('aes-256-gcm', key, nonce);
    cipher.setAAD(AAD); cipher.setAuthTag(tag);
    return Buffer.concat([cipher.update(Buffer.from(e.ciphertext, 'base64')), cipher.final()]);
  } finally { key.fill(0); }
}
function methodSettings(method) {
  if (!['aes-256-gcm', 'aes-128-gcm'].includes(method)) throw new Error('请选择 AES-256-GCM 或 AES-128-GCM');
  return { size: method === 'aes-256-gcm' ? 32 : 16,
    context: Buffer.from(`jianxin-config:password-key:v2:${method}`),
    aad: Buffer.from(`jianxin-config:v2:pbkdf2-sha256:600000:${method}`) };
}
export function encryptConfig(plaintext, password, method = 'aes-256-gcm', nonce = randomBytes(12)) {
  if (!password || Buffer.byteLength(password) > 1024) throw new Error('解密密码不能为空，最多 1024 字节');
  if (!plaintext.length || plaintext.length > 1400000) throw new Error('原始配置不能为空，最多 1.4 MB');
  const config = JSON.parse(plaintext.toString('utf8'));
  if (!Array.isArray(config.accounts) || config.accounts.length < 1 || config.accounts.length > 100) throw new Error('配置需要包含 1 至 100 个账号');
  const settings = methodSettings(method);
  if (nonce.length !== 12) throw new Error('nonce 长度必须为 12 字节');
  const key = pbkdf2Sync(Buffer.from(password, 'utf8'), settings.context, 600000, settings.size, 'sha256');
  try {
    const cipher = createCipheriv(method, key, nonce); cipher.setAAD(settings.aad);
    const ciphertext = Buffer.concat([cipher.update(plaintext), cipher.final()]);
    // No metadata fields, per-file salt, algorithm identifier or format marker.
    return Buffer.from(Buffer.concat([nonce, ciphertext, cipher.getAuthTag()]).toString('base64') + '\n');
  } finally { key.fill(0); }
}
export function decryptConfig(encrypted, password, method = 'aes-256-gcm') {
  if (method === 'legacy-json') return decryptLegacyConfig(encrypted, password);
  if (!password || Buffer.byteLength(password) > 1024) throw new Error('解密密码不能为空，最多 1024 字节');
  if (encrypted.length > LIMIT) throw new Error('加密配置不能超过 2 MB');
  const encoded = new TextDecoder('utf-8', { fatal: true }).decode(encrypted).replace(/[ \t\r\n]/g, '');
  const packed = Buffer.from(encoded, 'base64');
  if (packed.length <= 28 || packed.toString('base64') !== encoded) throw new Error('密文必须是完整的 Base64 内容');
  const settings = methodSettings(method);
  const key = pbkdf2Sync(Buffer.from(password, 'utf8'), settings.context, 600000, settings.size, 'sha256');
  try {
    const cipher = createDecipheriv(method, key, packed.subarray(0, 12));
    cipher.setAAD(settings.aad); cipher.setAuthTag(packed.subarray(-16));
    return Buffer.concat([cipher.update(packed.subarray(12, -16)), cipher.final()]);
  } finally { key.fill(0); }
}
async function hiddenPassword(prompt) {
  if (!process.stdin.isTTY || !process.stderr.isTTY) throw new Error('需要终端输入密码；自动化可通过 JIANXIN_CONFIG_PASSWORD 环境变量提供');
  const silent = new Writable({ write(_chunk, _encoding, callback) { callback(); } });
  silent.isTTY = true;
  const input = createInterface({ input: process.stdin, output: silent, terminal: true, historySize: 0 });
  process.stderr.write(prompt);
  try { return await new Promise(resolve => input.question('', resolve)); }
  finally { input.close(); process.stderr.write('\n'); }
}
async function main() {
  const [mode, source, destination, ...options] = process.argv.slice(2);
  let force = false, method = 'aes-256-gcm';
  for (let i = 0; i < options.length; i++) {
    if (options[i] === '--force') force = true;
    else if (options[i] === '--cipher' && options[i + 1]) method = options[++i];
    else throw new Error('参数无效：支持 --force 和 --cipher aes-256-gcm|aes-128-gcm');
  }
  if (mode !== 'encrypt' || !source || !destination) throw new Error('用法：node mobile/tools/config_crypto.mjs encrypt config.json config.enc.txt [--cipher aes-256-gcm|aes-128-gcm] [--force]');
  methodSettings(method);
  if (resolve(source) === resolve(destination)) throw new Error('输出路径不能覆盖原始 config.json');
  const raw = readFileSync(source);
  let password = process.env.JIANXIN_CONFIG_PASSWORD;
  if (password === undefined) {
    password = await hiddenPassword('解密密码（输入不回显）：');
    if (password !== await hiddenPassword('再次输入密码：')) throw new Error('两次密码不一致');
  }
  const encrypted = encryptConfig(raw, password, method);
  // Check the publishing tool's output before writing anything.
  const verified = decryptConfig(encrypted, password, method);
  try { if (!verified.equals(raw)) throw new Error('加密校验失败'); }
  finally { verified.fill(0); raw.fill(0); password = ''; }
  writeFileSync(destination, encrypted, { mode: 0o600, flag: force ? 'w' : 'wx' });
  process.stdout.write(`已生成纯 Base64 密文。在手机中选择 ${method.toUpperCase()}，填入 HTTPS URL 和解密密码。\n`);
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch(error => { process.stderr.write((error.code === 'EEXIST' ? '输出文件已存在，更新时添加 --force' : error instanceof SyntaxError ? '原始配置不是有效 JSON' : error.message) + '\n'); process.exitCode = 1; });
}
