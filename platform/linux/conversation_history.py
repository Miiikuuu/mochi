"""Bounded, in-memory plain text history, independent of skins and GTK."""
from dataclasses import dataclass


@dataclass(frozen=True)
class Turn:
    id: str
    speaker: str
    title: str
    text: str

    @property
    def bytes(self):
        return len((self.title+self.text).encode('utf-8'))


class ConversationHistory:
    limit = 200
    byte_limit = 512*1024

    def __init__(self):
        self.channel = None
        self.turns = []
        self.bytes = 0
        self.version = 0

    def clear(self):
        self.channel = None
        self.turns.clear()
        self.bytes = 0
        self.version += 1

    def add(self, message):
        reply = message.content.reply
        if not reply:return False
        channel = (message.source, reply.key)
        if channel != self.channel:
            self.turns.clear()
            self.bytes = 0
            self.channel = channel
            self.version += 1
        content = message.content
        turn = Turn(message.id, content.speaker or 'assistant', content.title, content.text)
        index = next((i for i,t in enumerate(self.turns) if t.id == turn.id), None)
        if index is not None:
            old = self.turns[index]
            if old == turn:return False
            self.bytes -= old.bytes
            self.turns[index] = turn
        else:self.turns.append(turn)
        self.bytes += turn.bytes
        while len(self.turns) > self.limit or self.bytes > self.byte_limit:
            self.bytes -= self.turns.pop(0).bytes
        self.version += 1
        return True
