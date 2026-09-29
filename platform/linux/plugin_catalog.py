"""Versioned local plugin manifests and transactional ZIP import; no code execution."""
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import tempfile
from zipfile import ZipFile
from resource_catalog import ResourceCatalog, package_sha256, read_bounded

from mochi_paths import DATA as ROOT
ID = r'[a-z][a-z0-9_-]{0,63}'


def relative_file(root, value):
    if not isinstance(value, str) or not value or '\\' in value:
        raise ValueError('插件文件路径无效。')
    path = PurePosixPath(value)
    target = root/path
    if path.is_absolute() or '..' in path.parts or not target.resolve().is_relative_to(root.resolve()) or not target.is_file() or target.is_symlink():
        raise ValueError('插件文件不存在或路径越界。')
    return target


def setting_value(field, value):
    kind = field['type']
    if kind == 'boolean':
        valid = type(value) is bool
    elif kind == 'integer':
        valid = type(value) is int and field.get('min', -1000000) <= value <= field.get('max', 1000000)
    elif kind == 'choice':
        valid = isinstance(value, str) and value in field['choices']
    else:
        valid = isinstance(value, str) and len(value) <= field.get('max_length', 500)
    if not valid:
        raise ValueError('插件设置无效：'+field['label'])
    return value


def settings_for(spec, values=None):
    fields = {field['id']: field for field in spec['settings']}
    if values is None:
        values = {}
    if not isinstance(values, dict) or set(values)-set(fields):
        raise ValueError('插件包含未知设置。')
    return {key: setting_value(field, values.get(key, field['default'])) for key, field in fields.items()}


def read_manifest(root, builtin=False):
    file = root/'mochi-plugin.json'
    if file.stat().st_size > 65536:
        raise ValueError('插件描述文件过大。')
    value = json.loads(read_bounded(file, 65536))
    allowed = {'format', 'api_version', 'id', 'name', 'version', 'description', 'runtime', 'entry', 'icon', 'settings', 'timeout_ms', 'capabilities', 'i18n'}
    if not isinstance(value, dict) or set(value)-allowed or value.get('format') != 'mochi-plugin-v1' or type(value.get('api_version')) is not int or value['api_version'] != 1:
        raise ValueError('不支持的 Mochi 插件协议。')
    if not isinstance(value.get('id'), str) or not re.fullmatch(ID, value['id']):
        raise ValueError('插件 ID 无效。')
    if not isinstance(value.get('name'), str) or not 1 <= len(value['name'].strip()) <= 60:
        raise ValueError('插件名称无效。')
    if not isinstance(value.get('version'), str) or not re.fullmatch(r'\d+\.\d+\.\d+(?:-[a-zA-Z0-9.-]+)?', value['version']):
        raise ValueError('插件版本需要形如 1.0.0。')
    if not isinstance(value.get('description', ''), str) or len(value.get('description', '')) > 500:
        raise ValueError('插件说明过长。')
    from resource_metadata import validate_i18n
    validate_i18n(value)
    runtime = value.get('runtime')
    capabilities = value.get('capabilities', [])
    if not isinstance(capabilities, list) or any(item not in ('external_messages', 'external_replies') for item in capabilities) or len(capabilities) != len(set(capabilities)) or len(capabilities) > 2 or ('external_replies' in capabilities and 'external_messages' not in capabilities):
        raise ValueError('插件声明了不支持的能力。')
    if runtime == 'builtin':
        # Only shipped manifests can select in-process adapters.
        if not builtin or value.get('entry') != 'translation_plugin:TranslationPlugin' or value['id'] != 'translation':
            raise ValueError('导入插件必须使用独立进程。')
    elif runtime in ('python3', 'node'):
        relative_file(root, value.get('entry'))
    else:
        raise ValueError('插件运行方式需要 python3 或 node。')
    if 'icon' in value:
        icon = relative_file(root, value['icon'])
        if icon.suffix.lower() not in ('.svg', '.png') or icon.stat().st_size > 256*1024:
            raise ValueError('插件图标需要小于 256 KiB 的 PNG 或 SVG。')
    timeout = value.get('timeout_ms', 30000)
    if type(timeout) is not int or not 1000 <= timeout <= 300000:
        raise ValueError('插件请求时限需要在 1–300 秒之间。')
    fields = value.get('settings', [])
    if not isinstance(fields, list) or len(fields) > 16:
        raise ValueError('插件最多声明 16 项设置。')
    ids = set()
    for field in fields:
        if not isinstance(field, dict) or set(field)-{'id', 'label', 'type', 'default', 'choices', 'min', 'max', 'max_length'}:
            raise ValueError('插件设置格式无效。')
        key = field.get('id')
        if not isinstance(key, str) or not re.fullmatch(ID, key) or key in ids:
            raise ValueError('插件设置 ID 重复或无效。')
        ids.add(key)
        if not isinstance(field.get('label'), str) or not 1 <= len(field['label']) <= 60 or 'default' not in field:
            raise ValueError('插件设置缺少名称或默认值。')
        kind = field.get('type')
        if kind not in ('string', 'boolean', 'integer', 'choice'):
            raise ValueError('不支持的插件设置类型。')
        if kind == 'choice':
            choices = field.get('choices')
            if not isinstance(choices, dict) or not 1 <= len(choices) <= 32 or any(not isinstance(k, str) or not 1 <= len(k) <= 80 or not isinstance(v, str) or not 1 <= len(v) <= 80 for k, v in choices.items()):
                raise ValueError('插件选项无效。')
        if kind == 'integer' and (any(type(field.get(k, d)) is not int for k, d in [('min', -1000000), ('max', 1000000)]) or not -1000000 <= field.get('min', -1000000) <= field.get('max', 1000000) <= 1000000):
            raise ValueError('插件数值范围无效。')
        if kind == 'string' and (type(field.get('max_length', 500)) is not int or not 1 <= field.get('max_length', 500) <= 4000):
            raise ValueError('插件文字长度无效。')
        setting_value(field, field['default'])
    return {**value, 'settings': fields, 'capabilities': capabilities, 'description': value.get('description', ''), 'timeout_ms': timeout, 'root': root, 'builtin': builtin}


