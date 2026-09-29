#!/usr/bin/env python3
"""Build a new history-free source tree; never export the private Git database."""
import argparse
import gzip
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile

ROOT = Path(__file__).resolve().parents[1]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def build(destination, archive):
    policy = json.loads((ROOT/'packaging/public-source.json').read_text())
    if destination.exists() and any(destination.iterdir()):
        raise ValueError('Public source destination must be empty.')
    destination.mkdir(parents=True,exist_ok=True)
    selected=set(policy['files']+policy['extra_files'])
    for base,suffixes in policy['trees'].items():
        for path in (ROOT/base).rglob('*'):
            relative=path.relative_to(ROOT)
            if (path.is_file() and path.suffix in suffixes and
                not set(relative.parts)&set(policy['excluded_parts']) and
                not any(str(relative).startswith(prefix) for prefix in policy['excluded_prefixes'])):
                selected.add(str(relative))
    from bundle_inventory import validate
    for item in validate(ROOT/'plugins/mochi-chatgpt')['packages']:
        selected.update('plugins/mochi-chatgpt/'+license['path'] for license in item['license_files'])
    for name in sorted(selected):
        source=ROOT/name
        if source.is_symlink() or not source.is_file():
            raise ValueError('Invalid allow-list entry: '+name)
        target=destination/name;target.parent.mkdir(parents=True,exist_ok=True)
        shutil.copyfile(source,target)
    # An explicit unavailable optional backend preserves Cargo's path resolution
    # without distributing the private evaluator or vendor runtime.
    stub=destination/'crates/mochi-spine';(stub/'src').mkdir(parents=True)
    (stub/'Cargo.toml').write_text('[package]\nname="mochi-spine"\nversion.workspace=true\nedition.workspace=true\nlicense.workspace=true\n[dependencies]\n[features]\ndefault=[]\nruntime=[]\nspine=["runtime"]\n[lints]\nworkspace=true\n')
    (stub/'src/lib.rs').write_text('#[cfg(feature = "runtime")]\ncompile_error!("Spine is unavailable in Public Source; use the audited private full source with separate runtime rights.");\n')
    manifest=destination/'Cargo.toml'
    text=manifest.read_text().replace('members = ["crates/*", "examples/*", "tests"]',
        'members = ["crates/*", "examples/desktop", "tests"]\ndefault-members = ["examples/desktop"]')
    manifest.write_text(text)
    for name in ('crates/mochi-package/Cargo.toml', 'crates/mochi-renderer-software/Cargo.toml', 'examples/desktop/Cargo.toml'):
        path = destination/name
        path.write_text(path.read_text().replace('mochi-spine = { workspace = true, optional = true }',
                                               'mochi-spine = { workspace = true, optional = true, features = ["runtime"] }'))
    path = destination/'tests/Cargo.toml'
    path.write_text(path.read_text().replace('mochi-spine.workspace = true\n', '').replace('features = ["spine"]', 'features = []').replace('["gpu", "spine"]', '["gpu"]'))
    path = destination/'tests/architecture.rs'
    path.write_text(path.read_text().replace('    pub mod spine;\n', ''))
    (destination/'PUBLIC_SOURCE_DISTRIBUTION.json').write_text(json.dumps({'format': 1, 'distribution': 'public-source', 'default_theme': 'welcome', 'spine': False}, indent=2)+'\n')
    workflow = destination/'.github/workflows/ci.yml'
    workflow.parent.mkdir(parents=True)
    shutil.copyfile(ROOT/'packaging/public-ci.yml', workflow)
    (destination/'.gitignore').write_text('/target/\n**/node_modules/\n**/__pycache__/\n*.pyc\n')
    # Replace the private brand reference with the original release vector.
    icon = destination/'assets/brand/mochi-icon.svg'
    icon.parent.mkdir(parents=True)
    shutil.copyfile(ROOT/'packaging/icons/io.mochi.Manager.svg', icon)
    generated_icons = set()
    for manifest_path in (destination/'plugins').glob('*/mochi-plugin.json'):
        metadata = json.loads(manifest_path.read_text())
        if metadata.get('icon'):
            path = manifest_path.parent/metadata['icon']
            if not path.resolve().is_relative_to(manifest_path.parent.resolve()):
                raise ValueError('Plugin icon path escapes package')
            path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT/'packaging/icons/io.mochi.Manager.svg', path)
            generated_icons.add(str(path.relative_to(destination)))
    subprocess.run(['cargo', 'fmt', '--all'], cwd=destination, check=True)

    (destination/'NOTICE').write_text('Mochi original source: Apache-2.0. This artifact contains no Spine vendor runtime, imported artwork or private Git objects.\nDependencies retain their upstream licenses. See package SBOM and dependency-licenses.\n')
    (destination/'README.md').write_text('# Mochi Public Source candidate\n\nThis audited staging tree has no private history or imported artwork. It is not a public Release.\n\nBuild the default Sprite renderer with `cargo build --release -p mochi-desktop`.\nPackage with `/usr/bin/python3 scripts/build-package.py target/stage --archive target/mochi.tar.gz --deb`.\nThe installed demo uses original Mochi Dot and Default Bubble.\nSee docs/PUBLIC_SOURCE.md for auditing and licensing boundaries.\n')
    subprocess.run(['cargo','metadata','--offline','--format-version','1'],cwd=destination,check=True,stdout=subprocess.DEVNULL)
    # Original image generated from source drawing primitives, not copied art.
    spec=importlib.util.spec_from_file_location('mochi_build',ROOT/'scripts/build-package.py')
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
    module.create_demo(destination/'characters/mochi-dot.mochi')
    epoch=int(os.environ.get('SOURCE_DATE_EPOCH') or subprocess.check_output(['git','log','-1','--format=%ct'],cwd=ROOT,text=True))
    provenance={'format':1,'source':'private-monorepo-allow-list','source_commit':subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),
                'source_date_epoch':epoch,'git_objects':[], 'policy_sha256':digest(ROOT/'packaging/public-source.json')}
    (destination/'SOURCE_PROVENANCE.json').write_text(json.dumps(provenance,sort_keys=True,indent=2)+'\n')
    audit(destination, selected, generated_icons)
    files = {str(p.relative_to(destination)): digest(p) for p in sorted(destination.rglob('*')) if p.is_file()}
    (destination/'PUBLIC_SOURCE_FILES.json').write_text(json.dumps({'format': 1, 'files': files}, sort_keys=True, indent=2)+'\n')
    archive.parent.mkdir(parents=True,exist_ok=True)
    with archive.open('wb') as raw,gzip.GzipFile(filename='',mode='wb',fileobj=raw,mtime=epoch) as zipped,tarfile.open(fileobj=zipped,mode='w') as tar:
        for path in sorted(destination.rglob('*')):
            info=tar.gettarinfo(str(path),str(path.relative_to(destination)))
            info.uid=info.gid=0;info.uname=info.gname='root';info.mtime=epoch
            if path.is_file():
                with path.open('rb') as stream:tar.addfile(info,stream)
            else:tar.addfile(info)
    print(json.dumps({'files':len(list(destination.rglob('*'))),'archive':str(archive),'sha256':digest(archive)}))


