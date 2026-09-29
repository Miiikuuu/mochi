// Versioned local session relay. No model calls or persistent reply credentials.
import { createConnection } from 'node:net';
import { join } from 'node:path';
import { pythonHelper } from './layout.mjs';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { lstat } from 'node:fs/promises';
import { readFileSync, closeSync } from 'node:fs';
let authority = {};
if (process.env.MOCHI_SESSION_FD) {
  const fd = Number(process.env.MOCHI_SESSION_FD);
  authority = JSON.parse(readFileSync(fd, 'utf8'));
  closeSync(fd);
  delete process.env.MOCHI_SESSION_FD;
}


export function sessionSocket() {
  return join(process.env.XDG_RUNTIME_DIR || `/tmp/mochi-${process.getuid()}`, 'mochi', 'session-v1.sock');
}

export async function requestSession(value, { signal, timeout = 3000 } = {}) {
  let ready = value.operation !== 'create';
  if (!ready) {
    try { ready = Boolean((await requestSession({ operation: 'status' }, { signal, timeout })).generation); }
    catch { if (signal?.aborted) throw new Error('Session request cancelled.'); }
  }
  if (!ready) {
    await promisify(execFile)('/usr/bin/python3', [pythonHelper('session_client.py')],
      { timeout: 5000, signal });
  }
  const path = sessionSocket();
  let info;
  try { info = await lstat(path); } catch { throw new Error('Mochi session disconnected.'); }
  if (!info.isSocket() || info.uid !== process.getuid() || (info.mode & 0o777) !== 0o600) {
    throw new Error('Unsafe session socket.');
  }
  const frame = JSON.stringify({ version: 2, ...authority, scope: 'chatgpt', operation: value.operation === 'ack' ? 'acknowledge' : value.operation,
    token: value.token, client: value.client, reply_id: value.reply_id, state: value.state })+'\n';
  if (Buffer.byteLength(frame) > 65536) throw new Error('Session request too large.');
  return new Promise((resolve, reject) => {
    if (signal?.aborted) return reject(new Error('Session request cancelled.'));
    const socket = createConnection(path);
    let buffer = Buffer.alloc(0), done = false;
    const abort = () => finish(new Error('Session request cancelled; delivery may be uncertain.'));
    const timer = setTimeout(() => finish(new Error('Session request timed out; delivery may be uncertain.')), timeout);
    function finish(error, result) {
      if (done) return;
      done = true;
      clearTimeout(timer);
      signal?.removeEventListener('abort', abort);
      socket.destroy();
      if (error) reject(error); else resolve(result);
    }
    signal?.addEventListener('abort', abort, { once: true });
    socket.on('connect', () => socket.write(frame));
    socket.on('error', () => finish(new Error('Mochi session disconnected. Reconnect the conversation card.')));
    socket.on('close', () => finish(new Error('Mochi session disconnected.')));
    socket.on('data', chunk => {
      buffer = Buffer.concat([buffer, chunk]);
      if (buffer.length > 65536) return finish(new Error('Invalid session response.'));
      const end = buffer.indexOf(10);
      if (end < 0) return;
      try {
        const reply = JSON.parse(buffer.subarray(0, end).toString('utf8'));
        if (reply.version !== 2 || reply.error?.code === 'session.version_mismatch') {
          finish(new Error('session.version_mismatch: upgrade or restart Mochi sessiond and the bridge.')); return;
        }
        if (reply.error || !reply.result) throw new Error();
        finish(null, reply.result);
      } catch { finish(new Error('Session binding expired or request rejected.')); }
    });
  });
}