class PluginCatalog:
    def __init__(self, directory=None, resources=None):
        self.directory = Path(directory or Path(os.environ.get('XDG_CONFIG_HOME', str(Path.home()/'.config')))/'mochi')/'plugins'
        self.resources = resources or ResourceCatalog(self.directory.parent)
        self.items, self.errors = {}, {}
        self.refresh()

    @staticmethod
    def metadata(spec, previous=None):
        bundled = spec['builtin']
        from plugin_launch import session_isolation_available
        isolated = not bundled and session_isolation_available()
        files = []
        if not bundled:
            files = [path.relative_to(spec['root']).as_posix()
                     for path in sorted(spec['root'].rglob('*')) if path.is_file()]
        digest = (hashlib.sha256(read_bounded(spec['root']/'mochi-plugin.json', 65536)).hexdigest()
                  if bundled else package_sha256(spec['root'], 16*1024*1024, files))
        approval = (previous or {}).get('approval')
        approved = bundled or bool(approval and approval.get('package_sha256') == digest and approval.get('uid') == os.getuid())
        return {**spec, 'root': str(spec['root']), 'origin': 'bundled' if bundled else 'local',
                'review': 'approved' if approved else 'unreviewed', 'isolation': 'sandboxed' if isolated else 'unsandboxed',
                'session_isolation': ('trusted bundled adapter' if bundled else ('Session socket hidden' if isolated else 'Session isolation unavailable')),
                'package_sha256': digest, 'package_files': files, 'approval': approval if approved and not bundled else None}

    def refresh(self):
        items, errors = {}, {}
        def scan(previous):
            records = {}
            for base, builtin in [(ROOT/'plugins', True), (self.directory, False), (self.resources.data_root/'plugins', False)]:
                for file in sorted(base.glob('*/mochi-plugin.json')):
                    try:
                        spec = read_manifest(file.parent, builtin)
                        key = spec['id']
                        if key in items or not builtin and file.parent.name != key:
                            raise ValueError('插件 ID 重复或目录不匹配。')
                        record = self.metadata(spec, previous.get(key))
                        from resource_metadata import localized
                        items[key] = {**localized(record), 'root': Path(record['root'])}
                        records[key] = record
                    except (OSError, ValueError, TypeError, KeyError) as error:
                        errors[str(file.parent)] = str(error)
            return records
        records = self.resources.reconcile('plugins', scan)
        from resource_metadata import localized
        self.items = {key: {**localized(record), 'root': Path(record['root'])} for key, record in records.items() if record.get('lifecycle') != 'missing'}
        self.errors = errors

    def require_approved(self, key, verify=True):
        spec = self.items.get(key, {})
        if not spec.get('builtin') and 'external_replies' in spec.get('capabilities', []):
            raise ValueError('Session isolation unavailable: third-party external_replies is not supported.')
        # Every entry point checks the actual bytes and exact file set. The
        # compatibility argument cannot turn integrity verification off.
        with self.resources.locked():
            current = self.resources.read()
            record = current['plugins'].get(key)
            if not record:
                raise PluginApprovalRequired(key)
            if record.get('origin') == 'bundled':
                return
            approval = record.get('approval') or {}
            try:
                digest = package_sha256(record['root'], 16*1024*1024, record.get('package_files'))
                valid = (record.get('review') == 'approved' and
                         approval.get('package_sha256') == digest and approval.get('uid') == os.getuid())
            except (OSError, ValueError):
                valid = False
            if not valid:
                if record.get('approval') or record.get('review') != 'unreviewed':
                    record.update(review='unreviewed', approval=None)
                    self.resources._commit(current)
                if key in self.items:
                    self.items[key].update(review='unreviewed', approval=None)
                raise PluginApprovalRequired(key)

    def approve(self, key, expected_sha256):
        record = self.resources.approve_plugin(key, expected_sha256)
        from resource_metadata import localized
        self.items[key] = {**localized(record), 'root': Path(record['root'])}
        return self.items[key]

    def uninstall(self, key):
        self.resources.uninstall('plugins', key, self.directory/key)
        self.refresh()

    def upgrade(self, archive):
        return self.install(archive, upgrade=True)

    def install(self, archive, upgrade=False):
        self.directory.mkdir(parents=True, exist_ok=True, mode=0o700)
        if not upgrade and len(self.items) >= 64:
            raise ValueError('插件数量已达到上限。')
        with ZipFile(archive) as bundle, tempfile.TemporaryDirectory(prefix='.import-', dir=self.directory) as temporary:
            entries = bundle.infolist()
            if len(entries) > 512 or sum(e.file_size for e in entries) > 16*1024*1024:
                raise ValueError('插件包过大（最多 512 个条目、16 MiB）。')
            files = {}
            for entry in entries:
                key = PurePosixPath(entry.filename)
                mode = entry.external_attr >> 16
                if key.is_absolute() or '..' in key.parts or '\\' in entry.filename or stat.S_ISLNK(mode) or stat.S_IFMT(mode) not in (0, stat.S_IFREG, stat.S_IFDIR):
                    raise ValueError('插件包包含无效路径或特殊文件。')
                if entry.is_dir():
                    continue
                if key in files:
                    raise ValueError('插件包包含重复文件。')
                files[key] = entry
            manifests = [key for key in files if key.name == 'mochi-plugin.json']
            if len(manifests) != 1:
                raise ValueError('ZIP 中需要且只能有一个 mochi-plugin.json。')
            base = manifests[0].parent
            package = Path(temporary)/'package'
            package.mkdir()
            for key, entry in files.items():
                if not key.is_relative_to(base):
                    raise ValueError('插件包包含包目录外的文件。')
                target = package/key.relative_to(base)
                target.parent.mkdir(parents=True, exist_ok=True)
                with bundle.open(entry) as source, target.open('wb') as output:
                    shutil.copyfileobj(source, output)
            spec = read_manifest(package)
            key = spec['id']
            destination = self.directory/key
            if key in self.items and self.items[key]['builtin']:
                raise ValueError('Bundled plugins cannot be replaced.')
            record = self.metadata(spec)
            record['root'] = str(destination)
            self.resources.replace_package('plugins', key, package, destination, record, upgrade)
        self.refresh()
        return key


class PluginApprovalRequired(ValueError):
    def __init__(self, key):
        self.key = key
        super().__init__('Review and approve this plugin before enabling it.')
