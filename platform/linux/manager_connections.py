"""Toolkit-free, allow-listed connection projection of the shared Runtime query."""
from dataclasses import dataclass, field
import json
import re


@dataclass
class ModelCatalogState:
    models: dict = field(default_factory=dict)
    loading: bool = False
    last_synced: str | None = None
    error: str | None = None


@dataclass(frozen=True)
class ConnectionsReadModel:
    connected: bool = False
    conversation: str = ''
    heartbeat_age: int | None = None
    heartbeat_at: str = ''
    reply: str = 'idle'
    confirmation_pending: bool = False
    translation_available: bool = False
    current_model: str = ''
    fast_tier: str | None = None
    notifications: str = 'Stopped'
    last_event: str = ''
    health: str = 'Unavailable'
    service_state: str = 'disconnected'
    generation: str = ''
    build: str = ''
    protocol: int | None = None
    service_error: str = ''

    def diagnostics(self):
        # Never copy the source snapshot: it can contain paths, keys and tokens.
        return json.dumps(self.__dict__, ensure_ascii=False, indent=2)


def project_connections(runtime, catalog, session=None):
    snapshot = runtime.snapshot if not runtime.disconnected else None
    snapshot = snapshot if isinstance(snapshot, dict) else {}
    plugins = snapshot.get('plugin_status', {})
    plugins = plugins if isinstance(plugins, dict) else {}
    def plugin(key):
        value = plugins.get(key, {})
        return value if isinstance(value, dict) else {}
    chat = plugin('chatgpt')
    mailbox = session if session is not None else chat.get('connection', {})
    mailbox = mailbox if isinstance(mailbox, dict) else {}
    raw_key = mailbox.get('conversation', '')
    key = raw_key[:8] if isinstance(raw_key, str) and re.fullmatch(r'[a-fA-F0-9]{1,64}', raw_key) else ''
    age = mailbox.get('heartbeat_age')
    age = int(age) if type(age) in (int, float) and 0 <= age <= 86400000 else None
    stamp = mailbox.get('heartbeat_at', '')
    stamp = stamp if isinstance(stamp, str) and re.fullmatch(r'[0-9TZ:.+ -]{1,40}', stamp) else ''
    phase = mailbox.get('phase', 'idle')
    if phase not in ('idle', 'queued', 'delivering', 'sent', 'uncertain', 'failed'):
        phase = 'uncertain'
    translation = snapshot.get('translation', {})
    translation = translation if isinstance(translation, dict) else {}
    model = translation.get('model', '')
    model = model if isinstance(model, str) and re.fullmatch(r'[a-zA-Z0-9._:-]{1,128}', model) else ''
    notify = plugin('codex-notify')
    enabled = bool(notify.get('enabled'))
    event = notify.get('last_event', '')
    event = event if isinstance(event, str) and re.fullmatch(r'[0-9TZ:.+ -]{1,40}', event) else ''
    return ConnectionsReadModel(
        connected=bool((session is not None or chat.get('enabled')) and mailbox.get('connected')),
        service_state='active' if mailbox.get('service_state') == 'active' else 'disconnected',
        generation=str(mailbox.get('generation', ''))[:32] if re.fullmatch(r'[a-f0-9]{32}', str(mailbox.get('generation', ''))) else '',
        build=str(mailbox.get('build', ''))[:64] if re.fullmatch(r'[a-zA-Z0-9._-]{1,64}', str(mailbox.get('build', ''))) else '',
        protocol=mailbox.get('protocol') if type(mailbox.get('protocol')) is int else None,
        service_error='session.version_mismatch' if 'session.version_mismatch' in str(mailbox.get('service_error', '')) else '',
        conversation=key, heartbeat_age=age, heartbeat_at=stamp, reply=phase,
        confirmation_pending=bool(phase in ('delivering', 'sent') and not mailbox.get('response_received', False)),
        translation_available=bool(plugin('translation').get('enabled') and not plugin('translation').get('failed')),
        current_model=model, fast_tier=catalog.models.get(model, {}).get('fast_tier'),
        notifications='Listening' if enabled and not notify.get('failed') else 'Stopped',
        last_event=event, health=('Error' if notify.get('failed') else 'Healthy') if enabled else 'Unavailable')
