import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createInterface } from 'node:readline';

const connections = new Set();
const disabled = ['apps', 'plugins', 'remote_plugin', 'hooks', 'shell_tool',
  'unified_exec', 'shell_snapshot', 'browser_use', 'browser_use_external',
  'computer_use', 'image_generation', 'view_image', 'multi_agent',
  'multi_agent_v2', 'code_mode', 'code_mode_host', 'goals', 'memories',
  'skill_search', 'tool_suggest', 'sleep_tool'];

class Connection {
  constructor(key, command, prefix, idleTimeoutMs, maxRequests) {
    Object.assign(this, { key, command, prefix, idleTimeoutMs, maxRequests });
    this.pending = new Map();
    this.serial = this.uses = 0;
    this.busy = true;
  }

  async open() {
    this.opening = mkdtemp(join(tmpdir(), 'mochi-translate-'));
    this.cwd = await this.opening;
    if (this.failure) throw this.failure;
    const args = [...this.prefix, 'app-server', '--listen', 'stdio://',
      ...disabled.flatMap(key => ['-c', `features.${key}=false`]),
      '-c', 'web_search="disabled"', '-c', 'agents.enabled=false',
      '-c', 'features.fast_mode=true', '-c', 'project_doc_max_bytes=0',
      '-c', 'history.persistence="none"'];
    this.child = spawn(this.command, args, { cwd: this.cwd, stdio: ['pipe', 'pipe', 'pipe'], detached: true });
    this.exited = new Promise(resolve => this.child.once('exit', resolve));
    this.child.on('error', () => this.fail(new Error('无法启动 Codex，请先安装并登录 Codex。')));
    this.child.stdin.on('error', () => this.fail(new Error('Codex 连接已关闭。')));
    this.child.stderr.on('data', () => {});
    this.child.on('exit', () => {
      this.fail(new Error('Codex 已退出，请检查登录状态后重试。'));
      void this.close();
    });
    this.lines = createInterface({ input: this.child.stdout });
    this.lines.on('line', line => {
      if (this.failure) return;
      try {
        const message = JSON.parse(line);
        if (Object.hasOwn(message, 'id') && !message.method) {
          const request = this.pending.get(message.id);
          if (!request) return;
          this.pending.delete(message.id);
          if (message.error) request.reject(new Error(message.error.message || 'Codex 请求失败。'));
          else request.resolve(message.result);
        } else if (Object.hasOwn(message, 'id') && message.method) {
          this.write({ id: message.id, error: { code: -32601, message: 'Translation does not allow tool requests.' } });
          this.fail(new Error('翻译引擎请求了额外操作，已停止本次翻译。'));
        } else this.onEvent?.(message);
      } catch {
        this.fail(new Error('Codex 返回了无效数据。'));
      }
    });
  }

  write(message) {
    if (this.failure) throw this.failure;
    this.child.stdin.write(JSON.stringify(message) + '\n');
  }

  request(method, params) {
    if (this.failure) return Promise.reject(this.failure);
    const id = ++this.serial;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      try { this.write({ id, method, params }); }
      catch (error) { this.pending.delete(id); reject(error); }
    });
  }

  fail(error) {
    if (this.failure) return;
    this.failure = error;
    for (const item of this.pending.values()) item.reject(error);
    this.pending.clear();
    this.onFailure?.(error);
  }

  close() {
    if (this.closing) return this.closing;
    clearTimeout(this.idleTimer);
    connections.delete(this);
    this.fail(new Error('Codex 连接已关闭。'));
    this.closing = (async () => {
      this.lines?.close();
      const child = this.child;
      if (child?.pid && child.exitCode === null && child.signalCode === null) {
        try { process.kill(-child.pid, 'SIGTERM'); } catch {}
        let timer;
        await Promise.race([this.exited, new Promise(resolve => { timer = setTimeout(resolve, 1000); })]);
        clearTimeout(timer);
        if (child.exitCode === null && child.signalCode === null) {
          try { process.kill(-child.pid, 'SIGKILL'); } catch {}
          await this.exited;
        }
      }
      const cwd = this.cwd || await this.opening?.catch(() => null);
      if (cwd) await rm(cwd, { recursive: true, force: true });
    })();
    return this.closing;
  }
}

export function acquireConnection(options = {}) {
  const command = options.command || process.env.MOCHI_CODEX_BIN || 'codex';
  const prefix = options.commandPrefix || [];
  const idleTimeoutMs = options.idleTimeoutMs ?? 120000;
  const maxRequests = options.maxRequests ?? 32;
  if (!Number.isFinite(idleTimeoutMs) || idleTimeoutMs < 0 || !Number.isInteger(maxRequests) || maxRequests < 1) {
    throw new Error('无效的翻译进程回收设置。');
  }
  const key = JSON.stringify([command, prefix, idleTimeoutMs, maxRequests]);
  for (const connection of connections) {
    if (!connection.busy && !connection.failure && connection.key === key) {
      clearTimeout(connection.idleTimer);
      connection.busy = true;
      return { connection, reused: true };
    }
  }
  const connection = new Connection(key, command, prefix, idleTimeoutMs, maxRequests);
  connections.add(connection);
  return { connection, reused: false };
}

export async function releaseConnection(connection, healthy) {
  connection.onEvent = connection.onFailure = null;
  // Unsubscribed ephemeral threads may remain loaded in Codex for a grace
  // period. Bound a continuously used process too, not only its idle lifetime.
  if (!healthy || connection.failure || ++connection.uses >= connection.maxRequests || !connection.idleTimeoutMs) {
    await connection.close();
    return;
  }
  connection.busy = false;
  connection.idleTimer = setTimeout(() => { void connection.close(); }, connection.idleTimeoutMs);
  connection.idleTimer.unref();
  await Promise.all([...connections].filter(other => other !== connection && !other.busy)
    .map(other => other.close()));
}

export async function closeTranslationEngines() {
  await Promise.all([...connections].map(connection => connection.close()));
}
