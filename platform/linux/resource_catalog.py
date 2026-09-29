"""Versioned cross-process resource index and content-bound plugin approvals."""
from contextlib import contextmanager
from copy import deepcopy
from datetime import datetime, timezone
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import tempfile


def read_bounded(path, limit):
    with Path(path).open('rb') as stream:
        data = stream.read(limit + 1)
    if len(data) > limit:
        raise ValueError(f'{Path(path).name} exceeds the {limit}-byte limit.')
    return data


def package_sha256(root, limit=64*1024*1024, package_files=None):
    """Canonical package-content digest, independent of ZIP ordering/timestamps."""
    root = Path(root)
    digest = hashlib.sha256()
    consumed = 0
    entries = sorted(root.rglob('*'))
    if any(path.is_symlink() or not (path.is_dir() or path.is_file()) for path in entries):
        raise ValueError('Package links and special files are not supported.')
    files = [path for path in entries if path.is_file()]
    if package_files is not None and set(package_files) != {path.relative_to(root).as_posix() for path in files}:
        raise ValueError('Package file set changed. Review the package again.')
    if len(files) > 512:
        raise ValueError('Too many package files.')
    for path in files:
        if not path.resolve().is_relative_to(root.resolve()) or path.is_symlink():
            raise ValueError('Package symlinks are not supported.')
        relative = path.relative_to(root).as_posix().encode()
        data = read_bounded(path, limit-consumed)
        consumed += len(data)
        digest.update(len(relative).to_bytes(4, 'big'))
        digest.update(relative)
        digest.update(len(data).to_bytes(8, 'big'))
        digest.update(data)
    return digest.hexdigest()


def atomic_write(path, data):
    with tempfile.NamedTemporaryFile(mode='w', dir=path.parent, prefix='.resource-', delete=False) as stream:
        temporary = Path(stream.name)
        try:
            json.dump(data, stream, ensure_ascii=False, indent=2)
            stream.write('\n')
            stream.flush()
            os.fsync(stream.fileno())
        except BaseException:
            temporary.unlink(missing_ok=True)
            raise
    try:
        temporary.replace(path)
        directory_fd = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)
    finally:
        temporary.unlink(missing_ok=True)


