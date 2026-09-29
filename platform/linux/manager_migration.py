"""Explicit adoption of old checkout resources and exact generated launchers."""
from pathlib import Path
import os
import tempfile
import shutil
from resource_catalog import package_sha256


def adopt(store, kind, key):
    """Copy only on explicit user action; Theme bytes and source stay intact."""
    with store.resources.leases.mutation(), store.resources.locked():
        store.resources.leases.assert_unused(kind, key)
        record = store.resources.read()[kind].get(key)
        if not record or not record.get('legacy_bundled'):
            raise ValueError('No legacy bundled resource is available for adoption.')
        source = Path(record.get('root') or record.get('path'))
        digest = package_sha256(source)
        root = store.resources.data_root/kind
        root.mkdir(parents=True, exist_ok=True)
        destination = root/(key+'.mochi' if kind == 'characters' else key)
        if destination.exists():
            raise ValueError('A local resource already occupies the destination.')
        with tempfile.TemporaryDirectory(prefix='.adopt-', dir=root) as temporary:
            package = Path(temporary)/'package'
            shutil.copytree(source, package, symlinks=True)
            if package_sha256(package) != digest:
                raise ValueError('Source changed during adoption; nothing was installed.')
            if kind == 'characters':
                metadata = store.character(package)
                preview = metadata.get('preview')
                metadata.update(path=str(destination), preview=str(destination/Path(preview).relative_to(package)) if preview else None)
            elif kind == 'plugins':
                from plugin_catalog import read_manifest, PluginCatalog
                spec = read_manifest(package, builtin=False)
                if spec['id'] != key:
                    raise ValueError('Plugin identity changed.')
                metadata = PluginCatalog.metadata(spec)
                metadata.update(root=str(destination), review='unreviewed', approval=None)
            elif kind == 'bubbles':
                # Validate all bounded manifest/CSS reads using the regular importer.
                import json
                from resource_catalog import read_bounded
                metadata = json.loads(read_bounded(package/'theme.json', 65536))
                if metadata.get('id') != key or metadata.get('format') != 'mochi-bubble-v1':
                    raise ValueError('Bubble identity changed.')
                for item in package.rglob('*'):
                    if item.is_file():
                        read_bounded(item, 256*1024 if item.suffix in ('.css', '.json') else 8*1024*1024)
                if not all((package/name).is_file() for name in ('layout.json', 'speech.css')):
                    raise ValueError('Incomplete Bubble package.')
                metadata.update(root=str(destination))
            else:
                raise ValueError('Unknown resource kind.')
            metadata.update(origin='local', lifecycle='active', package_sha256=digest, legacy_bundled=False)
            from resource_transactions import commit
            current = store.resources.read()
            current[kind][key] = metadata
            commit(store.resources, kind, key, destination, current, package=package, operation='install')
    return key


