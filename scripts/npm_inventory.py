"""Inventory installed npm package roots, not arbitrary nested package.json files."""
import hashlib
import json
from pathlib import Path
import shutil


def installed_packages(plugin):
    """Walk npm's scoped/unscoped layout, including nested dependencies."""
    lock = json.loads((plugin/'package-lock.json').read_text())
    if lock.get('lockfileVersion') not in (2, 3):
        raise ValueError('Unsupported npm lockfile version')
    records = []

    def visit(modules):
        if not modules.exists():
            return
        if modules.is_symlink():
            raise ValueError('Symlinked node_modules is not a release input')
        for entry in sorted(modules.iterdir()):
            if entry.name.startswith('.'):
                continue
            if entry.is_symlink():
                raise ValueError('Symlinked npm package: '+str(entry))
            roots = sorted(entry.iterdir()) if entry.name.startswith('@') else [entry]
            for package in roots:
                if package.is_symlink() or not (package/'package.json').is_file():
                    raise ValueError('Invalid installed npm package: '+str(package))
                data = json.loads((package/'package.json').read_text())
                relative = package.relative_to(plugin).as_posix()
                locked = lock['packages'].get(relative)
                if not locked or locked.get('link') or locked.get('version') != data.get('version'):
                    raise ValueError('Installed npm package does not match lockfile: '+relative)
                if not data.get('name') or not data.get('version') or not isinstance(data.get('license'), str):
                    raise ValueError('npm dependency license requires review: '+relative)
                if locked.get('license') and locked['license'] != data['license']:
                    raise ValueError('Installed npm license differs from lockfile: '+relative)
                records.append((package, data))
                visit(package/'node_modules')

    visit(plugin/'node_modules')
    return records


def prune_development_workspaces(plugin):
    """Exclude the reviewed fast-uri benchmark, never runtime code or licenses."""
    removed = []
    for package, data in installed_packages(plugin):
        benchmark = package/'benchmark'
        if data['name'] != 'fast-uri' or not benchmark.exists():
            continue
        metadata = json.loads((benchmark/'package.json').read_text())
        if (data['version'] != '3.1.8' or data['license'] != 'BSD-3-Clause'
                or metadata.get('name') != 'benchmark' or metadata.get('version') != '1.0.0'
                or metadata.get('private') is not True or metadata.get('license') != 'ISC'):
            raise ValueError('fast-uri benchmark exclusion requires renewed review')
        if benchmark.is_symlink():
            raise ValueError('Symlinked benchmark is not a release input')
        removed.append(benchmark.relative_to(plugin).as_posix())
        shutil.rmtree(benchmark)
    return removed


def copy_licenses(plugins, doc):
    records = []
    for plugin in sorted(plugins.iterdir()):
        if not (plugin/'package-lock.json').is_file():
            if (plugin/'node_modules').exists():
                raise ValueError('Installed npm dependencies have no lockfile: '+plugin.name)
            continue
        for package, data in installed_packages(plugin):
            relative = package.relative_to(plugin)
            # Nested workspaces are not installed dependencies, but shipping
            # their separately licensed contents still requires review.
            for manifest in sorted(package.rglob('package.json')):
                subpath = manifest.relative_to(package)
                if len(subpath.parts) == 1 or 'node_modules' in subpath.parts:
                    continue
                nested = json.loads(manifest.read_text())
                if (nested.get('name') and nested.get('version')
                        and nested.get('license') != data['license']):
                    raise ValueError('Nested npm workspace license requires review: '+str(manifest))
            target = doc/'dependency-licenses/npm'/plugin.name/relative
            notices = []
            for path in sorted(package.rglob('*')):
                subpath = path.relative_to(package)
                if 'node_modules' in subpath.parts or not path.is_file():
                    continue
                if not path.name.upper().startswith(('LICENSE', 'LICENCE', 'COPYING', 'NOTICE', 'COPYRIGHT')):
                    continue
                if path.is_symlink() or not path.read_bytes().strip():
                    raise ValueError('Invalid npm license file: '+str(path))
                destination = target/subpath
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(path, destination)
                notices.append({'path': destination.relative_to(doc).as_posix(),
                                'sha256': hashlib.sha256(destination.read_bytes()).hexdigest()})
            if not any(Path(item['path']).name.upper().startswith(('LICENSE', 'LICENCE', 'COPYING')) for item in notices):
                raise ValueError('Missing npm dependency license text: '+data['name'])
            records.append({'name': data['name'], 'version': data['version'], 'license': data['license'],
                            'path': 'usr/share/mochi/plugins/'+plugin.name+'/'+relative.as_posix(),
                            'license_files': notices})
    return records


def verify_stage(stage):
    """Read-only verification of the final package's npm and bundle closure."""
    stage = Path(stage)
    doc = stage/'usr/share/doc/mochi'
    records = json.loads((doc/'npm-licenses.json').read_text())
    expected = {}
    for plugin in sorted((stage/'usr/share/mochi/plugins').iterdir()):
        if not (plugin/'package-lock.json').is_file():
            if (plugin/'node_modules').exists():
                raise ValueError('Installed npm dependencies have no lockfile: '+plugin.name)
            continue
        for package, data in installed_packages(plugin):
            expected[package.relative_to(stage).as_posix()] = data
    if len(records) != len(expected) or {r['path'] for r in records} != set(expected):
        raise ValueError('Packaged npm inventory does not cover installed package roots')
    sbom = json.loads((doc/'sbom.spdx.json').read_text())
    from release_inventory import spdx_package
    sbom_packages = {p['SPDXID']: p for p in sbom['packages']}
    for record in records:
        data = expected[record['path']]
        if any(record[key] != data[key] for key in ('name', 'version', 'license')):
            raise ValueError('Packaged npm inventory differs from installed metadata')
        package = spdx_package('npm', record['name'], record['version'], record['license'])
        if sbom_packages.get(package['SPDXID']) != package:
            raise ValueError('npm dependency missing or inconsistent in SPDX')
        if not record.get('license_files'):
            raise ValueError('Missing npm dependency license records')
        for notice in record['license_files']:
            path = doc/notice['path']
            if not path.resolve().is_relative_to((doc/'dependency-licenses/npm').resolve()):
                raise ValueError('npm notice escapes license directory')
            if hashlib.sha256(path.read_bytes()).hexdigest() != notice['sha256']:
                raise ValueError('Packaged npm notice checksum mismatch')
    from bundle_inventory import validate
    bundle = validate(stage/'usr/share/mochi/plugins/mochi-chatgpt', doc/'dependency-licenses/bundle')
    if bundle != json.loads((doc/'bundled-js-licenses.json').read_text()):
        raise ValueError('Packaged bundle inventory mismatch')
    for record in bundle['packages']:
        package = spdx_package('npm', record['package'], record['version'], record['license'])
        if sbom_packages.get(package['SPDXID']) != package:
            raise ValueError('Bundle dependency missing or inconsistent in SPDX')
        relation = {'spdxElementId': 'SPDXRef-ConnectionBundle', 'relationshipType': 'CONTAINS',
                    'relatedSpdxElement': package['SPDXID']}
        if relation not in sbom['relationships']:
            raise ValueError('Missing bundle SPDX containment relationship')
        for notice in record['license_files']:
            path = doc/'dependency-licenses/bundle'/Path(notice['path']).relative_to('ui/bundle-licenses')
            if hashlib.sha256(path.read_bytes()).hexdigest() != notice['sha256']:
                raise ValueError('Packaged bundle notice checksum mismatch')
    return {'npm_packages': len(records), 'bundle_packages': len(bundle['packages']), 'status': 'PASS'}
