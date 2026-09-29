import { createMcpExpressApp } from '@modelcontextprotocol/sdk/server/express.js';
import { StreamableHTTPServerTransport } from '@modelcontextprotocol/sdk/server/streamableHttp.js';
import { createServer } from './server.mjs';
import { timingSafeEqual } from 'node:crypto';

// Local transport for an authenticated gateway/tunnel. Never listen publicly.
export async function listenLocal({ port = 17841, token, audit = () => {}, serverFactory = createServer } = {}) {
  if (!Number.isInteger(port) || port < 0 || port > 65535) throw new Error('Invalid MCP port');
  if (token !== undefined && !/^[a-f0-9]{64}$/.test(token)) throw new Error('MCP gateway requires a 256-bit token');
  const app = createMcpExpressApp({ host: '127.0.0.1' });
  const active = new Set();
  app.use((req, res, next) => {
    const start = performance.now();
    res.on('finish', () => {
      const method = typeof req.body?.method === 'string' ? req.body.method : null;
      // Allowlist method names: neither message text nor secret URLs enter logs.
      audit({ event: 'mcp_http', method: req.method, status: res.statusCode,
        route: req.path.startsWith('/.well-known/') ? 'discovery' : req.path.startsWith('/mcp/') || req.path === '/mcp' ? 'mcp' : 'other',
        rpc: ['initialize', 'notifications/initialized', 'tools/list', 'tools/call', 'ping'].includes(method) ? method : null,
        origin_present: Boolean(req.headers.origin), protocol: typeof req.headers['mcp-protocol-version'] === 'string' ? req.headers['mcp-protocol-version'].slice(0, 32) : null,
        elapsed_ms: Math.round(performance.now()-start) });
    });
    res.set('Cache-Control', 'no-store').set('Referrer-Policy', 'no-referrer');
    // This endpoint serves native MCP clients, not cross-site browser requests.
    if (req.headers.origin) return res.status(403).json({ error: 'Browser origins are not allowed' });
    if (token !== undefined) {
      const candidate = req.path.startsWith('/mcp/') ? req.path.slice(5) : '';
      if (!/^[a-f0-9]{64}$/.test(candidate) || !timingSafeEqual(Buffer.from(candidate), Buffer.from(token))) {
        return res.status(404).end();
      }
    }
    next();
  });
  const route = token === undefined ? '/mcp' : '/mcp/:token';
  app.post(route, async (req, res) => {
    if (active.size >= 8) return res.status(429).set('Retry-After', '3').end();
    const server = serverFactory();
    const transport = new StreamableHTTPServerTransport({ sessionIdGenerator: undefined, enableJsonResponse: true });
    active.add(server);
    res.on('close', () => { active.delete(server); void server.close(); });
    try {
      await server.connect(transport);
      await transport.handleRequest(req, res, req.body);
    } catch {
      if (!res.headersSent) res.status(500).json({ jsonrpc: '2.0', error: { code: -32603, message: 'MCP request failed' }, id: null });
      else res.end();
    }
  });
  app.all(route, (_, res) => res.status(405).set('Allow', 'POST').end());
  app.use((_, res) => res.status(404).end());
  app.use((error, _, res, _next) => res.status(error.status === 413 ? 413 : 400).json({ error: 'Invalid request body' }));
  const listener = await new Promise((resolve, reject) => {
    const socket = app.listen(port, '127.0.0.1', () => resolve(socket));
    socket.on('error', reject);
  });
  return {
    url: `http://127.0.0.1:${listener.address().port}/mcp${token === undefined ? '' : '/'+token}`,
    async close() {
      await Promise.allSettled([...active].map(server => server.close()));
      await new Promise(resolve => { listener.close(resolve); listener.closeAllConnections(); });
    },
  };
}
