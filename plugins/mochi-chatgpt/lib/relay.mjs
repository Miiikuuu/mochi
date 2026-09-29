// The relay never resends a claimed reply: ambiguous delivery needs user review.
export class ReplyRelay {
  constructor({ call, send, token, client, changed = () => {}, interval = 2000 }) {
    Object.assign(this, { call, send, token, client, changed, interval });
    this.running = false;
    this.timer = null;
    this.flight = null;
  }
  async bridge(operation, fields = {}) {
    const value = await this.call({ token: this.token, client: this.client, operation, ...fields });
    if (value.isError) throw new Error(value.content?.[0]?.text || '连接失败');
    return value;
  }
  async start() {
    if (this.running) return;
    await this.bridge('attach');
    this.running = true;
    this.changed('已连接，可以在桌宠气泡回复。');
    this.schedule();
  }
  schedule() {
    if (this.running) this.timer = setTimeout(() => {
      this.flight = this.tick().finally(() => { this.flight = null; this.schedule(); });
    }, this.interval);
  }
  async tick() {
    try {
      const value = await this.bridge('poll');
      const reply = value._meta?.reply;
      if (!reply) { this.changed('已连接，可以在桌宠气泡回复。'); return; }
      this.changed('正在将回复交给当前对话…');
      let state;
      try {
        const response = await this.send(reply.text);
        state = response?.isError ? 'failed' : 'sent';
      } catch { state = 'uncertain'; }
      // ACK retries are safe; sending the user message again is not.
      let acknowledged = false;
      for (let attempt = 0; attempt < 3; attempt++) {
        try { await this.bridge('ack', { reply_id: reply.id, state }); acknowledged = true; break; }
        catch { /* Keep the original delivery state for the next ACK attempt. */ }
      }
      this.changed(!acknowledged || state === 'uncertain' ? '发送结果未确认，请查看当前对话。'
        : state === 'sent' ? '已交给 ChatGPT；若出现 Send follow-up?，请确认发送。' : 'ChatGPT 未接收回复，文字已保留在桌宠。');
    } catch {
      this.changed('暂未连接，请检查 Mochi 是否运行；未确认的回复不会自动重发。');
    }
  }
  async stop() {
    this.running = false;
    clearTimeout(this.timer);
    await this.flight;
    await this.bridge('detach').catch(() => {});
    this.changed('已断开。');
  }
}
