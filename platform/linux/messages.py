"""UI-independent message data and primary-owned delivery queue.

No toolkit, process, translation, or skin dependencies. Callers serialize access
on their event loop; the clock is injected so expiry needs no polling here.
"""
from dataclasses import dataclass, replace
from enum import Enum
import math
import re
import time
import uuid


class MessageState(str, Enum):
    RUNNING = 'running'
    SUCCESS = 'success'
    ERROR = 'error'
    CANCELLED = 'cancelled'
    INFO = 'info'


@dataclass(frozen=True)
class Action:
    id: str
    label: str
    enabled: bool = True
    align: str = 'start'
    tooltip: str = ''

    def __post_init__(self):
        if not isinstance(self.id, str) or not re.fullmatch(r'[a-z][a-z0-9_-]{0,39}', self.id):
            raise ValueError('Invalid action id')
        if not isinstance(self.label, str) or not 0 < len(self.label) <= 40:
            raise ValueError('Invalid action label')
        if type(self.enabled) is not bool or self.align not in ('start', 'end'):
            raise ValueError('Invalid action presentation')
        if not isinstance(self.tooltip, str) or len(self.tooltip) > 240:
            raise ValueError('Invalid action tooltip')


@dataclass(frozen=True)
class Reply:
    key: str
    enabled: bool = True
    clear_id: str = ''
    waiting: bool = False

    def __post_init__(self):
        if not isinstance(self.key, str) or not re.fullmatch(r'[a-f0-9]{32}', self.key) or type(self.enabled) is not bool:
            raise ValueError('Invalid reply channel')
        if not isinstance(self.clear_id, str) or len(self.clear_id) > 64:
            raise ValueError('Invalid reply acknowledgement')
        if type(self.waiting) is not bool:
            raise ValueError('Invalid reply waiting state')


@dataclass(frozen=True)
class Content:
    """Plain-text content and declarative actions; never a toolkit text buffer."""
    title: str
    text: str = ''
    status: str = ''
    show_status: bool = True
    actions: tuple[Action, ...] = ()
    kind: str = 'chat'
    reply: Reply | None = None
    speaker: str = ''

    def __post_init__(self):
        if self.kind not in ('chat', 'choice', 'notice'):
            raise ValueError('Invalid content kind')
        for name, limit in [('title', 160), ('text', 96000), ('status', 4000)]:
            value = getattr(self, name)
            if not isinstance(value, str) or len(value.encode('utf-16-le')) // 2 > limit:
                raise ValueError('Invalid content '+name)
        if type(self.show_status) is not bool or not isinstance(self.actions, tuple) or len(self.actions) > 6:
            raise ValueError('Invalid content actions')
        if any(not isinstance(a, Action) for a in self.actions) or len({a.id for a in self.actions}) != len(self.actions):
            raise ValueError('Duplicate or invalid content action')
        if self.reply is not None and not isinstance(self.reply, Reply):
            raise ValueError('Invalid reply input')
        if self.speaker not in ('', 'user', 'assistant'):
            raise ValueError('Invalid message speaker')


@dataclass(frozen=True)
class Message:
    id: str
    source: str
    owner: str
    slot: str
    content: Content
    state: MessageState
    priority: int
    created_at: float
    expires_at: float | None


class MessageManager:
    def __init__(self, owner, clock=time.monotonic, capacity=32):
        self.owner, self.clock, self.capacity = owner, clock, capacity
        self._messages = {}
        self._sources = {}
        self._listeners = []

    def register(self, source, action, removed=lambda *_: None, reply=None):
        if source in self._sources:
            raise ValueError('Message source already registered')
        self._sources[source] = (action, removed, reply)

    def subscribe(self, callback):
        self._listeners.append(callback)

    def remove_source_messages(self, source, reason='disabled'):
        for message in list(self.messages):
            if message.source == source:
                self.dismiss(message.id, reason)

    def _changed(self):
        for callback in tuple(self._listeners):
            callback()

    @property
    def messages(self):
        return tuple(self._messages.values())

    @property
    def active(self):
        # Dict insertion order makes equal priorities FIFO, including equal clocks.
        return max(self._messages.values(), key=lambda m: m.priority, default=None)

    @property
    def next_deadline(self):
        return min((m.expires_at for m in self._messages.values() if m.expires_at is not None), default=None)

    def get(self, message_id):
        return self._messages.get(message_id)

    def publish(self, source, content, state=MessageState.INFO, *, priority=10, ttl_ms=None, slot=None):
        if source not in self._sources or not isinstance(content, Content) or not isinstance(state, MessageState):
            raise ValueError('Unregistered source or invalid message')
        if type(priority) is not int or not 0 <= priority <= 100:
            raise ValueError('Priority must be between 0 and 100')
        if ttl_ms is not None and (type(ttl_ms) not in (int, float) or not math.isfinite(ttl_ms) or not 100 <= ttl_ms <= 3600000):
            raise ValueError('Message lifetime must be 100–3600000 ms')
        if slot is not None and (not isinstance(slot, str) or not 0 < len(slot) <= 80):
            raise ValueError('Invalid message slot')
        old = next((m for m in self.messages if slot is not None and (m.source, m.slot) == (source, slot)), None)
        if old is None and len(self._messages) >= self.capacity:
            raise ValueError('消息队列已满，请稍后重试。')
        if old:
            self._remove(old.id, 'replaced')
        now = self.clock()
        message_id = uuid.uuid4().hex
        self._messages[message_id] = Message(message_id, source, self.owner, slot or message_id, content,
                                             state, priority, now, None if ttl_ms is None else now + ttl_ms/1000)
        self._changed()
        return message_id

    def update(self, source, message_id, *, content, state):
        message = self.get(message_id)
        if message is None:
            return False  # Late result after replacement, expiry, or dismissal.
        if message.source != source:
            raise ValueError('A source cannot update another source\'s message')
        if not isinstance(content, Content) or not isinstance(state, MessageState):
            raise ValueError('Invalid message update')
        self._messages[message_id] = replace(message, content=content, state=state)
        self._changed()
        return True

    def dispatch(self, message_id, action_id):
        message = self.get(message_id)
        if message is None or not any(a.id == action_id and a.enabled for a in message.content.actions):
            return False
        self._sources[message.source][0](message_id, action_id)
        return True

    def submit_reply(self, message_id, text):
        message = self.get(message_id)
        if not message or not message.content.reply or not message.content.reply.enabled:
            raise ValueError('这条消息现在不能回复。')
        handler = self._sources[message.source][2]
        if not handler:
            raise ValueError('消息来源不支持回复。')
        return handler(message_id, text)

    def _remove(self, message_id, reason):
        message = self._messages.pop(message_id, None)
        if message:
            self._sources[message.source][1](message_id, reason)
        return message is not None

    def dismiss(self, message_id, reason='closed'):
        removed = self._remove(message_id, reason)
        if removed:
            self._changed()
        return removed

    def expire(self):
        now = self.clock()
        expired = [m.id for m in self.messages if m.expires_at is not None and m.expires_at <= now]
        for message_id in expired:
            self._remove(message_id, 'expired')
        if expired:
            self._changed()

    def clear(self, reason='owner_closed'):
        # Remove the whole batch before callbacks so no retired message resurfaces.
        old = self.messages
        self._messages.clear()
        for message in old:
            self._sources[message.source][1](message.id, reason)
        self._changed()
