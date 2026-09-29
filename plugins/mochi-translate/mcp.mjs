import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js';
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import { z } from 'zod';
import { translate, listModels, closeTranslationEngines, LANGUAGES, MAX_TEXT } from './lib/translation.mjs';
import { showTranslation } from './lib/desktop.mjs';

const server = new McpServer({ name: 'mochi-translate', version: '0.1.0' }, {
  instructions: 'Translate only text the user supplies or explicitly selects. '
    + 'Each request uses an independent Codex thread. Do not send whole pages or files unless requested.',
});
let busy = false;
const shutdown = new AbortController();
const stop = () => { shutdown.abort(); void closeTranslationEngines(); void server.close(); };
process.stdin.on('end', stop);
process.on('SIGTERM', stop);
process.on('SIGINT', stop);
server.registerTool('translate_text', {
  title: 'Mochi 翻译',
  description: 'Translate supplied text using the local signed-in Codex account in a separate ephemeral session. '
    + 'Defaults to GPT-5.6 Luna with Standard speed. Use list_translation_models for available models. '
    + 'Fast is opt-in, where supported, and consumes more allowance. '
    + 'Sends only this text to the configured Codex provider and consumes its usage allowance. '
    + 'Does not read browser pages, clipboard, files, or other conversations.',
  inputSchema: { text: z.string().min(1).max(MAX_TEXT),
    target_language: z.enum(Object.keys(LANGUAGES)).default('zh-CN'),
    model: z.string().min(1).max(128).optional(), fast: z.boolean().optional() },
  annotations: { readOnlyHint: true, destructiveHint: false, openWorldHint: true },
}, async ({ text, target_language, model, fast }, extra) => {
  if (busy) return { content: [{ type: 'text', text: '翻译忙，请稍后重试。' }], isError: true };
  busy = true;
  try {
    const result = await translate(text, target_language, {
      model, fast,
      signal: AbortSignal.any([extra.signal, shutdown.signal]),
    });
    return { content: [{ type: 'text', text: result.translated_text }], structuredContent: result };
  } catch (error) {
    return { content: [{ type: 'text', text: error.message }], isError: true };
  } finally { busy = false; }
});
server.registerTool('translate_on_desktop', {
  title: '交给 Mochi 桌宠翻译',
  description: 'Send only supplied text to the running Mochi desktop pet. The pet owns a separate Codex translation task and displays progress and the result in its bubble. Returns acceptance, not the finished translation. Close the bubble or press Cancel there to stop the task. Uses the signed-in Codex allowance.',
  inputSchema: { text: z.string().min(1).max(MAX_TEXT),
    target_language: z.enum(Object.keys(LANGUAGES)).optional(),
    model: z.string().min(1).max(128).optional(), fast: z.boolean().optional() },
  annotations: { readOnlyHint: false, destructiveHint: false, openWorldHint: true },
}, async ({ text, target_language, model, fast }) => {
  try {
    await showTranslation(text, { target: target_language, model, fast });
    return { content: [{ type: 'text', text: '桌宠已接收翻译任务，请在角色旁的气泡查看结果。' }] };
  } catch (error) {
    return { content: [{ type: 'text', text: error.message }], isError: true };
  }
});
server.registerTool('list_translation_models', {
  title: '可用翻译模型',
  description: 'Read the signed-in Codex model catalog and Fast availability without starting a translation.',
  inputSchema: {},
  annotations: { readOnlyHint: true, destructiveHint: false, openWorldHint: true },
}, async (_, extra) => {
  try {
    const result = await listModels({ signal: AbortSignal.any([extra.signal, shutdown.signal]) });
    return { content: [{ type: 'text', text: JSON.stringify(result) }], structuredContent: result };
  } catch (error) {
    return { content: [{ type: 'text', text: error.message }], isError: true };
  }
});
await server.connect(new StdioServerTransport());
