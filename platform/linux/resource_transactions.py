"""Durable package/catalog/Theme transactions with validated crash recovery."""
from copy import deepcopy
import json
import os
from pathlib import Path
import re
import shutil
import uuid


def sync_directory(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def sync_tree(path):
    for item in path.rglob('*'):
        if item.is_symlink():
            raise ValueError('Package links are not supported.')
        if item.is_file():
            with item.open('rb') as stream:
                os.fsync(stream.fileno())
    for directory in sorted((item for item in path.rglob('*') if item.is_dir()), key=lambda p: len(p.parts), reverse=True):
        sync_directory(directory)
    sync_directory(path)


def durable_unlink(path):
    path.unlink(missing_ok=True)
    sync_directory(path.parent)


def checkpoint(stage):
    """Injectable fault boundary; production never reads crash-test environment."""


def validate_paths(resources, record):
    kind, key = record.get('kind'), record.get('key')
    if kind not in resources.roots or not isinstance(key, str) or not re.fullmatch(r'[a-zA-Z0-9][a-zA-Z0-9._-]{0,127}', key):
        raise ValueError('Invalid resource transaction identity.')
    destination = Path(record['destination'])
    roots = resources.allowed_roots(kind)
    root = destination.parent
    expected_name = key+'.mochi' if kind == 'characters' else key
    if root not in [p.resolve() for p in roots if not p.is_symlink()] or destination.name != expected_name or destination.is_symlink():
        raise ValueError('Transaction destination is outside the allowed resource root.')
    for field, prefix in (('backup', '.removed-'), ('stage', '.replace-')):
        value = record.get(field)
        if value is not None:
            path = Path(value)
            if path.parent != root or not path.name.startswith(prefix) or path.is_symlink():
                raise ValueError('Transaction backup/stage is outside the allowed resource root.')
    return destination


def cleanup_orphans(resources):
    for root in {root for kind in resources.roots for root in resources.allowed_roots(kind)}:
        if not root.exists() or root.is_symlink():
            continue
        for pattern in ('.replace-*', '.removed-*'):
            for path in root.glob(pattern):
                if path.is_symlink():
                    path.unlink()
                elif path.is_dir():
                    shutil.rmtree(path)
                else:
                    path.unlink()
            sync_directory(root)


def recover(resources):
    from resource_catalog import atomic_write, read_bounded
    journal = resources.directory/'resource-transaction.json'
    if not journal.exists():
        cleanup_orphans(resources)
        return
    record = json.loads(read_bounded(journal, 8*1024*1024))
    # Upgrade legacy removal journals only after deriving identity from the
    # previous catalog, never by trusting arbitrary destination paths.
    if 'kind' not in record:
        matches = [(kind, key) for kind in resources.roots for key, item in record['catalog'][kind].items()
                   if item.get('root', item.get('path')) == record.get('destination')]
        if len(matches) != 1:
            raise ValueError('Cannot identify legacy transaction.')
        record.update(kind=matches[0][0], key=matches[0][1], operation='uninstall', existed=True)
    destination = validate_paths(resources, record)
    backup = Path(record['backup']) if record.get('backup') else None
    if backup and backup.exists():
        if destination.exists():
            shutil.rmtree(destination)
        backup.rename(destination)
        sync_directory(destination.parent)
    elif not record.get('existed', True) and destination.exists():
        shutil.rmtree(destination)
        sync_directory(destination.parent)
    atomic_write(resources.path, record['catalog'])
    if record.get('themes') is not None:
        atomic_write(resources.directory/'themes.json', record['themes'])
    durable_unlink(journal)
    cleanup_orphans(resources)


def commit(resources, kind, key, destination, catalog, *, package=None, themes=None, operation='uninstall'):
    """Caller holds Theme/catalog locks and (except import) the mutation gate."""
    from resource_catalog import atomic_write, read_bounded
    destination = Path(destination).absolute()
    root = destination.parent
    root.mkdir(parents=True, exist_ok=True)
    identity = uuid.uuid4().hex
    backup = root/('.removed-'+identity)
    stage = root/('.replace-'+identity) if package is not None else None
    from resource_tasks import report
    report('committing')
    old_catalog = resources.read()
    theme_path = resources.directory/'themes.json'
    old_themes = json.loads(read_bounded(theme_path, 4*1024*1024)) if theme_path.exists() else None
    record = {'version': 2, 'operation': operation, 'kind': kind, 'key': key,
              'destination': str(destination), 'backup': str(backup),
              'stage': str(stage) if stage else None, 'existed': destination.exists(),
              'catalog': old_catalog, 'themes': old_themes}
    validate_paths(resources, record)
    if stage:
        shutil.copytree(package, stage)
        sync_tree(stage)
        sync_directory(root)
    checkpoint('staged')
    journal = resources.directory/'resource-transaction.json'
    atomic_write(journal, record)
    checkpoint('journal')
    try:
        if operation != 'repair':
            if destination.exists():
                destination.rename(backup)
                sync_directory(root)
            checkpoint('backup')
            if stage:
                stage.rename(destination)
                sync_directory(root)
            checkpoint('package')
        resources._commit(deepcopy(catalog))
        checkpoint('catalog')
        if themes is not None and themes != old_themes:
            atomic_write(theme_path, themes)
        checkpoint('themes')
        durable_unlink(journal)
        checkpoint('committed')
    except BaseException:
        if journal.exists():
            recover(resources)
            resources.snapshot = resources.read()
        raise
    cleanup_orphans(resources)
    checkpoint('cleaned')
