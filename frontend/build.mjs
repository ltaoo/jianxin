import { createRequire } from 'node:module';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = dirname(fileURLToPath(import.meta.url));
const timelessRoot = process.env.TIMELESS_ROOT || resolve(root, '../../timeless');
let esbuild;
try { esbuild = createRequire(import.meta.url)('esbuild'); }
catch { esbuild = createRequire(resolve(timelessRoot, 'node_modules/.pnpm/node_modules/resolve.cjs'))('esbuild'); }
const contract = JSON.parse(await readFile(resolve(root, '.design'), 'utf8'));
const declarations = Object.entries(contract.tokens).flatMap(([group, values]) =>
  Object.entries(values).filter(([, value]) => typeof value === 'string')
    .map(([key, value]) => `--jx-${group}-${key}:${value};`));
const tokens = `/* Generated from frontend/.design. */\n:root{${declarations.join('')}}\n`;
const source = (await readFile(resolve(root, 'packages/jxui/style.css'), 'utf8'))
  .replace(/\{\{([\w]+)\.([\w]+)\}\}/g, (_, group, key) => contract.tokens[group][key]);
const css = (await esbuild.transform(tokens + source, { loader: 'css', minify: true })).code;
const bundle = await esbuild.build({ entryPoints: [resolve(root, 'src/main.js')], bundle: true,
  loader: { '.design': 'json' }, format: 'iife', target: ['es2022'], minify: true, write: false, legalComments: 'inline' });
const runtimes = await Promise.all(['timeless.lite.umd.min.js', 'timeless.dom.umd.min.js']
  .map(file => readFile(resolve(root, 'vendor', file), 'utf8')));
const js = '/* Timeless 0.34.0, MIT; see frontend/vendor/TIMELESS-LICENSE. */\n'
  + runtimes.join('\n;\n') + '\n;\n' + bundle.outputFiles[0].text;
await mkdir(resolve(root, 'dist'), { recursive: true });
for (const [name, content] of [['app.js', js], ['app.css', css]]) {
  const path = resolve(root, 'dist', name);
  if (process.argv.includes('--check')) {
    if (await readFile(path, 'utf8') !== content) throw new Error(`${name} is stale; run npm run build`);
  } else await writeFile(path, content);
  console.log(`${name}: ${(Buffer.byteLength(content) / 1024).toFixed(1)} KiB`);
}
console.log('Design tokens and build artifacts verified.');
