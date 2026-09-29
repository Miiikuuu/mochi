"""DIY bubble material selection, independent of messages and translation."""
import json
import logging
import os
from pathlib import Path
import re
import stat
import tempfile
from pathlib import PurePosixPath
from zipfile import ZipFile
from resource_catalog import ResourceCatalog, package_sha256, read_bounded

from mochi_paths import DATA as ROOT
MAX_FILE_BYTES = 8*1024*1024
MAX_TEXT_BYTES = 256*1024


class BubbleThemes:
    def __init__(self, directory=None, preferences=None, resources=None, bundled_directory=None):
        self.preferences = Path(preferences or Path(os.environ.get('XDG_CONFIG_HOME', str(Path.home()/'.config')))/'mochi/appearance.json')
        self.directory = Path(directory or self.preferences.parent/'bubbles')
        self.bundled_directory = Path(bundled_directory) if bundled_directory else (ROOT/'assets/bubbles' if directory is None else None)
        self.resources = resources or ResourceCatalog(self.preferences.parent, roots={'bubbles': self.directory})
        self._records = {}
        self.selected = 'default'
        try:
            selected = json.loads(self.preferences.read_text()).get('bubble_theme', 'default')
            self.resolve(selected)
            self.selected = selected
        except (OSError, ValueError, TypeError, AttributeError) as error:
            if not isinstance(error, FileNotFoundError):
                logging.warning('Using the default bubble theme: %s', error)
        self.refresh()

    def resolve(self, name):
        if not isinstance(name, str) or not re.fullmatch(r'[a-z0-9][a-z0-9-]{0,63}', name):
            raise ValueError('无效的气泡主题名称。')
        base = self.directory
        adopted = self.resources.data_root/'bubbles' if hasattr(self, 'resources') else base
        if not (base/name).exists() and (adopted/name).exists():
            base = adopted
        if not (base/name).exists() and self.bundled_directory:
            base = self.bundled_directory
        path = base/name
        if not path.resolve().is_relative_to(base.resolve()):
            raise ValueError('气泡主题路径超出了素材目录。')
        data = json.loads(read_bounded(path/'theme.json', 65536))
        if not isinstance(data, dict) or data.get('id') != name or data.get('format') != 'mochi-bubble-v1':
            raise ValueError('不支持的气泡主题格式。')
        if not isinstance(data.get('name'), str) or not 1 <= len(data['name']) <= 80:
            raise ValueError('气泡主题缺少名称。')
        from resource_metadata import validate_i18n
        validate_i18n(data)
        if not isinstance(data.get('description', ''), str) or len(data.get('description', '')) > 500:
            raise ValueError('Invalid Bubble description.')
        for filename in ['layout.json', 'speech.css']:
            if not (path/filename).is_file():
                raise ValueError('气泡主题缺少 '+filename)
        for file in path.rglob('*'):
            if file.is_file():
                maximum = MAX_TEXT_BYTES if file.suffix in ('.css', '.json') else MAX_FILE_BYTES
                if file.stat().st_size > maximum:
                    raise ValueError('Bubble file is too large: '+file.name)
        return path

    def cached_resolve(self, name):
        if name not in self._records or self._records[name].get('lifecycle') == 'missing':
            raise ValueError('Bubble Theme is not installed.')
        return Path(self._records[name]['root'])

    def refresh(self):
        def scan(previous):
            records = {}
            bases = [self.directory, self.resources.data_root/'bubbles'] + ([self.bundled_directory] if self.bundled_directory else [])
            for path in sorted(path for base in bases for path in base.glob('*')):
                try:
                    directory = self.resolve(path.name)
                    metadata = json.loads(read_bounded(directory/'theme.json', 65536))
                    records[path.name] = {'description': metadata.get('description', ''), 'i18n': metadata.get('i18n', {}), 'name': metadata['name'], 'root': str(directory),
                        'origin': previous.get(path.name, {}).get('origin', 'bundled' if directory.parent == self.bundled_directory else 'local'),
                        'package_sha256': package_sha256(directory)}
                except (OSError, ValueError, TypeError):
                    continue
            return records
        self._records = self.resources.reconcile('bubbles', scan)

    @property
    def choices(self):
        from resource_metadata import localized
        return {key: localized(record)['name'] for key, record in self._records.items() if record.get('lifecycle', 'active') == 'active'}

    def upgrade(self, archive):
        return self.install(archive, upgrade=True)

    def uninstall(self, key):
        self.resources.uninstall('bubbles', key, self.directory/key)
        self.refresh()

    def select(self, name):
        self.resolve(name)
        try:
            values = json.loads(self.preferences.read_text())
            if not isinstance(values, dict):values = {}
        except (OSError, ValueError):
            values = {}
        values['bubble_theme'] = name
        self.preferences.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        temp = self.preferences.with_suffix('.tmp')
        with temp.open('w') as stream:
            os.chmod(temp, 0o600)
            json.dump(values, stream, ensure_ascii=False, indent=2)
        temp.replace(self.preferences)
        self.selected = name

    def install(self, archive, upgrade=False):
        """Install a native .bubble.zip as data, without running packaged code."""
        with ZipFile(archive) as zip:
            entries = zip.infolist()
            if len(entries)>512 or sum(e.file_size for e in entries)>64*1024*1024:
                raise ValueError('气泡包过大。')
            files = {}
            for entry in entries:
                path = PurePosixPath(entry.filename)
                if path.is_absolute() or '..' in path.parts or stat.S_ISLNK(entry.external_attr>>16):
                    raise ValueError('气泡包包含无效路径。')
                if entry.is_dir():continue
                maximum = MAX_TEXT_BYTES if path.suffix in ('.css', '.json') else MAX_FILE_BYTES
                if entry.file_size > maximum:
                    raise ValueError('Bubble file is too large: '+path.name)
                if path in files:raise ValueError('气泡包包含重复文件。')
                files[path] = zip.read(entry)
        manifests = [path for path in files if path.name=='theme.json']
        if len(manifests)!=1:raise ValueError('请选择包含一个 theme.json 的原生气泡包。')
        manifest = manifests[0]
        data = json.loads(files[manifest])
        name = data.get('id') if isinstance(data,dict) else None
        if not isinstance(name,str) or not re.fullmatch(r'[a-z0-9][a-z0-9-]{0,63}',name):
            raise ValueError('气泡主题 ID 无效。')
        destination = self.directory/name
        if name in self._records and self._records[name].get('origin') == 'bundled':
            raise ValueError('Bundled Bubble Themes cannot be replaced.')
        if destination.exists() and not upgrade:raise ValueError('同名主题已存在，未覆盖 DIY 修改。')
        self.directory.mkdir(parents=True,exist_ok=True)
        with tempfile.TemporaryDirectory(prefix='.bubble-import-',dir=self.directory) as temp:
            theme = Path(temp)/name;theme.mkdir()
            for path,content in files.items():
                if not path.is_relative_to(manifest.parent):raise ValueError('气泡包包含主题目录外的文件。')
                target=theme/path.relative_to(manifest.parent)
                target.parent.mkdir(parents=True,exist_ok=True);target.write_bytes(content)
            validator = object.__new__(BubbleThemes)
            validator.directory = Path(temp)
            validator.bundled_directory = None
            validator.resolve(name)
            record = {'description': data.get('description', ''), 'i18n': data.get('i18n', {}), 'name': data['name'], 'root': str(destination), 'origin': 'local',
                      'package_sha256': package_sha256(theme)}
            self.resources.replace_package('bubbles', name, theme, destination, record, upgrade)
        self.refresh()
        return name
