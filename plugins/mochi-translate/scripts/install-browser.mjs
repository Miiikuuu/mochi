#!/usr/bin/env node
import { createHash } from 'node:crypto';
import { readFileSync, mkdirSync, writeFileSync, existsSync } from 'node:fs';
import { dirname, resolve, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { homedir } from 'node:os';
import { spawnSync } from 'node:child_process';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const manifest = JSON.parse(readFileSync(join(root, 'browser/manifest.json')));
export const extensionId = createHash('sha256').update(Buffer.from(manifest.key, 'base64'))
  .digest('hex').slice(0, 32).replace(/[0-9a-f]/g, char => String.fromCharCode(97 + parseInt(char, 16)));
const index = process.argv.indexOf('--profile-root');
const profile = index >= 0 ? resolve(process.argv[index + 1])
  : join(process.env.XDG_CONFIG_HOME || join(homedir(), '.config'), 'google-chrome');
const launcherDir = join(root, '.local');
mkdirSync(launcherDir, { recursive: true, mode: 0o700 });
const codex = spawnSync('which', ['codex'], { encoding: 'utf8' }).stdout?.trim();
if (!codex) throw new Error('请先安装 Codex 并运行 codex login。');
const quote = value => "'" + value.replaceAll("'", "'\\''") + "'";
const launcher = join(launcherDir, 'native-host');
writeFileSync(launcher, '#!/bin/sh\n' + `export MOCHI_CODEX_BIN=${quote(codex)}\n`
  + `exec ${quote(process.execPath)} ${quote(join(root, 'native.mjs'))} "$@"\n`, { mode: 0o700 });
const hosts = join(profile, 'NativeMessagingHosts');
mkdirSync(hosts, { recursive: true, mode: 0o700 });
const hostFile = join(hosts, 'org.mochi.translate.json');
if (existsSync(hostFile)) {
  const old = JSON.parse(readFileSync(hostFile));
  if (old.path !== launcher) throw new Error(`已有另一份同名插件，未覆盖：${hostFile}`);
}
writeFileSync(hostFile, JSON.stringify({ name: 'org.mochi.translate',
  description: 'Mochi selection translation with Codex', path: launcher, type: 'stdio',
  allowed_origins: [`chrome-extension://${extensionId}/`] }, null, 2) + '\n', { mode: 0o600 });
console.log(JSON.stringify({ extensionId, extensionDirectory: join(root, 'browser'), nativeHost: hostFile }, null, 2));
