"""Theme configuration and local character catalog, independent of GTK/runtime."""
from copy import deepcopy
from contextlib import contextmanager
import fcntl
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
import tempfile
import tomllib
import uuid
from zipfile import ZipFile
from bubble_themes import BubbleThemes, ROOT
from resource_catalog import ResourceCatalog, package_sha256, read_bounded
from plugin_catalog import PluginCatalog, settings_for, ID as PLUGIN_ID

BUILTINS = {
    'shanbi': ('珊比', 'characters/shanbi-sit/shanbi.mochi', 'characters/lime-squad-png/shanbi.mochi/sprites/idle-000.png'),
    'angelina_wish': ('安洁莉娜', 'characters/angelina-drag/angelina_wish.mochi', 'characters/lime-squad-png/angelina_wish.mochi/sprites/idle-000.png'),
    'jacinta': ('嘉辛塔', 'characters/lime-squad-spine/jacinta.mochi', 'characters/lime-squad-png/jacinta.mochi/sprites/idle-000.png'),
    'timeslot': ('时隙', 'characters/lime-squad-spine/timeslot.mochi', 'characters/lime-squad-png/timeslot.mochi/sprites/idle-000.png'),
    'mutsumi': ('若叶睦', 'characters/mutsumi-character-v3/mutsumi-motion-walking.mochi', 'characters/mutsumi-character-v3/mutsumi-motion-walking.mochi/sprites/idle-000.png'),
}
DEFAULT = {'format': 'mochi-theme-v1', 'id': 'lime-squad', 'name': '酸橙小队', 'primary': 'shanbi',
           'characters': ['shanbi', 'angelina_wish', 'jacinta', 'timeslot'], 'plugins': ['translation'],
           'bubble': 'lime-notes', 'size': 256, 'idle_after': 30}


# Public staging builds contain only an original procedural demo. Existing
# user Themes and imported Resources retain their own catalog records.
LEGACY_BUILTINS = deepcopy(BUILTINS)
from mochi_paths import INSTALLED, PUBLIC_SOURCE
if INSTALLED or PUBLIC_SOURCE:
    BUILTINS = {'mochi-dot': ('Mochi Dot', 'characters/mochi-dot.mochi',
                            'characters/mochi-dot.mochi/sprites/dot.png')}
    DEFAULT = {**DEFAULT, 'id': 'welcome', 'name': 'Welcome', 'primary': 'mochi-dot',
               'characters': ['mochi-dot'], 'bubble': 'default'}


class ThemeConflict(ValueError):
    def __init__(self, expected, actual):
        self.expected, self.actual = expected, actual
        super().__init__(f'Theme configuration changed in another window (revision {expected} → {actual}).')


