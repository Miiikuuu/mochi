"""Cross-process resource dependencies and recoverable uninstall transactions."""
from contextlib import contextmanager
from copy import deepcopy
import fcntl
import json
from pathlib import Path
import threading
import uuid
import shutil

_LOCKS = {}
_GUARD = threading.Lock()
_LOCAL = threading.local()


@contextmanager
def file_lock(path):
    """Reentrant within a thread, serialized across threads and processes."""
    key = str(Path(path).resolve())
    with _GUARD:
        mutex = _LOCKS.setdefault(key, threading.RLock())
    with mutex:
        held = getattr(_LOCAL, 'held', None)
        if held is None:
            held = _LOCAL.held = {}
        if key in held:
            yield False
            return
        with Path(path).open('a+b') as stream:
            fcntl.flock(stream, fcntl.LOCK_EX)
            held[key] = stream
            try:
                yield True
            finally:
                del held[key]
                fcntl.flock(stream, fcntl.LOCK_UN)


class ResourceInUse(ValueError):
    def __init__(self, kind, key, themes):
        self.kind, self.key, self.themes = kind, key, themes
        super().__init__('Resource is used by Themes: ' + ', '.join(item['name'] for item in themes))


class ResourceDependencyIndex:
    def __init__(self, themes, runtime=()):
        self.runtime = tuple(runtime)
        self.entries = {kind: {} for kind in ('characters', 'bubbles', 'plugins')}
        for theme in themes:
            for kind, keys in (('characters', theme['characters']), ('bubbles', [theme['bubble']]), ('plugins', theme['plugins'])):
                for key in keys:
                    self.entries[kind].setdefault(key, []).append({'id': theme['id'], 'name': theme['name']})

    def runtime_users(self, kind, key):
        return tuple(item for item in self.runtime if item.uses(kind, key))

    def users(self, kind, key):
        return deepcopy(self.entries[kind].get(key, []))


def uninstall(resources, kind, key, destination, *, replacement=None, disable=False, expected_revision=None, repair_only=False):
    """All dependent Themes and package removal share a single rollback journal.

    Readers acquire the same Theme/resource locks. The journal is durable before
    mutation; disappearance is the commit point. Startup/read recovery rolls back
    an interrupted transaction, including restoring package bytes.
    """
    from resource_catalog import atomic_write, read_bounded
    from theme_store import ThemeConflict
    with resources.leases.mutation():
        if not repair_only:
            resources.leases.assert_unused(kind, key)
        return _uninstall_locked(resources, kind, key, destination, replacement=replacement, disable=disable, expected_revision=expected_revision, repair_only=repair_only)


def _uninstall_locked(resources, kind, key, destination, *, replacement, disable, expected_revision, repair_only):
    from resource_catalog import read_bounded
    from resource_transactions import commit
    from theme_store import ThemeConflict
    with resources.locked():
        catalog = resources.read()
        entry = catalog[kind].get(key)
        if not entry or entry.get('origin') == 'bundled':
            raise ValueError('Only installed local resources can be removed.')
        path = resources.directory/'themes.json'
        themes = json.loads(read_bounded(path, 4*1024*1024)) if path.exists() else None
        revision = (themes or {}).get('revision', 0)
        if expected_revision is not None and expected_revision != revision:
            raise ThemeConflict(expected_revision, revision)
        users = ResourceDependencyIndex((themes or {}).get('themes', [])).users(kind, key)
        if users and not (disable and kind == 'plugins') and replacement is None:
            raise ResourceInUse(kind, key, users)
        if users and expected_revision is None:
            raise ValueError('Dependency repair requires an expected Theme revision.')
        if replacement is not None:
            if replacement == key or replacement not in catalog[kind] or catalog[kind][replacement].get('lifecycle', 'active') != 'active':
                raise ValueError('Choose an installed replacement.')
            if kind == 'plugins':
                raise ValueError('Disable plugins before removal; replacement plugins require separate approval.')
        updated = deepcopy(themes)
        for theme in (updated or {}).get('themes', []):
            if kind == 'plugins' and key in theme['plugins']:
                theme['plugins'].remove(key)
                theme.get('plugin_settings', {}).pop(key, None)
            elif kind == 'bubbles' and theme['bubble'] == key:
                theme['bubble'] = replacement
            elif kind == 'characters' and key in theme['characters']:
                theme['characters'] = list(dict.fromkeys(replacement if value == key else value for value in theme['characters']))
                if theme['primary'] == key:
                    theme['primary'] = replacement
                theme.get('actions', {}).pop(key, None)
        if users:
            updated['revision'] = revision + 1
        if kind == 'characters' and updated:
            updated.get('characters', {}).pop(key, None)
        if repair_only:
            catalog[kind][key].update(lifecycle='pending_removal', removal_requested=True)
            catalog[kind][key].pop('pending_removal', None)
        else:
            del catalog[kind][key]
        commit(resources, kind, key, destination, catalog, themes=updated,
               operation='repair' if repair_only else 'uninstall')
        return {'kind': kind, 'key': key, 'repaired': users, 'pending_removal': repair_only,
                'theme_revision': (updated or {}).get('revision', revision)}
