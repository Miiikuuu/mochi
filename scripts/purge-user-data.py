#!/usr/bin/env python3
"""Explicit opt-in user data removal; package uninstall never calls this."""
import argparse
import os
from pathlib import Path
import shutil
import socket

parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--purge', action='store_true', help='Permanently delete Mochi Themes, Resources, preferences and plugin state; preserve release signing identities, including the legacy signing directory')
args=parser.parse_args()
roots=[Path(os.environ.get(name) or Path.home()/default).absolute()/'mochi'
       for name,default in [('XDG_CONFIG_HOME','.config'),('XDG_DATA_HOME','.local/share'),('XDG_STATE_HOME','.local/state')]]
if not args.purge:
    print('No data removed. Use --purge explicitly after stopping Mochi.');raise SystemExit(0)
runtime=Path(os.environ.get('XDG_RUNTIME_DIR',f'/tmp/mochi-{os.getuid()}'))/'mochi'
for path in runtime.glob('*.sock') if runtime.is_dir() else ():
    with socket.socket(socket.AF_UNIX) as stream:
        stream.settimeout(.2)
        try:stream.connect(str(path))
        except OSError:continue
        raise SystemExit('Stop Mochi and its user socket/service before purging.')
for root in roots:
    if root.is_symlink() or root.exists() and root.stat().st_uid != os.getuid():
        raise SystemExit('Refusing unsafe user-data path.')
config=roots[0].parent
protected=(config/'mochi-release-signing',config/'mochi/release-signing')


def remove_data(path):
    if path in protected:
        return
    if path.is_symlink():
        path.unlink()
    elif any(identity.is_relative_to(path) for identity in protected):
        # Also safe when custom XDG roots overlap. Never descend into a
        # signing directory, including legacy public identity/backup files.
        for child in path.iterdir():remove_data(child)
        if not any(path.iterdir()):path.rmdir()
    elif path.is_dir():
        shutil.rmtree(path)
    else:
        path.unlink()


for root in roots:
    if root.exists():remove_data(root)
print('Removed this user\'s Mochi Themes, Resources, preferences and plugin state. Release signing identities were preserved.')
