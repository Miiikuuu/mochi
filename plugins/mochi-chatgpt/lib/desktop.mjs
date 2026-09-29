import { createConnection } from 'node:net';
import { createHash } from 'node:crypto';
import { join } from 'node:path';
import { requestSession } from './session.mjs';

export function desktopSocket() {
  if (process.env.MOCHI_BUBBLE_SOCKET) return process.env.MOCHI_BUBBLE_SOCKET;
  const display = createHash('sha256').update(process.env.DISPLAY || ':0').digest('hex').slice(0, 12);
  return join(process.env.XDG_RUNTIME_DIR || `/tmp/mochi-${process.getuid()}`, 'mochi', `bubbles-${display}.sock`);
}

export function requestDesktop(request, { signal, path = desktopSocket(), timeout = 3000 } = {}) {
  if (request.type === 'plugin_reply_bridge' && request.plugin === 'chatgpt') {
    return requestSession(request, { signal, timeout });
  }
  return new Promise((resolve, reject) => {
    if (signal?.aborted) return reject(new Error('请求已取消。'));
    const frame = JSON.stringify(request) + '\n';
    if (Buffer.byteLength(frame) > 65536) return reject(new Error('消息过长。'));
    const socket = createConnection(path);
    socket.setEncoding('utf8');
    let data = '', bytes = 0, settled = false;
    const timer = setTimeout(() => finish(new Error('Mochi 响应超时；消息可能已送达，请先查看桌宠，避免重复发送。')), timeout);
    const abort = () => finish(new Error('请求已取消；消息可能已送达。'));
    function finish(error, result) {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      signal?.removeEventListener('abort', abort);
      socket.destroy();
      if (error) reject(error); else resolve(result);
    }
    signal?.addEventListener('abort', abort, { once: true });
    socket.on('error', error => {
      const stopped = ['ENOENT', 'ECONNREFUSED'].includes(error.code);
      finish(Object.assign(new Error(stopped ? '请先启动 Mochi 桌宠。' : 'Mochi 连接失败。'), { code: stopped ? 'MOCHI_STOPPED' : 'MOCHI_CONNECTION' }));
    });
    socket.on('close', () => finish(new Error('Mochi 连接已关闭。')));
    socket.on('connect', () => socket.write(frame));
    socket.on('data', chunk => {
      bytes += Buffer.byteLength(chunk);
      if (bytes > 262144) return finish(new Error('Mochi 返回的数据过长。'));
      data += chunk;
      if (!data.includes('\n')) return;
      try {
        const result = JSON.parse(data.split('\n', 1)[0]);
        if (!result || Array.isArray(result) || typeof result !== 'object') throw new Error();
        if (result.error) return finish(new Error(String(result.error).slice(0, 1000)));
        finish(undefined, result);
      } catch { finish(new Error('Mochi 返回了无效响应。')); }
    });
  });
}