class ResourceCatalog:
    def __init__(self, directory, roots=None):
        self.directory = Path(directory)
        from runtime_resource_leases import RuntimeResourceLeases
        self.leases = RuntimeResourceLeases(self.directory)
        self.roots = {kind: self.directory/kind for kind in ('characters', 'bubbles', 'plugins')}
        self.roots.update({key: Path(value) for key, value in (roots or {}).items()})
        self.data_root = Path(os.environ.get('XDG_DATA_HOME', str(Path.home()/'.local/share')))/'mochi'
        self.path = self.directory/'resource-catalog.json'
        self.lock_path = self.directory/'resource-catalog.json.lock'
        self.directory.mkdir(parents=True, exist_ok=True, mode=0o700)
        with self.locked():
            if not self.path.exists():
                atomic_write(self.path, self.empty())
        self.snapshot = self.read()

    def allowed_roots(self, kind):
        return (self.roots[kind], self.data_root/kind)

    @staticmethod
    def empty():
        return {'version': 1, 'revision': 0, 'characters': {}, 'bubbles': {}, 'plugins': {}}

    @contextmanager
    def locked(self):
        from resource_dependencies import file_lock
        from resource_transactions import recover
        with file_lock(self.directory/'themes.json.lock'):
            with file_lock(self.lock_path) as first:
                if first:
                    recover(self)
                yield

    def read(self):
        with self.locked():
            value = json.loads(read_bounded(self.path, 4*1024*1024))
            if (value.get('version') != 1 or type(value.get('revision')) is not int or value['revision'] < 0
                    or any(not isinstance(value.get(kind), dict) for kind in ('characters', 'bubbles', 'plugins'))):
                raise ValueError('Invalid resource catalog.')
            return value

    def _commit(self, value):
        value['revision'] += 1
        atomic_write(self.path, value)
        self.snapshot = deepcopy(value)

    def reconcile(self, kind, scan):
        with self.locked():
            current = self.read()
            entries = scan(deepcopy(current[kind]))
            for key in set(entries) | set(current[kind]):
                old = current[kind].get(key, {})
                item = entries.get(key)
                if (item or old).get('origin') != 'local':
                    if item is None and old:
                        entries[key] = {**old, 'lifecycle': 'missing', 'legacy_bundled': True}
                    continue
                pending = old.get('lifecycle') == 'pending_removal' or old.get('pending_removal', False) or old.get('removal_requested', False)
                root = Path((item or old).get('root', (item or old).get('path', '')))
                manifest = {'characters': 'manifest.toml', 'bubbles': 'theme.json', 'plugins': 'mochi-plugin.json'}[kind]
                available = item is not None and (root/manifest).is_file()
                record = item if item is not None else deepcopy(old)
                record.pop('pending_removal', None)
                record['lifecycle'] = ('pending_removal' if pending else 'active') if available else 'missing'
                record['removal_requested'] = bool(pending)
                if not available and kind == 'plugins':
                    record.update(review='unreviewed', approval=None)
                entries[key] = record
            if entries != current[kind]:
                current[kind] = entries
                self._commit(current)
            else:
                self.snapshot = current
        return deepcopy(entries)

    def replace_package(self, kind, key, package, destination, metadata, upgrade=False):
        from contextlib import nullcontext
        from resource_transactions import commit
        with self.leases.mutation() if upgrade else nullcontext():
            if upgrade:
                self.leases.assert_unused(kind, key)
            return self._replace_package_locked(kind, key, package, destination, metadata, upgrade)

    def _replace_package_locked(self, kind, key, package, destination, metadata, upgrade=False):
        """Internal: upgrades already hold mutation gate and verified leases."""
        from resource_transactions import commit
        with self.locked():
            current = self.read()
            exists = Path(destination).exists()
            if exists != upgrade:
                raise ValueError('Package already exists.' if exists else 'Package is not installed.')
            previous = current[kind].get(key)
            if previous and previous.get('origin') == 'bundled':
                raise ValueError('Bundled resources cannot be upgraded here.')
            if previous and previous.get('lifecycle') == 'pending_removal':
                raise ValueError('Cancel removal before upgrading this resource.')
            current[kind][key] = {**metadata, 'lifecycle': 'active', 'removal_requested': False}
            commit(self, kind, key, destination, current, package=package,
                   operation='upgrade' if upgrade else 'install')
        return key

    def uninstall(self, kind, key, destination, **repair):
        from resource_dependencies import uninstall
        return uninstall(self, kind, key, destination, **repair)

    def require_selectable(self, kind, key):
        record = self.read()[kind].get(key)
        if not record or record.get('lifecycle', 'active') != 'active':
            raise ValueError('Resource is missing or pending removal. Cancel removal before selecting it.')

    def cancel_removal(self, kind, key):
        with self.leases.mutation(), self.locked():
            current = self.read()
            entry = current[kind].get(key)
            if not entry or not (entry.get('lifecycle') == 'pending_removal' or entry.get('removal_requested')):
                raise ValueError('Resource is not pending removal.')
            entry.update(lifecycle='missing' if entry.get('lifecycle') == 'missing' else 'active', removal_requested=False)
            entry.pop('pending_removal', None)
            self._commit(current)
            return {'kind': kind, 'key': key, 'lifecycle': entry['lifecycle']}

    def finalize_removal(self, kind, key):
        from resource_dependencies import _uninstall_locked
        with self.leases.mutation():
            self.leases.assert_unused(kind, key)
            with self.locked():
                entry = self.read()[kind].get(key)
                if not entry or entry.get('lifecycle') != 'pending_removal':
                    raise ValueError('Resource is not pending removal.')
                return _uninstall_locked(self, kind, key, entry.get('root', entry.get('path')),
                    replacement=None, disable=False, expected_revision=None, repair_only=False)

    def approve_plugin(self, key, expected_sha256):
        with self.locked():
            current = self.read()
            entry = current['plugins'].get(key)
            if (not entry or entry['package_sha256'] != expected_sha256 or
                    package_sha256(entry['root'], 16*1024*1024, entry.get('package_files')) != expected_sha256):
                raise ValueError('The plugin changed. Review the current package before approving it.')
            entry['review'] = 'approved'
            entry['approval'] = {'package_sha256': expected_sha256,
                                 'approved_at': datetime.now(timezone.utc).isoformat(),
                                 'uid': os.getuid()}
            self._commit(current)
            return deepcopy(entry)
