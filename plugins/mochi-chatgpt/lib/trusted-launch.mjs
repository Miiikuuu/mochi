// Only the daemon-launched bridge receives an inherited capability FD.
import { spawn } from 'node:child_process';
import { pythonHelper } from './layout.mjs';
export async function trustedLaunch(role) {
  if (process.env.MOCHI_SESSION_FD) return;
  const launcher = pythonHelper('session_launch.py');
  const child = spawn('/usr/bin/python3', [launcher, role], {stdio: 'inherit'});
  for (const name of ['SIGINT', 'SIGTERM']) process.on(name, () => child.kill(name));
  const code = await new Promise((resolve, reject) => {child.on('exit', resolve); child.on('error', reject);});
  process.exit(code ?? 1);
}
