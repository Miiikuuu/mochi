#!/usr/bin/env python3
"""Reproducible staging package. Explicit asset allow-list, never a checkout copy."""
import argparse
import gzip
from email.utils import formatdate
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]
VERSION = tomllib.loads((ROOT/'Cargo.toml').read_text())['workspace']['package']['version']


def build(output, archive, deb=False, binary=None, spine=False):
    if output.exists() and any(output.iterdir()):
        raise SystemExit('Output must be an empty staging directory.')
    output.mkdir(parents=True, exist_ok=True)
    provenance = ROOT/'SOURCE_PROVENANCE.json'
    epoch=int(os.environ.get('SOURCE_DATE_EPOCH') or (json.loads(provenance.read_text())['source_date_epoch'] if provenance.exists() else subprocess.check_output(['git','log','-1','--format=%ct'],cwd=ROOT,text=True).strip()))
    prefix = output/'usr'
    code = prefix/'libexec/mochi'
    data = prefix/'share/mochi'
    def copy(source, target):
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
        target.chmod(0o755 if source.suffix == '.py' and source.read_bytes().startswith(b'#!') else 0o644)
    for source in sorted((ROOT/'platform/linux').glob('*.py')):
        copy(source, code/'platform/linux'/source.name)
    copy(ROOT/'scripts/purge-user-data.py', code/'platform/linux/purge_user_data.py')
    (code/'installation.json').write_text(json.dumps({'layout':1, 'version':VERSION, 'channel':'private-full' if spine else 'staging-no-spine', 'spine':spine})+'\n')
    binary = binary or ROOT/'target/release/mochi-desktop'
    if not binary.is_file(): raise SystemExit('Build cargo --release -p mochi-desktop first.')
    info = json.loads(subprocess.check_output([str(binary.resolve()),'--build-info'],text=True))
    if info != {'version':VERSION, 'spine':spine}:
        raise SystemExit('Binary version/features do not match requested package lane.')
    copy(binary, code/'bin/mochi-desktop');(code/'bin/mochi-desktop').chmod(0o755)
    subprocess.run(['strip','--strip-unneeded',str(code/'bin/mochi-desktop')],check=True)
    # The source archive's imported characters, brand image and DIY bubbles are
    # deliberately absent. Only code-authored default vectors/CSS are allowed.
    for directory in ('assets/manager', 'assets/icons', 'assets/bubbles/default'):
        for source in sorted((ROOT/directory).rglob('*')):
            if source.is_file() and 'portraits' not in source.parts and source.suffix in ('.svg', '.css', '.json', '.md'):
                copy(source, data/source.relative_to(ROOT))
    for target in (data/'assets/brand/mochi-icon.svg', prefix/'share/icons/hicolor/scalable/apps/io.mochi.Manager.svg'):
        copy(ROOT/'packaging/icons/io.mochi.Manager.svg', target)
    for source in sorted((ROOT/'locale').rglob('*.mo')):
        copy(source, prefix/'share/locale'/source.relative_to(ROOT/'locale'))
    for source in sorted((ROOT/'plugins').glob('*/mochi-plugin.json')):
        package = source.parent
        destination = data/'plugins'/package.name
        for item in sorted(package.rglob('*')):
            relative = item.relative_to(package)
            if any(part in ('node_modules', 'test', 'scripts', '.local', '__pycache__') for part in relative.parts): continue
            if item.is_file() and item.suffix in ('.py','.json','.mjs','.cjs','.html','.css','.js','.md'):
                copy(item, destination/relative)
        manifest=json.loads(source.read_text())
        if manifest.get('icon'):
            copy(ROOT/'packaging/icons/io.mochi.Manager.svg', destination/manifest['icon'])
        if (destination/'package-lock.json').exists():
            subprocess.run(['npm','ci','--ignore-scripts','--omit=dev','--no-audit','--no-fund','--prefix',str(destination)], check=True)
            from npm_inventory import prune_development_workspaces
            prune_development_workspaces(destination)
            # SDK examples are development programs, not bridge runtime modules.
            for examples in (destination/'node_modules/@modelcontextprotocol/sdk/dist').glob('*/examples'):
                shutil.rmtree(examples)
            for item in (destination/'node_modules').rglob('*'):
                if item.is_file() and (item.name.startswith('.eslintrc') or item.name in ('.npmignore', '.gitattributes', '.gitignore')):
                    item.unlink()
    # Relative wrapper remains valid after staging relocation; no source or
    # developer-specific Node/Codex executable path is embedded.
    native=data/'plugins/mochi-translate/.local/native-host';native.parent.mkdir(parents=True)
    native.write_text('#!/bin/sh\nexec node "$(dirname "$0")/../native.mjs" "$@"\n');native.chmod(0o755)
    create_demo(data/'characters/mochi-dot.mochi')
    for name, entry, args in [('mochi-manager','manager_launcher.py',''),
                              ('mochi-manager-legacy','manager_launcher.py',' --legacy'),
                              ('mochi-sessiond','sessiond.py',''),
                              ('mochi-purge-data','purge_user_data.py','')]:
        path=prefix/'bin'/name;path.parent.mkdir(parents=True,exist_ok=True)
        path.write_text('#!/bin/sh\nmochi_prefix=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)\nexec /usr/bin/python3 -B "$mochi_prefix/libexec/mochi/platform/linux/'+entry+'"'+args+' "$@"\n');path.chmod(0o755)
    for source in (ROOT/'packaging/systemd').glob('*'):
        copy(source,prefix/'lib/systemd/user'/source.name)
    wants=prefix/'lib/systemd/user/sockets.target.wants';wants.mkdir()
    (wants/'mochi-sessiond.socket').symlink_to('../mochi-sessiond.socket')
    copy(ROOT/'packaging/mochi.desktop',prefix/'share/applications/io.mochi.Manager.desktop')
    copy(ROOT/'packaging/metainfo/io.mochi.Manager.metainfo.xml',prefix/'share/metainfo/io.mochi.Manager.metainfo.xml')
    metadata = prefix/'share/metainfo/io.mochi.Manager.metainfo.xml'
    metadata.write_text(metadata.read_text().replace('@VERSION@', VERSION))
    for name in ('LICENSE','NOTICE','ASSET_LICENSES.md','SECURITY.md','CHANGELOG.md'):
        copy(ROOT/name,prefix/'share/doc/mochi'/name)
    if spine:
        copy(ROOT/'crates/mochi-spine/vendor/spine-c-3.8/LICENSE',prefix/'share/doc/mochi/Spine-Runtimes-LICENSE')
    else:
        (prefix/'share/doc/mochi/NOTICE').write_text('Mochi original code: Apache-2.0. This binary does not contain Spine Runtimes.\nThird-party dependencies retain their licenses; see sbom.spdx.json and dependency-licenses/.\n')
    from release_inventory import inventory
    inventory(ROOT, output, binary, spine)
    copy(ROOT/'packaging/copyright',prefix/'share/doc/mochi/copyright')
    for source in sorted((ROOT/'packaging/man').glob('*.1')):
        path = prefix/'share/man/man1'/(source.name+'.gz')
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open('wb') as raw, gzip.GzipFile(filename='', mode='wb', fileobj=raw, mtime=epoch) as compressed:
            compressed.write(source.read_text().replace('@VERSION@', VERSION).encode())
    changelog = ('mochi ('+VERSION+') unstable; urgency=medium\n\n  * Existing-user migration and audited no-Spine staging artifacts.\n\n -- Mochi contributors <noreply@mochi.invalid>  '+formatdate(epoch, usegmt=False)+'\n')
    with (prefix/'share/doc/mochi/changelog.gz').open('wb') as raw, gzip.GzipFile(filename='', mode='wb', fileobj=raw, mtime=epoch) as compressed:
        compressed.write(changelog.encode())
    files={str(p.relative_to(output)):hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(output.rglob('*')) if p.is_file() and not p.is_symlink()}
    (prefix/'share/doc/mochi/manifest.json').write_text(json.dumps({'format':1,'channel':'staging','files':files},sort_keys=True,indent=2)+'\n')
    normalize_tree(output, epoch)
    if archive:
        archive.parent.mkdir(parents=True,exist_ok=True)
        with archive.open('wb') as raw, gzip.GzipFile(filename='',mode='wb',fileobj=raw,mtime=epoch) as compressed, tarfile.open(fileobj=compressed,mode='w') as tar:
            for path in sorted(output.rglob('*')):
                info=tar.gettarinfo(str(path),str(path.relative_to(output)));info.uid=info.gid=0;info.uname=info.gname='root';info.mtime=epoch
                if info.isfile():
                    with path.open('rb') as stream: tar.addfile(info,stream)
                else: tar.addfile(info)
    if deb:
        control=output/'DEBIAN';control.mkdir()
        architecture=subprocess.check_output(['dpkg','--print-architecture'],text=True).strip()
        shared_dependencies = shlib_dependencies(code/'bin/mochi-desktop')
        (control/'control').write_text('Package: mochi\nVersion: '+VERSION+'\nArchitecture: '+architecture+'\nMaintainer: Mochi contributors <noreply@mochi.invalid>\nSection: utils\nPriority: optional\nDepends: python3, python3-gi, python3-pil, gir1.2-gtk-4.0, gir1.2-gtk-3.0, libx11-6, libxrender1, libxext6, libxkbcommon0, libwayland-client0, nodejs (>= 18.19), fonts-noto-cjk, '+shared_dependencies+'\nRecommends: bubblewrap\nDescription: desktop companion manager (staging)\n GTK4 Manager, Renderer and user session service.\n Dependency licenses are included in the package inventory.\n')
        for name in ('postinst','prerm'):
            copy(ROOT/'packaging'/name,control/name);(control/name).chmod(0o755)
        normalize_tree(output, epoch)
        subprocess.run(['dpkg-deb','--root-owner-group','--build',str(output),str(archive.with_suffix('').with_suffix('.deb'))],env={**os.environ,'SOURCE_DATE_EPOCH':str(epoch)},check=True)
    print(json.dumps({'staging':str(output),'archive':str(archive),'files':len(files),'channel':'staging'}))


