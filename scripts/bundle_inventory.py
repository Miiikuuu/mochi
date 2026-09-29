"""Validate the committed browser bundle independently of installed npm modules."""
import hashlib
import json
from pathlib import Path
import shutil

ORIGINAL_INPUTS = {'ui/connection.mjs', 'lib/relay.mjs'}


def validate(root, license_directory=None):
    root = Path(root)
    inventory = json.loads((root/'ui/bundled-js-licenses.json').read_text())
    bundle = root/inventory['bundle_output']
    if bundle.resolve() != (root/'ui/connection.bundle.html').resolve():
        raise ValueError('Unexpected bundle output')
    if hashlib.sha256(bundle.read_bytes()).hexdigest() != inventory['bundle_sha256']:
        raise ValueError('Bundle checksum differs from license inventory')
    inputs = set(json.loads((root/'ui/connection.metafile.json').read_text())['inputs'])
    covered = set(ORIGINAL_INPUTS)
    for record in inventory['packages']:
        if not all(record.get(key) for key in ('package', 'version', 'license', 'source_files', 'license_files')):
            raise ValueError('Incomplete bundle license record')
        if record['bundle_output'] != inventory['bundle_output']:
            raise ValueError('Wrong dependency bundle output')
        for source in record['source_files']:
            if source not in inputs or source in covered:
                raise ValueError('Duplicate or unlisted bundle input: '+source)
            covered.add(source)
        for item in record['license_files']:
            path = root/item['path']
            if not path.resolve().is_relative_to((root/'ui/bundle-licenses').resolve()):
                raise ValueError('License path outside inventory directory')
            if license_directory is not None:
                path = Path(license_directory)/Path(item['path']).relative_to('ui/bundle-licenses')
                if not path.resolve().is_relative_to(Path(license_directory).resolve()):
                    raise ValueError('License path outside package documentation')
            if hashlib.sha256(path.read_bytes()).hexdigest() != item['sha256']:
                raise ValueError('Bundle license checksum mismatch')
    if inputs - covered:
        raise ValueError('Non-Mochi bundle input has no license record: '+str(sorted(inputs-covered)))
    if not any(item['package'] == '@modelcontextprotocol/ext-apps' and item['version'] == '2.0.0' for item in inventory['packages']):
        raise ValueError('Expected ext-apps 2.0.0 bundle attribution missing')
    return inventory


def copy_licenses(root, destination):
    inventory = validate(root)
    destination = Path(destination)
    destination.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(Path(root)/'ui/bundled-js-licenses.json', destination/'bundled-js-licenses.json')
    for record in inventory['packages']:
        for item in record['license_files']:
            relative = Path(item['path']).relative_to('ui/bundle-licenses')
            target = destination/'dependency-licenses/bundle'/relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(Path(root)/item['path'], target)
    return inventory
