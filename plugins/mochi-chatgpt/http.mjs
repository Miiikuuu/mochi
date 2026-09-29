import { trustedLaunch } from './lib/trusted-launch.mjs';
await trustedLaunch('card-http');
const { listenLocal } = await import('./lib/http.mjs');

const port = process.env.MOCHI_MCP_PORT === undefined ? 17841 : Number(process.env.MOCHI_MCP_PORT);
const token = process.env.MOCHI_MCP_TOKEN;
if (!/^[a-f0-9]{64}$/.test(token || '')) throw new Error('MOCHI_MCP_TOKEN must be a 256-bit HTTP access token.');
const listener = await listenLocal({ port, token });
console.error(`Mochi MCP listening locally on port ${new URL(listener.url).port} (authenticated).`);
const stop = () => { void listener.close(); };
process.on('SIGINT', stop);
process.on('SIGTERM', stop);