def audit(destination, selected, generated_fixtures=()):
    # Known forbidden-byte inventory is calculated from private material, never
    # included in the source artifact. Renaming a prohibited file cannot pass.
    forbidden=set()
    for base in ('characters','assets/brand','assets/manager/portraits','assets/bubbles/lime-notes','assets/bubbles/lime-chat'):
        for path in (ROOT/base).rglob('*'):
            if path.is_file() and not path.is_symlink() and path.suffix.lower() in ('.png','.jpg','.jpeg','.webp','.svg','.mp4','.zip','.html','.woff','.woff2','.ttf','.wasm'):
                forbidden.add(digest(path))
    for path in ROOT.iterdir():
        if path.is_file() and path.suffix.lower() in ('.zip','.html','.mp4','.png'):
            forbidden.add(digest(path))
    generated=set(generated_fixtures) | {'PUBLIC_SOURCE_FILES.json', 'PUBLIC_SOURCE_DISTRIBUTION.json', '.github/workflows/ci.yml', '.gitignore', 'assets/brand/mochi-icon.svg', 'Cargo.toml','NOTICE','README.md','SOURCE_PROVENANCE.json','crates/mochi-spine/Cargo.toml','crates/mochi-spine/src/lib.rs',
               'characters/mochi-dot.mochi/manifest.toml','characters/mochi-dot.mochi/behavior.toml','characters/mochi-dot.mochi/sprites/dot.png'}
    for path in destination.rglob('*'):
        relative=str(path.relative_to(destination))
        if path.is_symlink() or '.git' in path.parts or 'objects' in path.parts or 'portraits' in path.parts:
            raise ValueError('Forbidden public path: '+relative)
        if not path.is_file():continue
        if relative not in selected|generated or path.stat().st_size>2*1024*1024:
            raise ValueError('Unlisted file or large media: '+relative)
        reviewed = json.loads((ROOT/'packaging/public-source.json').read_text()).get('reviewed_test_fixtures', {})
        if relative in reviewed and digest(path) != reviewed[relative]:
            raise ValueError('Reviewed decoder fixture checksum changed: '+relative)
        if digest(path) in forbidden and relative not in reviewed:
            raise ValueError('Forbidden private content hash: '+relative)
        if path.suffix.lower() in ('.png','.jpg','.webp','.mp4','.zip','.wasm') and relative not in generated and relative not in reviewed:
            raise ValueError('Forbidden media: '+relative)


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('destination',type=Path);parser.add_argument('--archive',type=Path,required=True)
    args=parser.parse_args();build(args.destination.resolve(),args.archive.resolve())
