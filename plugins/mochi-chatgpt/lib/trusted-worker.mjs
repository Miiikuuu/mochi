// Keep the existing gateway IPC/reload protocol while the daemon starts the
// actual authenticated HTTP worker with an inherited, process-bound authority.
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { pythonHelper } from './layout.mjs';
export async function trustedWorker(configPath) {
  if (process.env.MOCHI_SESSION_FD) return;
  const child = spawn('/usr/bin/python3', [pythonHelper('session_launch.py'), 'card-worker', configPath],
    { stdio: ['ignore', 'pipe', 'inherit'] });
  createInterface({ input: child.stdout }).on('line', line => {
    try {
      const value = JSON.parse(line);
      if (value.ready && Number.isInteger(value.port) && value.port > 0 && value.port < 65536) process.send?.(value);
    } catch { /* stdout is bounded readiness metadata, never tool results */ }
  });
  const stop = () => child.kill('SIGTERM');
  process.on('message', message => { if (message === 'stop') stop(); });
  process.on('disconnect', stop);
  process.on('SIGTERM', stop); process.on('SIGINT', stop);
  const code = await new Promise((resolve, reject) => {child.once('exit', resolve); child.once('error', reject);});
  if (process.connected) process.disconnect();
  process.exit(code ?? 1);
}
