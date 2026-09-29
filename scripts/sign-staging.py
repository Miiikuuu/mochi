#!/usr/bin/env python3
"""Sign SHA-256 manifest with an explicitly supplied Ed25519 PEM key."""
import argparse
import hashlib
from pathlib import Path
import subprocess

parser=argparse.ArgumentParser()
parser.add_argument('--key',type=Path,required=True)
parser.add_argument('--output',type=Path,required=True)
parser.add_argument('files',nargs='+',type=Path)
args=parser.parse_args()
args.output.mkdir(parents=True,exist_ok=True)
manifest=args.output/'SHA256SUMS'
manifest.write_text(''.join(hashlib.sha256(path.read_bytes()).hexdigest()+'  '+path.name+'\n' for path in sorted(args.files)))
public=args.output/'staging-signing-public.pem'
signature=args.output/'SHA256SUMS.sig'
subprocess.run(['openssl','pkey','-in',str(args.key),'-pubout','-out',str(public)],check=True)
subprocess.run(['openssl','pkeyutl','-sign','-rawin','-inkey',str(args.key),'-in',str(manifest),'-out',str(signature)],check=True)
subprocess.run(['openssl','pkeyutl','-verify','-rawin','-pubin','-inkey',str(public),'-in',str(manifest),'-sigfile',str(signature)],check=True)
