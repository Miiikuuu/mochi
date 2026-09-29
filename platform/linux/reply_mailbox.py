"""One explicitly bound conversation and one outgoing reply, independent of UI."""
import secrets
import time
import uuid
from session_state import SanitizedSessionState


class ReplyMailbox:
    def __init__(self, clock=lambda: time.clock_gettime(time.CLOCK_BOOTTIME)):
        self.clock = clock
        self.sessions = {}
        self.active = None
        self.client = None
        self.seen = 0
        self.pending = None
        self.clear_id = ''
        self.response_received = False

    def create(self):
        now = self.clock()
        self.sessions = {token: item for token, item in self.sessions.items()
                         if item['key'] == self.active or now-item['created'] < 600}
        if len(self.sessions) >= 8:
            raise ValueError('连接卡片过多，请稍后重试。')
        key, token = uuid.uuid4().hex, secrets.token_hex(32)
        self.sessions[token] = {'key': key, 'created': now}
        return {'conversation_key': key, 'reply_token': token}

    def validate(self, token):
        item = self.sessions.get(token) if isinstance(token, str) else None
        if not item or item['key'] != self.active and self.clock()-item['created'] >= 600:
            raise ValueError('连接卡片已失效，请在 ChatGPT 重新连接 Mochi。')
        return item['key']

    def attach(self, token, client):
        key = self.validate(token)
        self.snapshot()
        if not isinstance(client, str) or not 16 <= len(client) <= 64 or not client.isalnum():
            raise ValueError('连接标识无效。')
        if (key, client) != (self.active, self.client):
            if self.pending and self.pending['state'] in ('queued', 'delivering'):
                raise ValueError('上一条回复尚未确认，请先处理后再切换对话。')
            self.pending = None
            self.clear_id = ''
        self.active, self.client, self.seen = key, client, self.clock()
        return {'connected': True, 'conversation_key': key}

    def authorize(self, token, client):
        if self.validate(token) != self.active or client != self.client:
            raise ValueError('此卡片没有连接桌宠，请点击“连接此对话”。')

    def snapshot(self):
        now = self.clock()
        if self.pending and self.pending['state'] == 'delivering' and now-self.pending['claimed'] >= 30:
            self.pending['state'] = 'uncertain'
        connected = bool(self.active and now-self.seen < 20)
        # An abandoned, unclaimed submission must not lock out a reloaded card
        # forever or unexpectedly send when that page comes back much later.
        if not connected and self.pending and self.pending['state'] == 'queued':
            self.pending['state'] = 'failed'
        phase = self.pending['state'] if self.pending else None
        status = {'queued': '等待 ChatGPT 接收…', 'delivering': '正在交给 ChatGPT…',
                  'sent': '已交给 ChatGPT；若出现 Send follow-up?，请确认发送。',
                  'failed': '未发送，请确认连接后重试。', 'uncertain': '未确认是否送达，请先查看 ChatGPT 再决定是否重发。'}.get(phase, '')
        if not connected:
            status = '未连接，请保持 ChatGPT 的连接卡片打开。'
        elif phase == 'sent' and self.response_received:
            status = ''
        return {'key': self.active, 'connected': connected, 'enabled': connected and phase not in ('queued', 'delivering'),
                'status': status, 'clear_id': self.clear_id,
                'waiting': connected and phase in ('queued', 'delivering', 'sent') and not self.response_received}

    def connection_status(self):
        import math
        from datetime import datetime, timezone
        state = self.snapshot()
        age = self.clock()-self.seen
        return SanitizedSessionState(
            connected=state['connected'], conversation=self.active or '',
            heartbeat_age=int(max(0, age)) if self.active and math.isfinite(age) else None,
            heartbeat_at=datetime.fromtimestamp(time.time()-max(0, age), timezone.utc).isoformat(timespec='seconds') if self.active and math.isfinite(age) else None,
            phase=self.pending['state'] if self.pending else 'idle',
            response_received=self.response_received).as_dict()


    def submit(self, key, text):
        if key != self.active or not self.snapshot()['enabled']:
            raise ValueError('当前对话未连接或上一条仍在发送，请稍后再试。')
        if not isinstance(text, str) or not text.strip() or len(text.encode('utf-16-le'))//2 > 4000:
            raise ValueError('回复需要 1–4000 个字符。')
        self.pending = {'id': uuid.uuid4().hex, 'text': text, 'state': 'queued'}
        self.response_received = False
        return self.pending['id']

    def poll(self, token, client):
        self.authorize(token, client)
        self.snapshot()
        self.seen = self.clock()
        reply = None
        if self.pending and self.pending['state'] == 'queued':
            self.pending.update(state='delivering', claimed=self.clock())
            reply = {key: self.pending[key] for key in ('id', 'text')}
        return {'connected': True, 'reply': reply}

    def acknowledge(self, token, client, identity, state):
        self.authorize(token, client)
        if state not in ('sent', 'failed', 'uncertain') or not self.pending or self.pending['id'] != identity:
            raise ValueError('回复确认已失效。')
        if self.pending['state'] not in ('delivering', 'uncertain', state):
            raise ValueError('回复确认顺序无效。')
        self.pending['state'] = state
        if state == 'sent':
            self.clear_id = identity
            self.pending['text'] = ''
        self.seen = self.clock()
        return {'accepted': True}

    def detach(self, token, client):
        self.authorize(token, client)
        self.seen = float('-inf')
        return {'connected': False}

    def reset(self):
        self.sessions.clear()
        self.active = self.client = self.pending = None
        self.seen = 0
        self.clear_id = ''
        self.response_received = False