def shlib_dependencies(binary):
    # Let the distribution derive the actual ELF symbol-version requirements.
    with tempfile.TemporaryDirectory(prefix='mochi-shlibs-') as directory:
        root = Path(directory)
        (root/'debian').mkdir()
        (root/'debian/control').write_text('Source: mochi\n\nPackage: mochi\nArchitecture: any\nDescription: Mochi manager\n')
        result = subprocess.check_output(['dpkg-shlibdeps', '-O', '-e'+str(binary.resolve())], cwd=root, text=True)
        return next(line.removeprefix('shlibs:Depends=') for line in result.splitlines() if line.startswith('shlibs:Depends='))


def normalize_tree(output, epoch):
    # npm tarballs carry arbitrary mtimes/modes; dpkg only clamps newer mtimes.
    # Normalize every entry so independent builds stay identical even when the
    # source epoch lies after extraction or cached entries have old timestamps.
    for path in [output, *output.rglob('*')]:
        if not path.is_symlink():
            executable = path.is_dir() or bool(path.stat().st_mode & 0o111)
            path.chmod(0o755 if executable else 0o644)
        os.utime(path, (epoch, epoch), follow_symlinks=False)


def create_demo(directory):
    from PIL import Image, ImageDraw
    (directory/'sprites').mkdir(parents=True)
    image=Image.new('RGBA',(64,64));draw=ImageDraw.Draw(image)
    draw.rounded_rectangle((8,8,56,58),radius=22,fill='#7473c8')
    for x in (23,41):draw.ellipse((x-3,31,x+3,37),fill='white')
    draw.arc((27,36,37,47),0,180,fill='white',width=2)
    image.save(directory/'sprites/dot.png')
    (directory/'manifest.toml').write_text('schema_version = 1\nid = "mochi.dot"\nname = "Mochi Dot"\nversion = "1.0.0"\ncapabilities = []\npermissions = []\n\n[[components]]\nid = "mochi.behavior.wander"\n[components.config]\nfile = "behavior.toml"\n\n[[components]]\nid = "mochi.appearance.sprite"\n[components.config]\nasset = "sprites/dot.png"\nwidth = 64.0\nheight = 64.0\n')
    (directory/'behavior.toml').write_text('schema_version = 1\nmin_x = 0.0\nmax_x = 100.0\nspeed = 10.0\n')


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('output',type=Path);parser.add_argument('--archive',type=Path,required=True);parser.add_argument('--deb',action='store_true');parser.add_argument('--binary',type=Path);parser.add_argument('--spine',action='store_true');args=parser.parse_args()
    build(args.output.resolve(),args.archive.resolve(),args.deb,args.binary,args.spine)