def migrate_launchers(checkouts):
    """Match entire historical generated templates; retain all customized files."""
    from manager_launcher import desktop_exec
    data = Path(os.environ.get('XDG_DATA_HOME', str(Path.home()/'.local/share')))
    config = Path(os.environ.get('XDG_CONFIG_HOME', str(Path.home()/'.config')))
    changed = []
    for root in checkouts:
        def quote(value):
            return '"'+str(value).replace('\\', '\\\\').replace('"', '\\"').replace('`', '\\`').replace('$', '\\$').replace('%', '%%')+'"'
        command = quote('/usr/bin/python3')+' '+quote(Path(root)/'platform/linux/manager_launcher.py')
        desktop = ('[Desktop Entry]\nType=Application\nName=Mochi\nComment=管理 Theme、角色与素材\nExec='+command+
                   '\nIcon='+str(Path(root)/'assets/brand/mochi-icon.svg')+'\nTerminal=false\nCategories=Utility;\nStartupWMClass=io.mochi.Manager\n')
        autostart_prefix = '[Desktop Entry]\nType=Application\nName=Mochi\nExec='
        templates = [(data/'applications/io.mochi.Manager.desktop', desktop, command),
                     (config/'autostart/mochi-manager.desktop', autostart_prefix+command+'\nTerminal=false\n', command)]
        # Exact templates from 16f4ff4, d79e11f and cdc8ee0. No fuzzy matching.
        old_command = '/usr/bin/python3 '+quote(Path(root)/'platform/linux/manager.py').replace('%%', '%')
        for icon in ('assets/brand/mochi-icon.svg', 'assets/manager/mochi.svg'):
            old = ('[Desktop Entry]\nType=Application\nName=Mochi\nComment=管理 Theme、角色与素材\nExec='+old_command+
                   '\nIcon='+str(Path(root)/icon)+'\nTerminal=false\nCategories=Utility;\nStartupWMClass=MochiManager\nStartupNotify=true\n')
            templates.append((data/'applications/io.mochi.Manager.desktop',old,old_command))
        v2_command = '/usr/bin/python3 '+quote(Path(root)/'platform/linux/manager_v2.py')
        templates.append((config/'autostart/mochi-manager.desktop',autostart_prefix+v2_command+'\nTerminal=false\n',v2_command))
        for path, expected, previous_command in templates:
            if path.is_symlink() or not path.is_file() or path.read_bytes() != expected.encode():
                continue
            if path.parent.name == 'autostart':
                replacement = expected.replace(previous_command, desktop_exec())
                temporary = path.with_suffix('.migration.tmp')
                temporary.write_text(replacement)
                os.replace(temporary, path)
            else:
                path.unlink()
            changed.append(str(path))
    return changed


def discover_checkouts(resources):
    import re
    roots = set()
    for kind in ('characters', 'bubbles', 'plugins'):
        for record in resources.snapshot[kind].values():
            if record.get('origin') != 'bundled':
                continue
            value = str(record.get('root') or record.get('path') or '')
            for marker in ('/characters/', '/assets/bubbles/', '/plugins/'):
                if marker in value:
                    roots.add(value.split(marker)[0])
    data = Path(os.environ.get('XDG_DATA_HOME', str(Path.home()/'.local/share')))
    config = Path(os.environ.get('XDG_CONFIG_HOME', str(Path.home()/'.config')))
    for path in (data/'applications/io.mochi.Manager.desktop', config/'autostart/mochi-manager.desktop'):
        if path.is_file() and path.stat().st_size < 8192:
            match = re.search(r'"(/[^"\n]+)/platform/linux/manager(?:_launcher|_v2)?\.py"', path.read_text())
            if match:
                roots.add(match[1])
    from mochi_paths import DATA, LIBEXEC
    return sorted(roots - {str(DATA), str(LIBEXEC)})


def discover_legacy_records(resources, checkouts, legacy_characters):
    """Recover references from an old generated entry when no catalog survived."""
    import json
    from resource_catalog import read_bounded
    path = resources.directory/'themes.json'
    if not path.is_file():
        return
    raw = json.loads(read_bounded(path, 4*1024*1024))
    themes = raw.get('themes', [])
    if not isinstance(themes, list):
        return
    with resources.locked():
        current = resources.read()
        changed = False
        for root in map(Path, checkouts):
            for theme in themes:
                if not isinstance(theme, dict):
                    continue
                for kind, keys in (('characters', theme.get('characters', [])), ('bubbles', [theme.get('bubble')]), ('plugins', theme.get('plugins', []))):
                    for key in keys:
                        if not isinstance(key, str) or key in current[kind]:
                            continue
                        if kind == 'characters' and key in legacy_characters:
                            name, relative, preview = legacy_characters[key]
                            package = root/relative
                            metadata = {'name':name, 'path':str(package), 'preview':str(root/preview)}
                        elif kind in ('bubbles','plugins') and '/' not in key and '..' not in key:
                            package = root/('assets/bubbles' if kind == 'bubbles' else 'plugins')/key
                            metadata = {'name':key, 'root':str(package)}
                        else:
                            continue
                        manifest = {'characters':'manifest.toml','bubbles':'theme.json','plugins':'mochi-plugin.json'}[kind]
                        if (package/manifest).is_file():
                            current[kind][key] = {**metadata, 'origin':'bundled', 'lifecycle':'missing', 'legacy_bundled':True}
                            changed = True
        if changed:
            resources._commit(current)
