"""The public Sessiond read model. Binding credentials cannot be represented."""
from dataclasses import dataclass, asdict


@dataclass(frozen=True)
class SanitizedSessionState:
    connected: bool = False
    conversation: str = ''
    heartbeat_age: int | None = None
    heartbeat_at: str | None = None
    phase: str = 'idle'
    response_received: bool = False

    def as_dict(self):
        value = asdict(self)
        value['conversation'] = self.conversation[:8]
        return value
