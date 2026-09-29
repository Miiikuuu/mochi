import { trustedLaunch } from './lib/trusted-launch.mjs';
await trustedLaunch('card-stdio');
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
const { createServer } = await import('./lib/server.mjs');

const server = createServer();
const stop = () => { void server.close(); };
process.stdin.on('end', stop);
process.on('SIGINT', stop);
process.on('SIGTERM', stop);
await server.connect(new StdioServerTransport());
