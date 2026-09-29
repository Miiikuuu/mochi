"""Cross-process Manager preferences and saved, unapplied translation edits."""
from contextlib import contextmanager
from copy import deepcopy
import fcntl
import json
import os
from pathlib import Path
from manager_launcher import desktop_exec
from translation_controller import DEFAULT_MODEL, TranslationController
from theme_store import atomic_json


DEFAULT = {'appearance': 'system', 'close': 'background', 'translation_drafts': {}, 'revision': 0,
           'start_on_login': False, 'language': 'system', 'reduced_motion': False}


class PreferencesConflict(ValueError):
    def __init__(self, theme_id):
        self.theme_id = theme_id
        super().__init__(f'Translation settings for {theme_id} changed in another window.')


def translation_options(value):
    if not isinstance(value, dict) or set(value) != {'target', 'model', 'fast'}:
        raise ValueError('Invalid translation preferences.')
    TranslationController.validate_options(**value)
    if any(item is None for item in value.values()):
        raise ValueError('Incomplete translation preferences.')
    return deepcopy(value)


class ManagerPreferences:
    def __init__(self, store):
        self.store = store
        self.path = store.directory/'manager.json'
        self.lock_path = store.directory/'manager.json.lock'
        with self._lock():
            self.data = self._read_locked()

    @contextmanager
    def _lock(self):
        self.lock_path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        with self.lock_path.open('a+b') as stream:
            fcntl.flock(stream, fcntl.LOCK_EX)
            try:
                yield
            finally:
                fcntl.flock(stream, fcntl.LOCK_UN)

    def _read_locked(self):
        if not self.path.exists():
            return deepcopy(DEFAULT)
        value = json.loads(self.path.read_text())
        if (not isinstance(value, dict) or value.get('appearance') not in ('system', 'light', 'dark')
                or value.get('close') not in ('background', 'quit') or not isinstance(value.get('translation_drafts'), dict)
                or type(value.get('revision', 0)) is not int or value.get('revision', 0) < 0):
            raise ValueError('Invalid manager preferences. The original file has been preserved.')
        for key, draft in value['translation_drafts'].items():
            if not isinstance(key, str):
                raise ValueError('Invalid Theme reference.')
            translation_options(draft)
        value = {**deepcopy(DEFAULT), **value}
        self._validate(value)
        return value

    def _change(self, mutate):
        with self._lock():
            latest = self._read_locked()
            before = deepcopy(latest)
            result = mutate(latest)
            if latest != before:
                latest['revision'] += 1
                autostart_changed = latest['start_on_login'] != before['start_on_login']
                if autostart_changed:
                    self._autostart(latest['start_on_login'])
                try:
                    atomic_json(self.path, latest)
                except Exception:
                    if autostart_changed:
                        self._autostart(before['start_on_login'])
                    raise
            self.data = latest
            return result

    def refresh(self):
        with self._lock():
            self.data = self._read_locked()
            return deepcopy(self.data)

    def update(self, **changes):
        if set(changes)-(set(DEFAULT)-{'revision'}):
            raise ValueError('Unknown manager preference.')
        local_drafts = deepcopy(self.data['translation_drafts'])
        def mutate(latest):
            for key in ('appearance', 'close', 'start_on_login', 'language', 'reduced_motion'):
                if key in changes:
                    latest[key] = changes[key]
            if 'translation_drafts' in changes:
                requested = changes['translation_drafts']
                if not isinstance(requested, dict):
                    raise ValueError('Invalid translation drafts.')
                for theme_id in set(local_drafts) | set(requested):
                    old = local_drafts.get(theme_id)
                    new = requested.get(theme_id)
                    if old == new:
                        continue
                    if latest['translation_drafts'].get(theme_id) != old:
                        raise PreferencesConflict(theme_id)
                    if new is None:
                        latest['translation_drafts'].pop(theme_id, None)
                    else:
                        latest['translation_drafts'][theme_id] = translation_options(new)
            self._validate(latest)
        self._change(mutate)

    @staticmethod
    def _validate(value):
        if value['appearance'] not in ('system', 'light', 'dark') or value['close'] not in ('background', 'quit'):
            raise ValueError('Invalid manager preferences.')

        if value['language'] not in ('system', 'en', 'zh_CN') or any(type(value[key]) is not bool for key in ('start_on_login', 'reduced_motion')):
            raise ValueError('Invalid manager preferences.')

    def _autostart(self, enabled):
        directory = Path(os.environ.get('XDG_CONFIG_HOME', str(Path.home()/'.config')))/'autostart'
        directory.mkdir(parents=True, exist_ok=True)
        destination = directory/'mochi-manager.desktop'
        data = '[Desktop Entry]\nType=Application\nName=Mochi\nExec='+desktop_exec()+'\nTerminal=false\n'
        temporary = directory/('.mochi-manager-'+str(os.getpid())+'.tmp')
        try:
            if enabled:
                with temporary.open('w') as stream:
                    stream.write(data)
                    stream.flush()
                    os.fsync(stream.fileno())
                os.replace(temporary, destination)
            else:
                destination.unlink(missing_ok=True)
            fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY)
            try:
                os.fsync(fd)
            finally:
                os.close(fd)
        finally:
            temporary.unlink(missing_ok=True)

    def reset(self):
        def mutate(latest):
            revision = latest['revision']
            latest.clear()
            latest.update(deepcopy(DEFAULT), revision=revision)
        self._change(mutate)

    def translation(self, theme_id):
        self.store.get(theme_id)
        draft = self.data['translation_drafts'].get(theme_id)
        return deepcopy(draft) if draft is not None else self.applied_translation(theme_id)

    def applied_translation(self, theme_id):
        self.store.get(theme_id)
        for path in (self.store.directory/'themes'/theme_id/'translation.json', self.store.directory/'translation.json'):
            if path.exists():
                return translation_options(json.loads(path.read_text()))
        return {'target': 'zh-CN', 'model': DEFAULT_MODEL, 'fast': False}

    def edit_translation(self, theme_id, **changes):
        draft = translation_options({**self.translation(theme_id), **changes})
        old = deepcopy(self.data['translation_drafts'].get(theme_id))
        def mutate(latest):
            current = latest['translation_drafts'].get(theme_id)
            if current != old and current != draft:
                raise PreferencesConflict(theme_id)
            latest['translation_drafts'][theme_id] = draft
        self._change(mutate)

    def applied(self, theme_id, options):
        self.store.get(theme_id)
        def mutate(latest):
            if latest['translation_drafts'].get(theme_id) == options:
                del latest['translation_drafts'][theme_id]
        self._change(mutate)

    def force_translation(self, theme_id, options):
        self.store.get(theme_id)
        draft = translation_options(options)
        self._change(lambda latest: latest['translation_drafts'].__setitem__(theme_id, draft))
