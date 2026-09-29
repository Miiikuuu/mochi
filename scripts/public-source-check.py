#!/usr/bin/env python3
"""Initialize a history-free artifact repo, clone it, and run its complete CI."""
from pathlib import Path
import argparse
import hashlib
import json
import os
import subprocess

parser=argparse.ArgumentParser();parser.add_argument('source',type=Path);parser.add_argument('clone',type=Path);args=parser.parse_args()
source=args.source.resolve();clone=args.clone.resolve()
assert not (source/'.git').exists(), 'Artifact already has history'
marker=json.loads((source/'PUBLIC_SOURCE_DISTRIBUTION.json').read_text());assert marker['distribution']=='public-source'
expected=json.loads((source/'PUBLIC_SOURCE_FILES.json').read_text())['files']
expected['PUBLIC_SOURCE_FILES.json']=hashlib.sha256((source/'PUBLIC_SOURCE_FILES.json').read_bytes()).hexdigest()
actual={str(p.relative_to(source)):hashlib.sha256(p.read_bytes()).hexdigest() for p in source.rglob('*') if p.is_file()}
assert actual==expected, 'Source differs from audited allow-list manifest'
def run(*command,cwd=source,**kw):return subprocess.run(command,cwd=cwd,check=True,**kw)
run('git','init','--initial-branch=main')
run('git','-c','user.name=Mochi Artifact Audit','-c','user.email=artifact@mochi.invalid','add','.')
run('git','-c','user.name=Mochi Artifact Audit','-c','user.email=artifact@mochi.invalid','commit','-m','Initial audited public source artifact')
assert subprocess.check_output(['git','rev-list','--count','HEAD'],cwd=source,text=True).strip()=='1'
files=subprocess.check_output(['git','ls-tree','-r','--name-only','HEAD'],cwd=source,text=True).splitlines()
assert set(files)==set(expected), 'Initial commit differs from audited artifact file list'
run('git','clone','--no-local',str(source),str(clone))
for name,digest in expected.items():assert hashlib.sha256((clone/name).read_bytes()).hexdigest()==digest,name
run('sh','scripts/public-ci.sh',cwd=clone)
report={'history_commits':1,'artifact_files':len(expected),'clone_bytes_match':True,'complete_public_ci':True,
        'first_commit':subprocess.check_output(['git','rev-parse','HEAD'],cwd=clone,text=True).strip()}
(clone/'target/public-source-results.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(report))
