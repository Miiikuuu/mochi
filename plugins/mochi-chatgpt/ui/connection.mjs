import { App } from '@modelcontextprotocol/ext-apps';
import { ReplyRelay } from '../lib/relay.mjs';

const status = document.querySelector('#status');
const button = document.querySelector('#connect');
const app = new App({ name: 'Mochi reply connection', version: '0.2.0' }, {});
const client = crypto.randomUUID().replaceAll('-', '');
let token, conversation, relay, ready = false;
app.ontoolresult = result => {
  token = result._meta?.reply_token;
  conversation = result.structuredContent?.conversation_key;
  button.disabled = !(ready && token && conversation);
};
app.onteardown = async () => { await relay?.stop(); return {}; };
button.addEventListener('click', async () => {
  button.disabled = true;
  try {
    if (relay?.running) {
      await relay.stop(); button.textContent = '连接此对话';
    } else {
      relay = new ReplyRelay({ token, client,
        changed: text => { status.textContent = text; },
        call: arguments_ => app.callServerTool({ name: 'mochi_reply_bridge', arguments: arguments_ }, { timeout: 15000 }),
        send: async text => {
          if (app.getHostCapabilities()?.updateModelContext) await app.updateModelContext({ structuredContent: { mochi_conversation_key: conversation,
            instruction: 'The user is replying from the Mochi desktop pet. Continue this conversation, then call mochi_say with this conversation_key and your reply.' } }, { timeout: 10000 }).catch(() => {});
          return app.sendMessage({ role: 'user', content: [{ type: 'text', text }] }, { timeout: 15000 });
        },
      });
      await relay.start(); button.textContent = '断开';
    }
  } catch (error) { status.textContent = error.message || '连接失败，请重新打开连接卡片。'; }
  finally { button.disabled = !(token && conversation); }
});
try {
  await app.connect();
  if (!app.getHostCapabilities()?.serverTools || !app.getHostCapabilities()?.message) {
    throw new Error('Host does not support replies');
  }
  ready = true;
  button.disabled = !(token && conversation);
  status.textContent = '连接后可在桌宠气泡回复。若 ChatGPT 弹出 Send follow-up?，请确认发送；保持此页面打开。';
} catch { status.textContent = '此页面未提供 ChatGPT 连接能力，请在 ChatGPT 中调用 mochi_connect。'; }
