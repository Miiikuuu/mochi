// Stable localhost MCP endpoint for a private SSH reverse tunnel to a VPS.
import { readFile, stat } from 'node:fs/promises';
import { listenLocal } from './lib/http.mjs';

const path = process.argv[2];
if (!path) throw new Error('Expected private VPS gateway config path');
const info = await stat(path);
if (info.uid !== process.getuid() || (info.mode & 0o077)) {
  throw new Error('VPS gateway config must be private to its owner');
}
const { token, port } = JSON.parse(await readFile(path, 'utf8'));
const listener = await listenLocal({ port, token, audit: event => console.error(JSON.stringify(event)) });
console.error(`Mochi MCP VPS upstream ready on 127.0.0.1:${port}`);

let stopping = false;
async function stop() {
  if (stopping) return;
  stopping = true;
  await listener.close();
}
process.on('SIGINT', () => { void stop(); });
process.on('SIGTERM', () => { void stop(); });
