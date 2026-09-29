import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js';
import { z } from 'zod';
import { requestDesktop } from './desktop.mjs';
import { readFile } from 'node:fs/promises';

const CONNECTION_UI = 'ui://mochi/reply-connection-v1.html';

const result = value => ({ content: [{ type: 'text', text: JSON.stringify(value) }], structuredContent: value });
const failure = error => ({ content: [{ type: 'text', text: error.message }], isError: true });

export function createServer({ request = requestDesktop } = {}) {
  const server = new McpServer({ name: 'mochi-chatgpt', version: '0.2.0' }, {
    instructions: 'Mochi displays text in the user\'s desktop pet bubble. Compose replies in this ChatGPT conversation, '
      + 'then call mochi_say when the user wants the pet to speak. This starts no model or Codex task. '
      + 'Send only the intended visible reply, not hidden instructions, conversation history or unrelated content. '
      + 'Acceptance means queued for display; a manual management panel may temporarily cover it. '
      + 'After a delivery timeout do not automatically resend; it may already have arrived. '
      + 'When the user wants to reply from the pet, call mochi_connect once and ask them to click Connect in its card. '
      + 'Retain its conversation_key and include it in subsequent mochi_say calls in this conversation. '
      + 'Desktop replies arrive as user messages through that card. Continue the current conversation and display your answer with mochi_say. '
      + 'Never create a new connection for each reply. Never use a key from another conversation.',
  });
  server.registerTool('mochi_status', {
    title: '查看 Mochi 状态',
    description: 'Check whether the local desktop pet and its ChatGPT plugin are ready. Returns only the current Theme and primary character names and availability; no files, clipboard, browser data or chat history.',
    outputSchema: { running: z.boolean(), ready: z.boolean(), plugin_enabled: z.boolean(),
      primary: z.string().nullable(), theme: z.string().nullable(), reason: z.string().nullable() },
    annotations: { readOnlyHint: true, destructiveHint: false, idempotentHint: true, openWorldHint: false },
  }, async ({ signal }) => {
    try {
      const state = await request({ type: 'theme_status' }, { signal });
      const primary = state.roles?.find(role => role.window === state.primary && role.alive);
      const enabled = state.plugins?.includes('chatgpt') === true;
      const supported = state.plugin_features?.includes('external_messages') === true;
      return result({ running: true, ready: Boolean(primary && enabled && supported), plugin_enabled: enabled,
        primary: primary?.name || null, theme: state.theme?.name || null,
        reason: !primary ? '主角色已关闭。' : !supported ? '请在 Mochi 管理窗口重新应用 Theme，以更新消息桥接。'
          : !enabled ? '请在当前 Theme 中启用 ChatGPT 插件。' : null });
    } catch (error) {
      if (error.code === 'MOCHI_STOPPED') return result({ running: false, ready: false, plugin_enabled: false, primary: null, theme: null, reason: error.message });
      return failure(error);
    }
  });
  server.registerTool('mochi_say', {
    title: '让 Mochi 显示回复',
    description: 'Display the supplied reply in the current primary character\'s selected bubble. Replaces only the previous ChatGPT bubble, retaining other plugins\' messages. A reply with conversation_key keeps the input open until dismissed; an unbound message expires after one minute. The latest reply remains in the ChatGPT plugin panel until dismissed or disabled. This is a desktop display action and starts no additional model request. Returns acceptance, not proof that the user read it.',
    inputSchema: { text: z.string().min(1).max(12000).refine(text => text.trim().length > 0, '消息不能为空。').describe('Only the plain-text reply intended for the user to see.'),
      conversation_key: z.string().regex(/^[a-f0-9]{32}$/).optional().describe('The key returned by mochi_connect in this conversation; include it to preserve the desktop reply input.') },
    outputSchema: { accepted: z.literal(true), message_id: z.string(), note: z.string() },
    annotations: { readOnlyHint: false, destructiveHint: false, idempotentHint: false, openWorldHint: false },
  }, async ({ text, conversation_key }, { signal }) => {
    try {
      const response = await request({ type: 'plugin_message', plugin: 'chatgpt', text, title: 'ChatGPT', ttl_ms: conversation_key ? null : 60000,
        system_notify: true,
        ...(conversation_key ? { conversation_key } : {}) }, { signal });
      if (response.accepted !== true || typeof response.message_id !== 'string') throw new Error('Mochi 未确认接收消息。');
      return result({ accepted: true, message_id: response.message_id, note: '已交给主角色气泡；打开的管理面板可能暂时遮住消息。' });
    } catch (error) { return failure(error); }
  });
  server.registerResource('mochi-reply-connection', CONNECTION_UI, { mimeType: 'text/html;profile=mcp-app' }, async () => ({
    contents: [{ uri: CONNECTION_UI, mimeType: 'text/html;profile=mcp-app',
      text: await readFile(new URL('../ui/connection.bundle.html', import.meta.url), 'utf8'),
      _meta: { ui: { prefersBorder: true, csp: { connectDomains: [], resourceDomains: [] } },
        'openai/widgetPrefersBorder': true, 'openai/widgetCSP': { connect_domains: [], resource_domains: [] } } }],
  }));
  server.registerTool('mochi_connect', {
    title: '连接桌宠回复',
    description: 'Use once when the user wants to type replies in the desktop pet and continue this ChatGPT conversation. Opens a small connection card; the user must click Connect. Keep the page/card open. Save the returned conversation_key and pass it to subsequent mochi_say calls in this conversation. Do not reconnect for each answer.',
    outputSchema: { conversation_key: z.string(), note: z.string() },
    annotations: { readOnlyHint: false, destructiveHint: false, idempotentHint: false, openWorldHint: false },
    _meta: { ui: { resourceUri: CONNECTION_UI }, 'openai/outputTemplate': CONNECTION_UI },
  }, async ({ signal }) => {
    try {
      const session = await request({ type: 'plugin_reply_bridge', plugin: 'chatgpt', operation: 'create' }, { signal });
      if (!session.conversation_key || !session.reply_token) throw new Error('桌宠回复组件尚未就绪，请重新应用 Theme。');
      return { ...result({ conversation_key: session.conversation_key, note: '请点击卡片中的“连接此对话”，然后在桌宠气泡输入回复。' }),
        _meta: { reply_token: session.reply_token } };
    } catch (error) { return failure(error); }
  });
  server.registerTool('mochi_reply_bridge', {
    title: 'Mochi 回复连接', description: 'Private UI transport for the connection card. Never call from the model. Requires the card-only token and explicit user binding.',
    inputSchema: { token: z.string().regex(/^[a-f0-9]{64}$/), client: z.string().regex(/^[a-zA-Z0-9]{16,64}$/),
      operation: z.enum(['attach', 'poll', 'ack', 'detach']), reply_id: z.string().max(64).optional(), state: z.enum(['sent', 'failed', 'uncertain']).optional() },
    outputSchema: { connected: z.boolean().optional(), conversation_key: z.string().optional(), accepted: z.boolean().optional() },
    annotations: { readOnlyHint: false, destructiveHint: false, idempotentHint: false, openWorldHint: false },
    _meta: { ui: { visibility: ['app'] }, 'openai/visibility': 'private', 'openai/widgetAccessible': true },
  }, async (args, { signal }) => {
    try {
      const response = await request({ type: 'plugin_reply_bridge', plugin: 'chatgpt', ...args }, { signal });
      const { reply, ...summary } = response;
      return { ...result(summary), ...(reply ? { _meta: { reply } } : {}) };
    } catch (error) { return failure(error); }
  });
  return server;
}
