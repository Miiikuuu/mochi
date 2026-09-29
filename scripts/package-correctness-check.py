#!/usr/bin/env python3
"""Fail on incorrect layout/version/desktop metadata; retain real validator logs."""
import json
from pathlib import Path
import subprocess
import sys
import tomllib
import xml.etree.ElementTree as ET

stage, deb = map(Path, sys.argv[1:])
from npm_inventory import verify_stage
print(json.dumps(verify_stage(stage)))
root = Path(__file__).resolve().parents[1]
version = tomllib.loads((root/'Cargo.toml').read_text())['workspace']['package']['version']
assert not any(path.is_file() for path in stage.iterdir()), 'Package root contains ordinary files'
assert not (stage/'manifest.json').exists()
assert (stage/'usr/share/doc/mochi/manifest.json').is_file()
assert json.loads((stage/'usr/libexec/mochi/installation.json').read_text())['version'] == version
assert subprocess.check_output(['dpkg-deb','-f',str(deb),'Version'],text=True).strip() == version
metadata = stage/'usr/share/metainfo/io.mochi.Manager.metainfo.xml'
assert ET.parse(metadata).find('releases/release').get('version') == version
assert ('## '+version+' —') in (root/'CHANGELOG.md').read_text()
sys.path.insert(0,str(root/'platform/linux'))
from session_protocol import BUILD
assert BUILD == version
commands = [
    ['lintian','--fail-on','error,warning',str(deb)],
    ['appstreamcli','validate','--no-net',str(metadata)],
    ['desktop-file-validate',str(stage/'usr/share/applications/io.mochi.Manager.desktop')],
    # Run after installation in CI: ExecStart must actually exist.
    ['systemd-analyze','--user','verify',str(stage/'usr/lib/systemd/user/mochi-sessiond.service'),str(stage/'usr/lib/systemd/user/mochi-sessiond.socket')],
]
results=[]
for command in commands:
    result=subprocess.run(command,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True)
    results.append({'command':command,'returncode':result.returncode,'output':result.stdout})
    print(result.stdout)
(stage.parent/'package-validation.json').write_text(json.dumps(results,indent=2))
assert all(result['returncode']==0 for result in results), 'Package validator failed; see package-validation.json'
