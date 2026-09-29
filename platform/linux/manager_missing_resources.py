"""Missing resource presentation; never rewrites a Theme or substitutes assets."""
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class MissingResource:
    kind: str
    technical_id: str
    label: str
    original_path: str | None = None
    adoptable: bool = False


def missing_resources(theme, characters, bubbles, plugins, check_files=True):
    from mochi_paths import SPINE_SUPPORTED
    result = []
    for kind, keys, catalog, manifest, label in (
        ('characters', theme['characters'], characters, 'manifest.toml', 'Missing Character'),
        ('bubbles', [theme['bubble']], bubbles, 'theme.json', 'Missing Bubble'),
        ('plugins', theme['plugins'], plugins, 'mochi-plugin.json', 'Missing Plugin'),
    ):
        for key in keys:
            item = catalog.get(key, {})
            path = item.get('root') or item.get('path')
            exists = bool(path and (Path(path)/manifest).is_file()) if check_files else bool(item)
            if kind == 'characters' and item.get('requires_spine') and not SPINE_SUPPORTED:
                result.append(MissingResource(kind, key, 'Requires a Spine-enabled build', str(path) if path else None))
            elif not exists or item.get('lifecycle') in ('missing', 'pending_removal'):
                result.append(MissingResource(kind, key, 'Pending removal' if item.get('lifecycle') == 'pending_removal' else label, str(path) if path else None,
                                              exists and item.get('origin') == 'bundled'))
    return tuple(result)
