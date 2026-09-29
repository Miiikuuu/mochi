"""Toolkit-free Manager application operations and read models."""
from collections import OrderedDict
from copy import deepcopy
from io import BytesIO
import json
import os
from pathlib import Path

from character_actions import settings
from manager_preferences import ManagerPreferences, PreferencesConflict, translation_options
from manager_character_read_model import CharacterReadModel, display_name
from manager_undo import UndoExpired
from manager_feature_read_model import feature_models
from manager_resources import ResourceLoader, fallback_bubble
from manager_state import AppError, AppState, ImportResult, Route, ThemeDraft, error_from_exception
from native_jobs import NativeJobs
from plugin_catalog import settings_for, PluginApprovalRequired
from theme_store import ROOT, ThemeConflict, ThemeStore, atomic_json
from theme_runtime import ThemeRuntime
from resource_catalog import read_bounded
from translation_controller import valid_model


class ManagerController:
    def __init__(self, store=None, runtime=None):
        self.store = store or ThemeStore()
        self.runtime = runtime or ThemeRuntime(self.store)
        if runtime is not None and hasattr(runtime, 'status'):
            self.store.resources.leases.probe = runtime.status
        self.preferences = ManagerPreferences(self.store)
        self.catalog_transport = NativeJobs(os.environ.get('MOCHI_TRANSLATION_LAUNCHER', str(ROOT/'plugins/mochi-translate/.local/native-host')))
        self.preview_cache = OrderedDict()
        self.preview_cache_bytes = 0
        self.resource_loader = ResourceLoader(self.store.directory, self.store.bubbles.directory, self.store.bubbles.preferences, self.store.bubbles.bundled_directory)
        self.resource_loader.leases = self.store.resources.leases
        self.resource_snapshot = None
        self.resource_thumbnails = {}
        self.bubble_previews = {}
        self.resource_revision = self.store.resources.snapshot['revision']
        self.state = AppState(Route(), ThemeDraft(self.store.get(), store_revision=self.store.revision))
        self.state.settings = deepcopy(self.preferences.data)
        self.translation_editing = None
        self.translation_conflict = None
        self.refresh_resources()

    def sync_theme(self):
        previous = self.store.revision
        current = self.store.reload()
        if current == previous:
            return 'unchanged'
        self.refresh_resources()
        if self.state.draft.needs_save:
            return 'pending'
        key = self.state.draft.value['id']
        try:
            value = self.store.get(key)
        except StopIteration:
            value = self.store.get()
        self.state.draft = ThemeDraft(value, store_revision=current)
        return 'reloaded'

    def sync_preferences(self):
        previous = deepcopy(self.preferences.data)
        old_options = self.translation(self.translation_editing) if self.translation_editing else None
        current = self.preferences.refresh()
        self.state.settings = deepcopy(current)
        theme_id = self.translation_editing
        if theme_id and previous['translation_drafts'].get(theme_id) != current['translation_drafts'].get(theme_id):
            latest = self.preferences.translation(theme_id)
            if self.translation_conflict and self.translation_conflict['theme_id'] == theme_id:
                self.translation_conflict['latest'] = latest
            else:
                self.translation_conflict = {'theme_id': theme_id, 'mine': old_options, 'latest': latest}
        return current['revision'] != previous['revision']

    def set_translation_editing(self, theme_id):
        if theme_id == self.translation_editing:
            return
        self.translation_editing = theme_id
        self.translation_conflict = None

    def resolve_translation_conflict(self, action):
        conflict = self.translation_conflict
        if not conflict:
            return None
        if action == 'compare':
            return deepcopy(conflict)
        if action == 'latest':
            self.preferences.refresh()
        elif action == 'mine':
            self.preferences.force_translation(conflict['theme_id'], conflict['mine'])
        else:
            raise ValueError('Unknown translation conflict action.')
        self.state.settings = deepcopy(self.preferences.data)
        self.translation_conflict = None
        return None

    def update_settings(self, **changes):
        try:
            self.preferences.update(**changes)
            self.state.settings = deepcopy(self.preferences.data)
        except Exception as error:
            raise error_from_exception(error, 'settings') from error

    def reset_settings(self):
        self.preferences.reset()
        self.state.settings = deepcopy(self.preferences.data)
        self.translation_conflict = None

    def settings_read_model(self):
        return {**deepcopy(self.state.settings), 'log_location': str(self.store.directory/'runs')}

    def disconnect_chatgpt(self):
        from session_client import request, status
        try:
            request('disconnect')
            return status()
        except (OSError, ValueError) as error:
            raise AppError('session.disconnected', 'The ChatGPT session disconnected. Reconnect the conversation card.') from error

    def start_connection_events(self, callback, dispatch):
        from session_client import Subscription
        return Subscription(callback, dispatch)

    def manager_status(self):
        from session_client import status
        connection = status()
        try:
            return self.status(), connection, None
        except AppError as error:
            return None, connection, error

    def refresh_resources(self):
        self.state.catalog = dict(self.store.catalog)
        for theme in self.store.themes:
            for key in theme['characters']:
                self.state.catalog.setdefault(key, {'name': key, 'lifecycle': 'missing'})
        self.state.bubbles = dict(self.store.bubbles.choices)
        self.state.plugins = dict(self.store.plugins.items)
        self.state.themes = deepcopy(self.store.themes)

    def load_resources(self):
        try:
            return self.resource_loader.load()
        except Exception as error:
            raise AppError('resource.failed', 'Could not refresh the resource catalog.', str(error)) from error

    def adopt_resources(self, snapshot):
        self.resource_snapshot = snapshot
        self.resource_revision = snapshot.revision
        self.store.resources.snapshot = snapshot.index
        self.store.catalog = dict(snapshot.characters)
        self.store.bubbles._records = dict(snapshot.bubbles)
        self.store.plugins.items = {key: value for key, value in snapshot.plugins.items() if value.get('lifecycle') != 'missing'}
        self.resource_thumbnails = snapshot.thumbnails
        self.bubble_previews = snapshot.previews
        self.refresh_resources()

    def apply_saved_replacement(self, theme_id):
        """Explicit Library action; never applies an unrelated selected draft."""
        with self.store.resources.leases.mutation():
            current = self.status()
            if not current or (current.get('theme') or {}).get('id') != theme_id:
                raise AppError('resource.runtime_changed', 'The running Theme changed. Refresh its dependencies.')
            store = ThemeStore(self.store.directory)
            theme = store.get(theme_id)
            return self.apply(theme, self.translation(theme_id))

    def resource_operation(self, action, kind, *, progress=None, **arguments):
        from resource_tasks import reporting, report
        from contextlib import nullcontext
        gate = self.store.resources.leases.mutation() if action in ('upgrade', 'uninstall', 'finalize', 'cancel_removal') else nullcontext()
        with gate, reporting(progress):
            report('validating')
            result = self._resource_operation(action, kind, **arguments)
            report('refreshing')
            return result

    def _resource_operation(self, action, kind, *, path=None, key=None, repair=None):
        """Worker-only operations use a separate Store; never mutate live drafts."""
        from resource_dependencies import ResourceInUse
        from runtime_resource_leases import RuntimeResourceInUse
        try:
            store = ThemeStore(self.store.directory)
            store.resources.leases = self.store.resources.leases
            if action == 'adopt':
                from manager_migration import adopt
                return adopt(store, kind, key)
            if action == 'finalize':
                return store.resources.finalize_removal(kind, key)
            if action == 'cancel_removal':
                return store.resources.cancel_removal(kind, key)
            if action == 'approve':
                return store.plugins.approve(key, repair['package_sha256'])
            if action == 'uninstall':
                return store.uninstall_resource(kind, key, **(repair or {}))
            if action not in ('import', 'upgrade') or kind not in ('characters', 'bubbles', 'plugins'):
                raise ValueError('Unknown resource operation.')
            if action == 'upgrade' and key:
                self._verify_upgrade_id(kind, path, key)
            if kind == 'characters':
                result = store.import_character(path, upgrade=action == 'upgrade')
            else:
                service = store.plugins if kind == 'plugins' else store.bubbles
                result = service.install(path, upgrade=action == 'upgrade')
            return ImportResult(kind, result)
        except ThemeConflict as error:
            raise AppError('theme.conflict', 'Theme changed in another window.', str(error)) from error
        except RuntimeResourceInUse as error:
            raise AppError('resource.runtime_in_use', 'Stop the running Theme or Apply a replacement first.', str(error)) from error
        except ResourceInUse as error:
            raise AppError('resource.in_use', 'This resource is still used by a Theme.', str(error)) from error
        except Exception as error:
            raise error_from_exception(error, 'resource') from error

    @staticmethod
    def _verify_upgrade_id(kind, path, key):
        from zipfile import ZipFile
        import tomllib
        filename = {'characters': 'manifest.toml', 'bubbles': 'theme.json', 'plugins': 'mochi-plugin.json'}[kind]
        with ZipFile(path) as archive:
            entries = [item for item in archive.infolist() if item.filename.split('/')[-1] == filename]
            if len(entries) != 1 or entries[0].file_size > 256*1024:
                raise ValueError('Upgrade package metadata is invalid.')
            with archive.open(entries[0]) as source:
                data = source.read(256*1024+1)
            if len(data) > 256*1024:
                raise ValueError('Upgrade metadata is too large.')
            value = tomllib.loads(data.decode()) if kind == 'characters' else json.loads(data)
            if value.get('id') != key:
                raise ValueError('Upgrade package ID does not match the selected resource.')

    def plugin_review(self, key):
        spec = self.state.plugins.get(key)
        if spec and spec.get('origin') == 'local' and spec.get('review') != 'approved':
            return {**{field: spec[field] for field in ('id', 'name', 'runtime', 'capabilities', 'isolation', 'package_sha256')},
                    'session_isolation': spec.get('session_isolation', 'Session isolation unavailable')}
        return None

    def approve_plugin(self, key, digest):
        try:
            return self.store.plugins.approve(key, digest)
        except Exception as error:
            raise AppError('plugin.approval_failed', 'Could not approve the plugin. Review its current package.', str(error)) from error

    def choose(self, key):
        try:
            self.store.choose(key)
            self.state.draft = ThemeDraft(self.store.get(), store_revision=self.store.revision)
            self.refresh_resources()
        except Exception as error:
            raise error_from_exception(error, 'theme') from error

    def create(self, name):
        try:
            self.state.draft = ThemeDraft(self.store.create(name), store_revision=self.store.revision)
            self.refresh_resources()
        except Exception as error:
            raise error_from_exception(error, 'theme') from error

    def save(self):
        try:
            value = self.store.save(self.state.draft.snapshot(), expected_revision=self.state.draft.store_revision)
        except Exception as error:
            issue = (AppError('theme.conflict', 'Theme changed in another window.', str(error))
                     if isinstance(error, ThemeConflict) else error_from_exception(error, 'theme'))
            self.state.draft.failed(issue, isinstance(error, ValueError) and not isinstance(error, (OSError, ThemeConflict)))
            raise issue from error
        self.state.draft.saved(value, self.store.revision)
        try:
            self.refresh_resources()
        except Exception as error:
            raise AppError('theme.catalog_refresh_failed', 'Theme saved, but the resource list could not be refreshed.', str(error)) from error
        return self.state.draft.value

    def reload_latest(self):
        key = self.state.draft.value['id']
        self.store.reload()
        try:
            value = self.store.get(key)
        except StopIteration:
            value = self.store.get()
        self.state.draft = ThemeDraft(value, store_revision=self.store.revision)
        self.refresh_resources()
        return value

    def save_as_new(self):
        value = self.store.save_as_new(self.state.draft.snapshot())
        self.state.draft = ThemeDraft(value, store_revision=self.store.revision)
        self.refresh_resources()
        return value

    def edit_theme(self, **changes):
        self.state.draft.edit(**changes)

    def feature_read_models(self):
        return feature_models(self.state.plugins, self.state.draft.value['plugins'])

    def missing_resource_models(self):
        from manager_missing_resources import missing_resources
        return missing_resources(self.state.draft.value, self.state.catalog,
                                 self.store.bubbles._records, self.state.plugins, check_files=False)

    def bubble_preview_read_model(self):
        key = self.state.draft.value['bubble']
        return self.bubble_previews.get(key) or fallback_bubble(key, self.state.bubbles.get(key, key))

    def character_read_models(self, language=None):
        theme = self.state.draft.value
        result = {}
        for key, item in self.state.catalog.items():
            order = theme['characters'].index(key) if key in theme['characters'] else None
            original = item.get('name', key)
            result[key] = CharacterReadModel(display_name(key, original, language), original, key,
                                             self.resource_thumbnails.get(key), key == theme['primary'],
                                             order, order, item.get('lifecycle', 'active'))
        return result

    def apply_undo(self, command):
        # Reload the durable revision first. A file monitor callback may still
        # be waiting in its debounce window when the user clicks Undo.
        try:
            revision = self.store.reload()
            command.validate(self.state.draft.value, revision)
            if self.state.draft.needs_save:
                raise UndoExpired('Save the current draft before Undo.')
        except UndoExpired:
            raise
        except (OSError, ValueError) as error:
            raise UndoExpired(str(error)) from error
        changes = command.changes()
        try:
            for kind, values, previous in (
                ('characters', changes.get('characters', []), self.state.draft.value['characters']),
                ('plugins', changes.get('plugins', []), self.state.draft.value['plugins']),
                ('bubbles', [changes['bubble']] if 'bubble' in changes else [], [self.state.draft.value['bubble']])):
                for key in set(values) - set(previous):
                    self.store.resources.require_selectable(kind, key)
        except ValueError as error:
            raise UndoExpired(str(error)) from error
        for key in changes.get('plugins', []):
            self.require_plugin_approval(key)
        self.edit_theme(**changes)
        return self.save()

    def restore_cast(self, previous):
        theme = self.state.draft.value
        self.edit_theme(characters=deepcopy(previous['characters']), primary=previous['primary'])
        return self.save()

    def reorder_character(self, source, target):
        characters = self.state.draft.value['characters']
        if source not in characters or target not in characters:
            raise AppError('theme.invalid_order', 'Cannot change this character.')
        if source == target:
            return self.state.draft.value
        old = list(characters)
        characters.remove(source)
        characters.insert(old.index(target), source)
        self.state.draft.changed()
        return self.save()

    def add_character(self, key):
        self.store.resources.require_selectable('characters', key)
        characters = self.state.draft.value['characters']
        if key not in characters:
            if len(characters) >= 4:
                raise AppError('theme.limit', 'This theme supports up to four characters.')
            characters.append(key)
            self.state.draft.changed()
        return self.save()

    def remove_character(self, key):
        theme = self.state.draft.value
        if key == theme['primary']:
            raise AppError('theme.primary_required', 'Choose another primary character before disabling this one.')
        if key in theme['characters']:
            theme['characters'].remove(key)
            self.state.draft.changed()
        return self.save()

    def replace_character(self, key, replacement):
        self.store.resources.require_selectable('characters', replacement)
        theme = self.state.draft.value
        characters = list(theme['characters'])
        if key not in characters or replacement in characters:
            raise AppError('theme.invalid_character', 'Choose a different installed character.')
        characters[characters.index(key)] = replacement
        self.edit_theme(characters=characters, primary=replacement if theme['primary'] == key else theme['primary'])
        return self.save()

    def disable_character(self, key):
        theme = self.state.draft.value
        if key not in theme['characters']:
            return theme
        if len(theme['characters']) == 1:
            raise AppError('theme.last_character', 'A Theme needs at least one character.')
        theme['characters'].remove(key)
        if theme['primary'] == key:
            theme['primary'] = theme['characters'][0]
        self.state.draft.changed()
        return self.save()

    def move_character(self, key, direction):
        characters = self.state.draft.value['characters']
        if key not in characters or direction not in (-1, 1):
            raise AppError('theme.invalid_order', 'Cannot move this character.')
        old = characters.index(key)
        new = old + direction
        if not 0 <= new < len(characters):
            return self.state.draft.value
        characters[old], characters[new] = characters[new], characters[old]
        self.state.draft.changed()
        return self.save()

    def set_primary(self, key):
        theme = self.state.draft.value
        if key not in theme['characters']:
            if len(theme['characters']) >= 4:
                raise AppError('theme.limit', 'Disable another character before adding a new primary.')
            theme['characters'].append(key)
        theme['primary'] = key
        self.state.draft.changed()
        return self.save()

    def require_plugin_approval(self, key, verify=False):
        try:
            self.store.plugins.require_approved(key, verify=verify)
        except PluginApprovalRequired as error:
            raise AppError('plugin.approval_required', str(error), key) from error

    def set_plugin_enabled(self, key, enabled):
        if enabled:
            self.store.resources.require_selectable('plugins', key)
        if enabled:
            self.require_plugin_approval(key)
        plugins = self.state.draft.value['plugins']
        if enabled and key not in plugins:
            plugins.append(key)
        elif not enabled and key in plugins:
            plugins.remove(key)
        self.state.draft.changed()
        return self.save()

    def edit_plugin_setting(self, key, identity, value):
        self.state.draft.value.setdefault('plugin_settings', {}).setdefault(key, {})[identity] = value
        self.state.draft.changed()

    def set_bubble(self, key):
        self.store.resources.require_selectable('bubbles', key)
        self.state.draft.value['bubble'] = key
        self.state.draft.changed()
        return self.save()

    def edit_action(self, key, action, **changes):
        value = self.state.draft.snapshot()
        current = settings(value, key, action)
        current.update(changes)
        value.setdefault('actions', {}).setdefault(key, {})[action['id']] = current
        self.state.draft.value = value
        self.state.draft.changed()
        return self.save()

    def export_theme(self, path):
        try:
            atomic_json(path, self.state.draft.snapshot())
        except Exception as error:
            raise error_from_exception(error, 'export') from error

    def status(self):
        try:
            return self.runtime.status()
        except Exception as error:
            raise error_from_exception(error, 'status') from error

    def apply(self, theme, options):
        try:
            with self.store.resources.leases.mutation():
                self.store.validate_availability(theme)
                for key in theme['plugins']:
                    self.require_plugin_approval(key, verify=True)
                return self.runtime.apply(deepcopy(theme), deepcopy(options))
        except Exception as error:
            raise error_from_exception(error, 'runtime') from error

    def stop(self):
        try:
            with self.store.resources.leases.mutation():
                return self.runtime.stop()
        except Exception as error:
            raise error_from_exception(error, 'runtime') from error

    def same_config(self, left, right):
        return self.runtime.same_config(left, right)

    def import_file(self, kind, path):
        key = None
        try:
            if kind == 'theme':
                if Path(path).stat().st_size > 65536:
                    raise ValueError('Theme 配置过大。')
                value = self.store.validate(json.loads(read_bounded(path, 65536)))
                key = self.store.import_theme(value)['id']
            elif kind == 'character':
                key = self.store.import_character(path)
            elif kind == 'plugin':
                key = self.store.plugins.install(path)
            elif kind == 'bubble':
                key = self.store.bubbles.install(path)
            else:
                raise ValueError('Unknown import kind.')
        except Exception as error:
            raise error_from_exception(error, 'import') from error
        try:
            self.refresh_resources()
        except Exception as error:
            issue = AppError('import.catalog_refresh_failed', 'Import completed, but the resource list could not be refreshed.', str(error))
            return ImportResult(kind, key, 'partial', issue)
        return ImportResult(kind, key)

    def after_import(self):
        self.state.draft = ThemeDraft(self.store.get(), store_revision=self.store.revision)
        self.refresh_resources()

    def plugin_settings(self, spec, configured):
        return settings_for(spec, configured)

    def translation(self, theme_id):
        if self.translation_conflict and self.translation_conflict['theme_id'] == theme_id:
            return deepcopy(self.translation_conflict['mine'])
        return self.preferences.translation(theme_id)

    def applied_translation(self, theme_id):
        return self.preferences.applied_translation(theme_id)

    def mark_translation_applied(self, theme_id, options):
        self.preferences.applied(theme_id, options)

    def edit_translation(self, theme_id, **changes):
        if self.translation_conflict and self.translation_conflict['theme_id'] == theme_id:
            self.translation_conflict['mine'] = translation_options({**self.translation_conflict['mine'], **changes})
            return
        try:
            self.preferences.edit_translation(theme_id, **changes)
        except Exception as error:
            if isinstance(error, PreferencesConflict):
                mine = translation_options({**self.preferences.translation(theme_id), **changes})
                self.preferences.refresh()
                self.translation_conflict = {'theme_id': theme_id, 'mine': mine,
                                             'latest': self.preferences.translation(theme_id)}
            raise (AppError('translation.conflict', 'Translation settings changed in another window.', str(error))
                   if isinstance(error, PreferencesConflict) else error_from_exception(error, 'translation')) from error

    def bubble_material(self, key):
        try:
            return self.store.bubbles.resolve(key)
        except Exception as error:
            raise error_from_exception(error, 'preview') from error

    def preview_binary(self):
        return self.runtime.binary

    def character_preview_bytes(self, key):
        """Supply bounded, cached static art without exposing file access to views."""
        preview = self.state.catalog.get(key, {}).get('preview')
        if not preview:
            return None
        try:
            source = Path(preview)
            stat = source.stat()
            if stat.st_size > 8*1024*1024:
                return None
            identity = (key, str(source), stat.st_mtime_ns, stat.st_size)
            if identity in self.preview_cache:
                self.preview_cache.move_to_end(identity)
                return self.preview_cache[identity]
            from PIL import Image
            with Image.open(BytesIO(read_bounded(source, 8*1024*1024))) as image:
                if image.format != 'PNG' or image.width*image.height > 8_000_000:
                    return None
                image.thumbnail((256, 256), Image.Resampling.LANCZOS)
                with BytesIO() as output:
                    image.save(output, 'PNG')
                    data = output.getvalue()
            if len(data) > 512*1024:
                return None
            self.preview_cache[identity] = data
            self.preview_cache_bytes += len(data)
            while len(self.preview_cache) > 12 or self.preview_cache_bytes > 2*1024*1024:
                _, evicted = self.preview_cache.popitem(last=False)
                self.preview_cache_bytes -= len(evicted)
            return data
        except (OSError, ImportError, ValueError):
            return None

    def brand_icon_bytes(self):
        try:
            return read_bounded(ROOT/'assets/brand/mochi-icon.svg', 1024*1024)
        except (OSError, ValueError):
            return None

    @staticmethod
    def preview_error(error):
        return error_from_exception(error if isinstance(error, Exception) else RuntimeError(str(error)), 'preview')

    def start_model_catalog(self, received, failed):
        return self.catalog_transport.start({'type': 'list_models'}, received, failed, 18000)

    @staticmethod
    def catalog_error(error):
        return error if isinstance(error, AppError) else AppError('catalog.failed', str(error))

    @staticmethod
    def parse_model_catalog(event):
        if event.get('type') == 'error':
            raise AppError('catalog.failed', str(event.get('error', 'Could not load models.')))
        models = event.get('models')
        if (event.get('type') != 'models' or not isinstance(models, list) or not models or len(models) > 256 or any(
                not isinstance(model, dict) or not valid_model(model.get('id'))
                or not isinstance(model.get('name'), str) or len(model['name']) > 160
                or model.get('fast_tier') not in (None, 'fast', 'priority') for model in models)):
            raise AppError('catalog.invalid', 'Invalid model catalog.')
        return {model['id']: model for model in models}

    def stop_model_catalog(self, job):
        self.catalog_transport.stop(job)

    def release_model_catalog(self, job):
        self.catalog_transport.release(job)

    def shutdown(self):
        self.catalog_transport.shutdown()
