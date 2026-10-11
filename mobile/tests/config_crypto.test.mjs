import { strict as assert } from 'node:assert';
import { readFileSync, writeFileSync, mkdtempSync, statSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { encryptConfig, decryptConfig, encryptLegacyConfig, decryptLegacyConfig } from '../tools/config_crypto.mjs';
const fixture = JSON.parse(readFileSync(new URL('fixtures/config.json', import.meta.url)));
const bytes = Buffer.from(fixture.plaintext), encrypted = Buffer.from(JSON.stringify(fixture.envelope));
assert.deepEqual(decryptLegacyConfig(encrypted, fixture.password), bytes);
assert.deepEqual(JSON.parse(encryptLegacyConfig(bytes, fixture.password, Buffer.from(fixture.envelope.salt, 'base64'), Buffer.from(fixture.envelope.nonce, 'base64'))), fixture.envelope);
assert.throws(() => decryptLegacyConfig(encrypted, 'wrong'));
const modified = { ...fixture.envelope, ciphertext: Buffer.from('changed').toString('base64') };
assert.throws(() => decryptLegacyConfig(Buffer.from(JSON.stringify(modified)), fixture.password));
assert.throws(() => decryptLegacyConfig(Buffer.from(JSON.stringify({ ...fixture.envelope, iterations: 1 })), fixture.password));
for (const method of ['aes-256-gcm', 'aes-128-gcm']) {
  const ciphertext = Buffer.from(fixture[method]);
  assert.deepEqual(decryptConfig(ciphertext, fixture.password, method), bytes);
  assert.deepEqual(encryptConfig(bytes, fixture.password, method, Buffer.from('101112131415161718191a1b', 'hex')), ciphertext);
  assert.match(ciphertext.toString('utf8').trim(), /^[A-Za-z0-9+/]+={0,2}$/);
  assert.throws(() => decryptConfig(ciphertext, 'wrong', method));
  assert.throws(() => decryptConfig(ciphertext, fixture.password, method === 'aes-256-gcm' ? 'aes-128-gcm' : 'aes-256-gcm'));
  const packed = Buffer.from(ciphertext.toString('utf8'), 'base64');
  for (const offset of [0, 12, packed.length - 1]) {
    const changed = Buffer.from(packed); changed[offset] ^= 1;
    assert.throws(() => decryptConfig(Buffer.from(changed.toString('base64')), fixture.password, method));
  }
}
assert.throws(() => decryptConfig(encrypted, fixture.password)); // no automatic legacy fallback
assert.throws(() => decryptConfig(Buffer.from('not-base64?!'), fixture.password));
const random1 = encryptConfig(bytes, fixture.password), random2 = encryptConfig(bytes, fixture.password);
assert.notDeepEqual(random1, random2);
assert.deepEqual(decryptConfig(random1, fixture.password), bytes);
assert.throws(() => encryptConfig(bytes, ''));
let directory;
try { directory = mkdtempSync(join(tmpdir(), 'jianxin-config-cli-')); }
catch (error) {
  if (error.code !== 'EACCES') throw error;
  directory = mkdtempSync('/tmp/jianxin-config-cli-');
}
try {
  const source = join(directory, 'config.json'), destination = join(directory, 'config.enc.json');
  writeFileSync(source, bytes, { mode: 0o600 });
  const tool = fileURLToPath(new URL('../tools/config_crypto.mjs', import.meta.url));
  const run = (...args) => spawnSync(process.execPath, [tool, ...args], { env: { ...process.env, JIANXIN_CONFIG_PASSWORD: fixture.password }, encoding: 'utf8' });
  const result = run('encrypt', source, destination);
  assert.equal(result.status, 0, result.stderr);
  assert.equal(statSync(destination).mode & 0o777, 0o600);
  assert.deepEqual(decryptConfig(readFileSync(destination), fixture.password), bytes);
  assert.ok(!result.stdout.includes(fixture.password) && !result.stderr.includes(fixture.password));
  assert.equal(run('encrypt', source, destination).status, 1);
  assert.equal(run('encrypt', source, destination, '--cipher', 'aes-128-gcm', '--force').status, 0);
  assert.deepEqual(decryptConfig(readFileSync(destination), fixture.password, 'aes-128-gcm'), bytes);
  assert.equal(run('encrypt', source, source, '--force').status, 1);
  assert.deepEqual(readFileSync(source), bytes);
} finally { rmSync(directory, { recursive: true, force: true }); }
console.log('Publishing crypto: Unicode password, known vector, wrong password, tampering, algorithm selection, nonce/ciphertext/tag tampering, pure ciphertext, CLI and random nonce passed');
