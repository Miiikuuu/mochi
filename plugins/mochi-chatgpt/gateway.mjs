// Supervised development gateway. The complete URL is a revocable capability.
import { readFile, writeFile, stat, unlink } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { createServer } from 'node:http';
import reloadable from './lib/reloadable-http.cjs';

const configPath = process.argv[2];
const info = await stat(configPath);
if (info.uid !== process.getuid() || (info.mode & 0o077)) throw new Error('Gateway config must be private to its owner');
const config = JSON.parse(await readFile(configPath, 'utf8'));
if (!config.token) throw new Error('Missing gateway token');
const listener = createServer();
await reloadable.installReload(listener, configPath);
await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve));
const origin = `http://127.0.0.1:${listener.address().port}`;
const tunnel = spawn(config.cloudflared, ['tunnel', '--no-autoupdate', '--protocol', 'http2',
  '--url', origin, '--http-host-header', '127.0.0.1'], { stdio: ['ignore', 'pipe', 'pipe'] });
let stopping = false;
let wrote = false;
const stop = async () => {
  if (stopping) return;
  stopping = true;
  tunnel.kill('SIGTERM');
  await new Promise(resolve => { listener.close(resolve); listener.closeAllConnections(); });
  await unlink(config.state).catch(() => {});
};
for (const stream of [tunnel.stdout, tunnel.stderr]) {
  createInterface({ input: stream }).on('line', line => {
    // Never log tool payloads or the credential-bearing endpoint.
    console.error(line);
    const match = line.match(/https:\/\/[a-z0-9-]+\.trycloudflare\.com/);
    if (match && !wrote && !stopping) {
      wrote = true;
      void writeFile(config.state, JSON.stringify({ url: match[0]+'/mcp/'+config.token,
        created_at: new Date().toISOString(), pid: process.pid, temporary: true, reloadable: true })+'\n', { mode: 0o600 });
    }
  });
}
tunnel.on('error', async error => { console.error(error.message); process.exitCode = 1; await stop(); });
tunnel.on('exit', async code => { if (!stopping) process.exitCode = code || 1; await stop(); });
process.on('SIGINT', () => { void stop(); });
process.on('SIGTERM', () => { void stop(); });
