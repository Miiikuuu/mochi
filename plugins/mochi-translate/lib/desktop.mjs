import { createConnection } from 'node:net';
import { createHash } from 'node:crypto';
import { join } from 'node:path';

export function desktopSocket() {
  if (process.env.MOCHI_BUBBLE_SOCKET) return process.env.MOCHI_BUBBLE_SOCKET;
  const display = createHash('sha256').update(process.env.DISPLAY || ':0').digest('hex').slice(0, 12);
  const runtime = process.env.XDG_RUNTIME_DIR || `/tmp/mochi-${process.getuid()}`;
  return join(runtime, 'mochi', `bubbles-${display}.sock`);
}
export function showTranslation(text = '', options = {}) {
  return new Promise((resolve, reject) => {
    const socket = createConnection(desktopSocket());
    socket.setEncoding('utf8');
    let data = '';
    const timer = setTimeout(() => finish(new Error('桌宠暂时没有响应，请重试。')), 3000);
    let settled = false;
    function finish(error, result) {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      socket.destroy();
      if (error) reject(error); else resolve(result);
    }
    socket.on('error', () => finish(new Error('请先启动 Mochi 桌宠，再使用右键翻译。')));
    socket.on('end', () => finish(new Error('桌宠连接已关闭。')));
    socket.on('connect', () => socket.write(JSON.stringify({ type: 'translate', text, ...options }) + '\n'));
    socket.on('data', chunk => {
      data += chunk;
      if (data.length > 8192) return finish(new Error('桌宠返回了无效响应。'));
      if (!data.includes('\n')) return;
      try {
        const result = JSON.parse(data.split('\n', 1)[0]);
        if (result.error) finish(new Error(result.error));
        else if (result.accepted === true) finish(undefined, result);
        else finish(new Error('桌宠没有接收翻译请求。'));
      } catch { finish(new Error('桌宠返回了无效响应。')); }
    });
  });
}
