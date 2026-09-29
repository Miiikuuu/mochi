// Reloadable MCP worker. The gateway owns the public tunnel and private config.
import { readFile, stat } from 'node:fs/promises';
import { trustedWorker } from './lib/trusted-worker.mjs';
const info = await stat(process.argv[2]);
if (info.uid !== process.getuid() || (info.mode & 0o077)) throw Error('Gateway config must be private');
const { token } = JSON.parse(await readFile(process.argv[2], 'utf8'));
if (!token) throw Error('Missing gateway token');
await trustedWorker(process.argv[2]);
const { listenLocal } = await import('./lib/http.mjs');
const listener = await listenLocal({ port: 0, token, audit: event => console.error(JSON.stringify(event)) });
let stopping = false;
async function stop() {
  if (stopping) return;
  stopping = true;
  await listener.close();
  if (process.connected) process.disconnect();
}
process.on('message', message => { if (message === 'stop') void stop(); });
process.on('disconnect', () => { void stop(); });
process.on('SIGTERM', () => { void stop(); });
const ready = { ready: true, port: Number(new URL(listener.url).port) };
if (process.connected) process.send(ready);
else console.log(JSON.stringify(ready));
