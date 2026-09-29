#!/usr/bin/env python3
"""Persistent owner-controlled signing identity; private key never enters source."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile

root=Path(__file__).resolve().parents[1]
def public_key(key):
    return subprocess.check_output(['openssl','pkey','-in',str(key),'-pubout'])

def validate_key(key):
    if key.is_symlink() or not key.is_file():
        raise SystemExit('Signing key is missing or a symlink.')
    if key.stat().st_uid != os.getuid() or stat.S_IMODE(key.stat().st_mode) != 0o600:
        raise SystemExit('Signing key must be owned by this user with mode 0600.')
    return public_key(key)


def sync_directory(path):
    fd=os.open(path,os.O_RDONLY | os.O_DIRECTORY)
    try:os.fsync(fd)
    finally:os.close(fd)


config=Path(os.environ.get('XDG_CONFIG_HOME') or Path.home()/'.config')
default_key=config/'mochi-release-signing/release-ed25519.pem'
legacy_key=config/'mochi/release-signing/release-ed25519.pem'
parser=argparse.ArgumentParser(description=__doc__, epilog=(
    'Legacy keys under $XDG_CONFIG_HOME/mochi/release-signing remain usable. '
    'Use --migrate-key to move the existing private key without changing its identity. '
    'Mochi user-data purge preserves both the independent and legacy signing directories.'))
parser.add_argument('--init',action='store_true',help='Initialize only if no existing key or pinned public identity exists; never rotate an identity')
parser.add_argument('--migrate-key',action='store_true',help='Explicitly migrate the legacy default key to the independent default directory, verifying the pinned public key first')
parser.add_argument('--key',type=Path,help='Explicit private key path; default: $XDG_CONFIG_HOME/mochi-release-signing/release-ed25519.pem (~/.config when unset). Keep custom keys outside Mochi data roots')
parser.add_argument('--public',type=Path,default=root/'packaging/release-signing/public.pem',help='Pinned public identity (default: packaging/release-signing/public.pem); never overwritten')
parser.add_argument('--output',type=Path,help='Directory for the signed SHA-256 manifest')
parser.add_argument('files',nargs='*',type=Path,help='Reviewed artifacts to sign; omit to inspect the public fingerprint')
args=parser.parse_args()
if args.migrate_key and (args.key is not None or args.init):
    parser.error('--migrate-key cannot be combined with --key or --init.')
if args.key is None:
    new_exists=os.path.lexists(default_key)
    old_exists=os.path.lexists(legacy_key)
    if new_exists and old_exists:
        if validate_key(default_key)!=validate_key(legacy_key):
            raise SystemExit('Independent and legacy signing identities differ; refusing to select or overwrite either key.')
    args.key=default_key if new_exists or not old_exists else legacy_key
    if args.migrate_key:
        if not old_exists:
            raise SystemExit('No legacy signing key to migrate.')
        public=validate_key(legacy_key)
        if not args.public.is_file() or args.public.read_bytes()!=public:
            raise SystemExit('Legacy signing key does not match the pinned release public key; migration refused.')
        if not new_exists:
            default_key.parent.mkdir(parents=True,exist_ok=True,mode=0o700)
            with tempfile.TemporaryDirectory(dir=default_key.parent,prefix='.migrate-key-') as directory:
                temporary=Path(directory)/'private.pem'
                with temporary.open('xb') as stream:
                    os.fchmod(stream.fileno(),0o600)
                    stream.write(legacy_key.read_bytes())
                    stream.flush()
                    os.fsync(stream.fileno())
                if validate_key(temporary)!=public:
                    raise SystemExit('Legacy signing identity changed during migration; original retained.')
                os.link(temporary,default_key) # Atomic, exclusive: never overwrite another identity.
        sync_directory(default_key.parent)
        legacy_key.unlink() # Only after the verified destination is durable.
        sync_directory(legacy_key.parent)
        args.key=default_key
        print('Migrated the legacy signing key; public identity unchanged.',file=sys.stderr)
    elif args.key==legacy_key:
        print('Using the legacy signing key. Use --migrate-key to move it outside Mochi user data.',file=sys.stderr)
if args.init:
    args.key.parent.mkdir(parents=True,exist_ok=True,mode=0o700)
    if not os.path.lexists(args.key):
        if os.path.lexists(args.public):
            raise SystemExit('Pinned release identity exists but its private key is missing; refusing to generate a replacement.')
        with tempfile.TemporaryDirectory(dir=args.key.parent,prefix='.new-key-') as directory:
            temporary=Path(directory)/'private.pem'
            subprocess.run(['openssl','genpkey','-algorithm','Ed25519','-out',str(temporary)],check=True)
            temporary.chmod(0o600)
            os.link(temporary,args.key) # Refuse to overwrite a concurrent/existing identity.
public=validate_key(args.key)
if args.public.exists():
    if args.public.read_bytes()!=public:raise SystemExit('Signing key does not match the pinned release public key.')
elif args.init:
    args.public.parent.mkdir(parents=True,exist_ok=True)
    with args.public.open('xb') as stream:stream.write(public)
else:raise SystemExit('Pinned release public key is missing.')
der=subprocess.check_output(['openssl','pkey','-pubin','-outform','DER'],input=public)
identity='SHA256:'+hashlib.sha256(der).hexdigest()
if args.files:
    if not args.output:raise SystemExit('--output is required for signing artifacts.')
    if len({p.name for p in args.files})!=len(args.files):raise SystemExit('Artifact basenames must be unique.')
    args.output.mkdir(parents=True,exist_ok=True)
    manifest=args.output/'SHA256SUMS'
    manifest.write_text('# release-key '+identity+'\n'+''.join(hashlib.sha256(p.read_bytes()).hexdigest()+'  '+p.name+'\n' for p in sorted(args.files)))
    signature=args.output/'SHA256SUMS.sig'
    subprocess.run(['openssl','pkeyutl','-sign','-rawin','-inkey',str(args.key),'-in',str(manifest),'-out',str(signature)],check=True)
    subprocess.run(['openssl','pkeyutl','-verify','-rawin','-pubin','-inkey',str(args.public),'-in',str(manifest),'-sigfile',str(signature)],check=True)
    (args.output/'signing-identity.json').write_text(json.dumps({'algorithm':'Ed25519','key_id':identity,'identity':'persistent release key','published':False},indent=2)+'\n')
print(json.dumps({'algorithm':'Ed25519','key_id':identity,'private_key_exported':False}))
