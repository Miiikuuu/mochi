// Keep the tunnel's listener stable while replacing the MCP worker. CommonJS
// also permits a one-time migration of a running v0.1 gateway without rotating
// the user's freshly configured URL.
const { fork } = require('node:child_process');
const { request } = require('node:http');
const { resolve } = require('node:path');

async function installReload(server, configPath) {
  let current, replacing = false, closed = false;
  const workers = new Set();
  function retire(worker) {
    if (!worker || worker.stopping) return;
    worker.retired = true;
    if (worker.active) return;
    worker.stopping = true;
    if (worker.child.connected) worker.child.send('stop');
    const timer = setTimeout(() => worker.child.kill('SIGKILL'), 5000);
    timer.unref();
    worker.child.once('exit', () => clearTimeout(timer));
  }
  async function reload() {
    if (closed || replacing) return;
    replacing = true;
    try {
      const child = fork(resolve(__dirname, '../http-worker.mjs'), [configPath], { stdio: ['ignore', 'ignore', 'inherit', 'ipc'] });
      const worker = { child, active: 0, retired: false, stopping: false };
      workers.add(worker);
      child.once('exit', () => workers.delete(worker));
      worker.port = await new Promise((resolve, reject) => {
        const timer = setTimeout(() => { child.kill('SIGKILL'); reject(Error('MCP worker startup timed out')); }, 10000);
        const fail = () => { clearTimeout(timer); reject(Error('MCP worker could not start')); };
        child.once('error', fail); child.once('exit', fail);
        child.once('message', message => {
          if (!message.ready || !Number.isInteger(message.port) || message.port < 1 || message.port > 65535) return fail();
          clearTimeout(timer); child.off('exit', fail); child.off('error', fail); resolve(message.port);
        });
      });
      if (closed) { retire(worker); return; }
      const old = current;
      current = worker;
      retire(old);
    } finally { replacing = false; }
  }
  await reload(); // Do not touch the existing listener until replacement is ready.
  function proxy(req, res) {
    const worker = current;
    if (!worker || worker.child.exitCode !== null || worker.child.signalCode !== null) {
      req.resume(); res.writeHead(503); res.end(); return;
    }
    worker.active++;
    const upstream = request({ host: '127.0.0.1', port: worker.port, method: req.method, path: req.url,
      headers: req.headers, timeout: 35000 }, response => {
      res.writeHead(response.statusCode, response.headers); response.pipe(res);
      response.on('error', () => res.destroy());
    });
    // Preserve Host and Origin so the protected worker performs its normal checks.
    upstream.on('timeout', () => upstream.destroy(Error('MCP worker timeout')));
    upstream.on('error', () => { if (!res.headersSent) res.writeHead(502); res.end(); });
    req.on('error', () => upstream.destroy());
    res.once('close', () => { upstream.destroy(); worker.active--; if (worker.retired) retire(worker); });
    req.pipe(upstream);
  }
  server.removeAllListeners('request');
  server.on('request', proxy);
  const onReload = () => { void reload().catch(() => console.error('MCP reload failed; previous worker retained')); };
  process.on('SIGHUP', onReload);
  function close() {
    closed = true;
    process.off('SIGHUP', onReload);
    for (const worker of workers) retire(worker);
  }
  server.once('close', close);
  return { reload, close };
}
module.exports = { installReload };
