"""User-facing Theme feature metadata without exposing plugin infrastructure."""
from dataclasses import dataclass


FEATURE_NAMES = {
    'translation': 'Translation',
    'chatgpt': 'ChatGPT conversation',
    'codex-notify': 'Codex notifications',
}


@dataclass(frozen=True)
class FeatureReadModel:
    key: str
    display_name: str
    enabled: bool
    version: str
    capabilities: tuple[str, ...]
    origin: str
    review: str
    isolation: str
    runtime: str
    package_sha256: str
    available: bool
    third_party: bool
    description: str
    lifecycle: str = "active"
    session_isolation: str = "Session isolation unavailable"


def feature_models(catalog, enabled):
    result = []
    for key in dict.fromkeys([*enabled, *catalog]):
        spec = catalog.get(key)
        result.append(FeatureReadModel(
            key=key,
            display_name=FEATURE_NAMES.get(key, spec.get('name', key) if spec else key),
            enabled=key in enabled,
            version=spec.get('version', '') if spec else '',
            capabilities=tuple(spec.get('capabilities', ())) if spec else (),
            origin=spec.get('origin', 'local') if spec else 'local',
            review=spec.get('review', 'unreviewed') if spec else 'unreviewed',
            isolation=spec.get('isolation', 'unsandboxed') if spec else 'unsandboxed',
            runtime=spec.get('runtime', '') if spec else '',
            package_sha256=spec.get('package_sha256', '') if spec else '',
            available=spec is not None,
            third_party=key not in FEATURE_NAMES,
            description=spec.get('description', '') if spec else 'Plugin unavailable.',
            session_isolation=spec.get('session_isolation', 'Session isolation unavailable') if spec else 'Session isolation unavailable',
            lifecycle=spec.get('lifecycle', 'active') if spec else 'missing',
        ))
    return tuple(sorted(result, key=lambda model: (not model.enabled, model.display_name.casefold())))