def atomic_json(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    with tempfile.NamedTemporaryFile(mode='w', dir=path.parent, prefix='.'+path.name, delete=False) as stream:
        temporary = Path(stream.name)
        try:
            json.dump(value, stream, ensure_ascii=False, indent=2)
            stream.write('\n')
            stream.flush()
            os.fsync(stream.fileno())
        except BaseException:
            temporary.unlink(missing_ok=True)
            raise
    try:
        temporary.replace(path)
    finally:
        temporary.unlink(missing_ok=True)


class ThemeStore:
    def __init__(self, directory=None, bubbles=None):
        self.directory = Path(directory or Path(os.environ.get('XDG_CONFIG_HOME', str(Path.home()/'.config')))/'mochi')
        self.file = self.directory/'themes.json'
        self.lock_file = self.directory/'themes.json.lock'
        self.resources = ResourceCatalog(self.directory)
        self.legacy_checkouts = []
        if INSTALLED:
            from manager_migration import discover_checkouts, discover_legacy_records, migrate_launchers
            self.legacy_checkouts = discover_checkouts(self.resources)
            discover_legacy_records(self.resources, self.legacy_checkouts, LEGACY_BUILTINS)
            migrate_launchers(self.legacy_checkouts)
        self.bubbles = bubbles or BubbleThemes(preferences=self.directory/'appearance.json', resources=self.resources)
        self.plugins = PluginCatalog(self.directory, self.resources)
        self.catalog = {key: {'name': name, 'path': str(ROOT/path), 'preview': str(ROOT/preview), 'origin': 'bundled'}
                        for key, (name, path, preview) in BUILTINS.items()}
        self._builtins = dict(self.catalog)
        with self._lock():
            self._reload_locked()
        self.catalog = self.resources.reconcile('characters', self._reconcile_characters)
        with self._lock():
            self._reload_locked()

    def _reconcile_characters(self, current):
        result = {**self.catalog, **current, **self._builtins}
        for root in self.legacy_checkouts:
            for key, (name, path, preview) in LEGACY_BUILTINS.items():
                if key not in result and (Path(root)/path/'manifest.toml').is_file():
                    result[key] = {'name': name, 'path': str(Path(root)/path), 'preview': str(Path(root)/preview), 'origin': 'bundled'}
        for key, item in result.items():
            if item.get('origin') == 'bundled' and key not in self._builtins:
                item.update(lifecycle='missing', legacy_bundled=True)
            elif not (Path(item.get('path', ''))/'manifest.toml').is_file():
                item['lifecycle'] = 'missing'
            manifest = Path(item.get('path', ''))/'manifest.toml'
            if manifest.is_file():
                try:
                    metadata = tomllib.loads(read_bounded(manifest, 256*1024).decode('utf-8'))
                    item['requires_spine'] = any(component.get('id') == 'mochi.appearance.spine' for component in metadata.get('components', []))
                except (OSError, ValueError, AttributeError):
                    item['lifecycle'] = 'missing'
        return result

    @contextmanager
    def _lock(self):
        with self.resources.locked():
            self.resources.snapshot = self.resources.read()
            yield

    def _reload_locked(self):
        self.catalog = {**self._builtins, **self.resources.snapshot['characters']}
        self.custom = {}
        if self.file.exists():
            raw = json.loads(self.file.read_text())
            if not isinstance(raw, dict) or raw.get('version') != 1:
                raise ValueError('Theme 配置格式不支持，原文件已保留。')
            revision = raw.get('revision', 0)
            if type(revision) is not int or revision < 0:
                raise ValueError('Theme revision 无效，原文件已保留。')
            self.revision = revision
            selected_revision = raw.get('selected_revision', 0)
            if type(selected_revision) is not int or selected_revision < 0:
                raise ValueError('Theme selection revision 无效，原文件已保留。')
            self.selected_revision = selected_revision
            self.custom = raw.get('characters', {})
            if not isinstance(self.custom, dict) or len(self.custom) > 128:
                raise ValueError('角色目录无效。')
            for key, item in self.custom.items():
                if key in self._builtins or not isinstance(item, dict):
                    raise ValueError('角色目录存在重复或无效项。')
                legacy = self.directory/'characters'/key
                path = self.directory/'characters'/(key+'.mochi')
                if not re.fullmatch(r'[a-zA-Z0-9][a-zA-Z0-9._-]{0,127}', key) or any(not p.resolve().is_relative_to((self.directory/'characters').resolve()) for p in (legacy, path)):
                    raise ValueError('角色目录名称无效。')
                # The Rust package reader requires the .mochi suffix. Preserve
                # early imports by moving their directory without changing IDs.
                if legacy.exists() and not path.exists():
                    path = legacy
                if key not in self.catalog:
                    self.catalog[key] = ({**self.character(path), 'origin': 'local', 'package_sha256': package_sha256(path)}
                                         if (path/'manifest.toml').is_file() else
                                         {'name': key, 'path': str(path), 'origin': 'local', 'lifecycle': 'missing'})
            themes = raw.get('themes')
            if not isinstance(themes, list) or not 1 <= len(themes) <= 64:
                raise ValueError('Theme 列表无效。')
            self.themes = [self.validate_schema(theme) for theme in themes]
            if len({theme['id'] for theme in self.themes}) != len(themes):
                raise ValueError('Theme 名称重复。')
            self.selected = raw.get('selected')
            if self.selected not in {theme['id'] for theme in self.themes}:
                self.selected = self.themes[0]['id']
        else:
            self.revision = 0
            self.selected_revision = 0
            initial = deepcopy(DEFAULT)
            initial['bubble'] = self.bubbles.selected
            self.themes, self.selected = [initial], initial['id']

        if INSTALLED and not any(not self.missing_resources(theme) for theme in self.themes):
            welcome = deepcopy(DEFAULT)
            while welcome['id'] in {theme['id'] for theme in self.themes}:
                welcome['id'] += '-new'
            self.themes.append(welcome)
            # Preserve selected old Theme so the user sees what needs repair.

    def reload(self):
        with self._lock():
            self._reload_locked()
            return self.revision

    def validate(self, value, require_plugins=True):
        result = self.validate_schema(value)
        if require_plugins:
            self.validate_availability(result)
            for key, options in result.get('plugin_settings', {}).items():
                if key in self.plugins.items:
                    settings_for(self.plugins.items[key], options)
            if 'actions' in result:
                from character_actions import validate
                validate(result['actions'], self.catalog)
        return result

    def missing_resources(self, value):
        from manager_missing_resources import missing_resources
        return missing_resources(value, self.catalog, self.bubbles._records, self.plugins.items)

    def validate_availability(self, value):
        missing = self.missing_resources(value)
        if missing:
            raise ValueError('Theme resources unavailable: ' + ', '.join(item.label + ': ' + item.technical_id for item in missing))

    def validate_schema(self, value):
        if not isinstance(value, dict) or set(value)-{'actions', 'plugin_settings'} != set(DEFAULT) or value.get('format') != 'mochi-theme-v1':
            raise ValueError('请选择 Mochi Theme 配置。')
        if not isinstance(value['id'], str) or not re.fullmatch(r'[a-z0-9][a-z0-9-]{0,63}', value['id']):
            raise ValueError('Theme ID 无效。')
        if not isinstance(value['name'], str) or not value['name'].strip() or len(value['name']) > 60:
            raise ValueError('Theme 名称需要 1–60 个字。')
        roles = value['characters']
        if not isinstance(roles, list) or not 1 <= len(roles) <= 4 or any(not isinstance(key, str) or not re.fullmatch(r'[a-zA-Z0-9][a-zA-Z0-9._-]{0,127}', key) for key in roles) or len(set(roles)) != len(roles):
            raise ValueError('每个 Theme 需要 1–4 个已导入角色。')
        if value['primary'] not in roles:
            raise ValueError('请选择一个已启用的主角色。')
        plugins = value['plugins']
        if not isinstance(plugins, list) or len(plugins) > 8 or any(not isinstance(key, str) or not re.fullmatch(PLUGIN_ID, key) for key in plugins) or len(set(plugins)) != len(plugins):
            raise ValueError('Theme 最多启用 8 个不同插件。')
        settings = value.get('plugin_settings', {})
        if not isinstance(settings, dict) or len(settings) > 64 or len(json.dumps(settings)) > 65536:
            raise ValueError('插件设置格式无效或过大。')
        for key, options in settings.items():
            if not isinstance(key, str) or not re.fullmatch(PLUGIN_ID, key) or not isinstance(options, dict):
                raise ValueError('插件设置格式无效。')
        if not isinstance(value['bubble'], str) or not re.fullmatch(r'[a-z0-9][a-z0-9-]{0,63}', value['bubble']):
            raise ValueError('Invalid Bubble Theme.')
        if type(value['size']) is not int or not 128 <= value['size'] <= 512:
            raise ValueError('角色大小需要在 128–512 之间。')
        if type(value['idle_after']) is not int or not 0 <= value['idle_after'] <= 3600:
            raise ValueError('待机时间需要在 0–3600 秒之间。')
        result = deepcopy(value)
        if 'actions' in result:
            from character_actions import validate_schema
            result['actions'] = validate_schema(result['actions'])
        result['name'] = result['name'].strip()
        return result

    def require_active_theme(self, value):
        self.validate_availability(value)
        for kind, keys in (('characters', value['characters']), ('bubbles', [value['bubble']]), ('plugins', value['plugins'])):
            for key in keys:
                self.resources.require_selectable(kind, key)

    def get(self, key=None):
        return deepcopy(next(theme for theme in self.themes if theme['id'] == (key or self.selected)))

    def persist(self):
        atomic_json(self.file, {'version': 1, 'revision': self.revision,
                                'selected_revision': self.selected_revision, 'selected': self.selected,
                                'themes': self.themes, 'characters': self.custom})

    def _change(self, mutate, expected_revision=None):
        with self._lock():
            self._reload_locked()
            if expected_revision is not None and self.revision != expected_revision:
                raise ThemeConflict(expected_revision, self.revision)
            old_themes, old_selected = self.themes, self.selected
            old_revision, old_selected_revision = self.revision, self.selected_revision
            try:
                result = mutate()
                self.revision += 1
                if self.selected != old_selected:
                    self.selected_revision += 1
                self.persist()
                return result
            except BaseException:
                self.themes, self.selected, self.revision = old_themes, old_selected, old_revision
                self.selected_revision = old_selected_revision
                raise

    def save(self, value, expected_revision=None):
        def mutate():
            validated = self.validate(value, require_plugins=False)
            if 'actions' in validated:
                from character_actions import validate
                available = {key: options for key, options in validated['actions'].items() if key in self.catalog and self.catalog[key].get('lifecycle', 'active') == 'active'}
                validate(available, self.catalog)
            baseline = next((item for item in self.themes if item['id'] == validated['id']), {})
            if any(key not in self.plugins.items for key in set(validated['plugins']) - set(baseline.get('plugins', []))):
                raise ValueError('Plugin is not installed.')
            if validated['bubble'] != baseline.get('bubble'):
                self.bubbles.cached_resolve(validated['bubble'])
            for kind, keys, old in (('characters', validated['characters'], baseline.get('characters', [])),
                                    ('plugins', validated['plugins'], baseline.get('plugins', [])),
                                    ('bubbles', [validated['bubble']], [baseline.get('bubble')])):
                for key in set(keys) - set(old):
                    self.resources.require_selectable(kind, key)
            previous = self.themes
            if not any(item['id'] == validated['id'] for item in previous) and len(previous) >= 64:
                raise ValueError('Theme 数量已达到上限。')
            self.themes = [validated if item['id'] == validated['id'] else item for item in previous]
            if not any(item['id'] == validated['id'] for item in previous):
                self.themes.append(validated)
            return deepcopy(validated)
        return self._change(mutate, expected_revision)

    def choose(self, key):
        with self._lock():
            self._reload_locked()
            self.get(key)
            if self.selected == key:
                return
            old_selected, old_selected_revision = self.selected, self.selected_revision
            self.selected = key
            self.selected_revision += 1
            try:
                self.persist()
            except BaseException:
                self.selected, self.selected_revision = old_selected, old_selected_revision
                raise

    def create(self, name='新的 Theme'):
        value = deepcopy(DEFAULT)
        value.update(id='theme-'+uuid.uuid4().hex[:12], name=name, characters=list(DEFAULT['characters'][:1]), primary=DEFAULT['primary'], bubble=self.bubbles.selected)
        return self.import_theme(value)

    def import_theme(self, value):
        def mutate():
            validated = self.validate(value)
            self.require_active_theme(validated)
            if any(item['id'] == validated['id'] for item in self.themes):
                raise ValueError('同名 Theme 已存在，未覆盖当前配置。')
            if len(self.themes) >= 64:
                raise ValueError('Theme 数量已达到上限。')
            self.themes = [*self.themes, validated]
            self.selected = validated['id']
            return deepcopy(validated)
        return self._change(mutate)

    def save_as_new(self, value):
        copy = deepcopy(value)
        if not isinstance(copy.get('name'), str) or not copy['name'].strip():
            raise ValueError('Theme 名称需要 1–60 个字。')
        copy['id'] = 'theme-'+uuid.uuid4().hex[:12]
        copy['name'] = (copy['name'].strip()[:52] + ' (copy)')[:60]
        return self.import_theme(copy)

    def paths(self, theme):
        value = self.validate(theme)
        keys = [value['primary'], *(key for key in value['characters'] if key != value['primary'])]
        return keys, [self.catalog[key]['path'] for key in keys]

    @staticmethod
    def character(path):
        manifest = tomllib.loads(read_bounded(path/'manifest.toml', 256*1024).decode('utf-8'))
        if manifest.get('schema_version') != 1 or not isinstance(manifest.get('name'), str) or not 1 <= len(manifest['name']) <= 80:
            raise ValueError('角色需要有效的 manifest.toml。')
        if not manifest.get('components'):
            raise ValueError('角色没有配置动作或外观。')
        preview = next(iter(sorted(path.glob('sprites/idle-*.png'))), None)
        if preview is None:
            preview = next(iter(sorted(path.glob('sprites/*.png'))), None)
        return {'name': manifest['name'], 'path': str(path), 'preview': str(preview) if preview else None,
                'requires_spine': any(component.get('id') == 'mochi.appearance.spine' for component in manifest['components'])}

    def import_character(self, archive, upgrade=False):
        from contextlib import nullcontext
        with self.resources.leases.mutation() if upgrade else nullcontext():
            return self._import_character(archive, upgrade)

    def _import_character(self, archive, upgrade=False):
        destination_root = self.directory/'characters'
        destination_root.mkdir(parents=True, exist_ok=True)
        with ZipFile(archive) as bundle, tempfile.TemporaryDirectory(prefix='.import-', dir=destination_root) as temporary:
            entries = bundle.infolist()
            if len(entries) > 512 or sum(entry.file_size for entry in entries) > 64*1024*1024:
                raise ValueError('角色包过大。')
            files, manifests = {}, []
            for entry in entries:
                key = PurePosixPath(entry.filename)
                if key.is_absolute() or '..' in key.parts or '\\' in entry.filename or stat.S_ISLNK(entry.external_attr >> 16):
                    raise ValueError('角色包包含无效路径。')
                if entry.is_dir():
                    continue
                if key in files:
                    raise ValueError('角色包包含重复文件。')
                files[key] = entry
                if key.name == 'manifest.toml':
                    manifests.append(key)
            if len(manifests) != 1 or len(files) > 256:
                raise ValueError('请选择只包含一个 .mochi 角色目录的 ZIP 包（最多 256 个文件）。')
            base = manifests[0].parent
            path = Path(temporary)/'character'
            path.mkdir()
            for key, entry in files.items():
                if not key.is_relative_to(base):
                    raise ValueError('压缩包包含角色目录外的文件。')
                target = path/key.relative_to(base)
                target.parent.mkdir(parents=True, exist_ok=True)
                with bundle.open(entry) as source, target.open('wb') as output:
                    import shutil
                    shutil.copyfileobj(source, output)
            item = self.character(path)
            manifest = tomllib.loads(read_bounded(path/'manifest.toml', 256*1024).decode('utf-8'))
            key = manifest.get('id')
            if not isinstance(key, str) or not re.fullmatch(r'[a-zA-Z0-9][a-zA-Z0-9._-]{0,127}', key):
                raise ValueError('角色 ID 无效。')
            destination = destination_root/(key+'.mochi')
            if upgrade:
                self.resources.leases.assert_unused('characters', key)
            with self._lock():
                self._reload_locked()
                if key in self._builtins:
                    raise ValueError('Bundled characters cannot be replaced.')
                record = {**item, 'path': str(destination), 'origin': 'local',
                          'preview': str(destination/Path(item['preview']).relative_to(path)) if item['preview'] else None,
                          'package_sha256': package_sha256(path)}
                self.resources._replace_package_locked('characters', key, path, destination, record, upgrade)
                self.catalog[key] = record
                return key

    def upgrade_character(self, archive):
        return self.import_character(archive, upgrade=True)

    def uninstall_resource(self, kind, key, *, replacement=None, disable=False, expected_revision=None, repair_only=False):
        with self.resources.leases.mutation():
            if not repair_only:
                self.resources.leases.assert_unused(kind, key)
            return self.resources_uninstall_locked(kind, key, replacement, disable, expected_revision, repair_only)

    def resources_uninstall_locked(self, kind, key, replacement, disable, expected_revision, repair_only):
        with self._lock():
            self._reload_locked()
            current = self.resources.read()[kind].get(key)
            if not current:
                raise ValueError('Resource is not installed.')
            destination = current.get('root') or current.get('path')
            from resource_dependencies import _uninstall_locked
            result = _uninstall_locked(self.resources, kind, key, destination, replacement=replacement,
                                              disable=disable, expected_revision=expected_revision, repair_only=repair_only)
            self.resources.snapshot = self.resources.read()
            self._reload_locked()
            return result

    def uninstall_character(self, key):
        return self.uninstall_resource('characters', key)
