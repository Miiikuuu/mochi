import { translate, validate, listModels, closeTranslationEngines } from './lib/translation.mjs';
import { Decoder, encode } from './lib/framing.mjs';
import { showTranslation } from './lib/desktop.mjs';

const decoder = new Decoder();
let active, closing = false;
const send = value => { if (!closing) process.stdout.write(encode(value)); };
const stop = () => {
  if (closing) return;
  closing = true;
  active?.abort();
  process.stdin.destroy();
  void closeTranslationEngines();
};
process.stdout.on('error', stop);
process.stdin.on('end', stop);
process.on('SIGTERM', stop);
process.on('SIGINT', stop);
process.stdin.on('data', chunk => {
  try { for (const message of decoder.push(chunk)) void handle(message); }
  catch { stop(); process.exitCode = 1; }
});
async function handle(message) {
  if (closing) return;
  if (message?.type === 'cancel') { active?.abort(); return; }
  const id = message?.id;
  if (typeof id !== 'string' || id.length > 80) return;
  if (active) { send({ id, type: 'error', error: '已有翻译正在进行，请先取消。' }); return; }
  try {
    if (message.type === 'show_translation') {
      if (message.text) validate(message.text, message.target);
      const result = await showTranslation(message.text || '', { target: message.target, model: message.model, fast: message.fast });
      send({ id, type: 'accepted', ...result });
      return;
    }
    if (message.type === 'list_models') {
      const controller = new AbortController();
      active = controller;
      const result = await listModels({ signal: controller.signal });
      send({ id, type: 'models', ...result });
      return;
    }
    if (message.type !== 'translate') throw new Error('不支持的请求。');
    validate(message.text, message.target);
    const controller = new AbortController();
    active = controller;
    const result = await translate(message.text, message.target, {
      model: message.model, fast: message.fast,
      signal: controller.signal,
      onStatus: status => send({ id, type: 'status', status }),
    });
    send({ id, type: 'result', ...result });
  } catch (error) {
    send({ id, type: 'error', error: error.message });
  } finally { active = undefined; }
}
