#!/usr/bin/env python3
"""Exercise signing and user-data purge only with temporary keys and isolated XDG."""
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class SigningChecks(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.env = dict(os.environ)
        for name, directory in [('HOME', 'home'), ('XDG_CONFIG_HOME', 'config'),
                                ('XDG_DATA_HOME', 'data'), ('XDG_STATE_HOME', 'state'),
                                ('XDG_CACHE_HOME', 'cache'), ('XDG_RUNTIME_DIR', 'runtime')]:
            path = self.root / directory
            path.mkdir(mode=0o700)
            self.env[name] = str(path)
        self.config = Path(self.env['XDG_CONFIG_HOME'])
        self.key = self.config / 'mochi-release-signing/release-ed25519.pem'
        self.legacy = self.config / 'mochi/release-signing/release-ed25519.pem'
        self.public = self.root / 'pinned/public.pem'
        self.artifact = self.root / 'artifact.tar'
        self.artifact.write_bytes(b'original code')

    def sign(self, *args, success=True):
        result = subprocess.run(
            ['/usr/bin/python3', str(ROOT / 'scripts/sign-release.py'),
             '--public', str(self.public), *map(str, args)],
            env=self.env, capture_output=True, text=True)
        if success:
            self.assertEqual(result.returncode, 0, result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0)
        return result

    def identity(self, result):
        return json.loads(result.stdout.splitlines()[-1])['key_id']

    def seed_data(self):
        self.data = []
        for name, relative in [('XDG_CONFIG_HOME', 'themes.json'),
                               ('XDG_CONFIG_HOME', 'manager.json'),
                               ('XDG_DATA_HOME', 'characters/example/asset'),
                               ('XDG_DATA_HOME', 'bubbles/example/asset'),
                               ('XDG_DATA_HOME', 'plugins/example/asset'),
                               ('XDG_STATE_HOME', 'plugins/example/state')]:
            path = Path(self.env[name]) / 'mochi' / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('temporary user data')
            self.data.append(path)

    def purge(self, *args):
        result = subprocess.run(
            ['/usr/bin/python3', str(ROOT / 'scripts/purge-user-data.py'), *args],
            env=self.env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        return result

    def test_persistent_identity_and_tamper_rejection(self):
        self.sign('--init')
        before = hashlib.sha256(self.key.read_bytes()).hexdigest()
        self.assertEqual(stat.S_IMODE(self.key.stat().st_mode), 0o600)
        self.sign('--init')
        self.assertEqual(before, hashlib.sha256(self.key.read_bytes()).hexdigest())
        self.sign('--output', self.root / 'signed', self.artifact)
        manifest = self.root / 'signed/SHA256SUMS'
        verify = ['openssl', 'pkeyutl', '-verify', '-rawin', '-pubin', '-inkey',
                  str(self.public), '-in', str(manifest), '-sigfile',
                  str(self.root / 'signed/SHA256SUMS.sig')]
        subprocess.run(verify, env=self.env, check=True, capture_output=True)
        manifest.write_text(manifest.read_text() + 'tampered\n')
        self.assertNotEqual(subprocess.run(verify, env=self.env, capture_output=True).returncode, 0)

    def test_purge_removes_data_preserves_independent_identity(self):
        before = self.identity(self.sign('--init'))
        key_bytes, public_bytes = self.key.read_bytes(), self.public.read_bytes()
        self.seed_data()
        self.purge('--purge')
        self.assertTrue(all(not p.exists() for p in self.data))
        self.assertEqual(self.key.read_bytes(), key_bytes)
        self.assertEqual(self.public.read_bytes(), public_bytes)
        self.assertEqual(before, self.identity(self.sign('--output', self.root / 'after', self.artifact)))

    def test_purge_preserves_unmigrated_legacy_identity(self):
        before = self.identity(self.sign('--key', self.legacy, '--init'))
        # Older layouts may keep their public identity alongside the key.
        self.public = self.legacy.parent / 'public.pem'
        self.public.write_bytes((self.root / 'pinned/public.pem').read_bytes())
        key_bytes, public_bytes = self.legacy.read_bytes(), self.public.read_bytes()
        self.seed_data()
        self.purge('--purge')
        self.assertTrue(all(not p.exists() for p in self.data))
        self.assertEqual(self.legacy.read_bytes(), key_bytes)
        self.assertEqual(self.public.read_bytes(), public_bytes)
        result = self.sign('--init', '--output', self.root / 'after', self.artifact)
        self.assertIn('legacy', result.stderr)
        self.assertEqual(before, self.identity(result))
        self.assertFalse(self.key.exists())

    def test_explicit_migration_keeps_fingerprint_and_survives_purge(self):
        before = self.identity(self.sign('--key', self.legacy, '--init'))
        key_bytes, public_bytes = self.legacy.read_bytes(), self.public.read_bytes()
        result = self.sign('--migrate-key')
        self.assertIn('Migrated', result.stderr)
        self.assertEqual(before, self.identity(result))
        self.assertEqual(self.key.read_bytes(), key_bytes)
        self.assertEqual(stat.S_IMODE(self.key.stat().st_mode), 0o600)
        self.assertFalse(self.legacy.exists())
        self.seed_data()
        self.purge('--purge')
        self.assertTrue(all(not p.exists() for p in self.data))
        self.assertEqual(self.key.read_bytes(), key_bytes)
        self.assertEqual(self.public.read_bytes(), public_bytes)
        self.assertEqual(before, self.identity(self.sign('--output', self.root / 'after', self.artifact)))

    def test_mismatched_pinned_identity_rejects_signing_and_migration(self):
        self.sign('--key', self.legacy, '--init')
        key_bytes = self.legacy.read_bytes()
        # A different *valid* public key, not just malformed PEM.
        other = self.root / 'other.pem'
        subprocess.run(['openssl', 'genpkey', '-algorithm', 'Ed25519', '-out', str(other)],
                       env=self.env, check=True, capture_output=True)
        self.public.write_bytes(subprocess.check_output(
            ['openssl', 'pkey', '-in', str(other), '-pubout'], env=self.env))
        pinned = self.public.read_bytes()
        for args in [('--init',), ('--migrate-key',),
                     ('--output', self.root / 'rejected', self.artifact)]:
            self.assertIn('does not match', self.sign(*args, success=False).stderr)
            self.assertEqual(key_bytes, self.legacy.read_bytes())
            self.assertEqual(pinned, self.public.read_bytes())
            self.assertFalse(self.key.exists())
        self.assertFalse((self.root / 'rejected').exists())

    def test_different_existing_destination_is_never_overwritten(self):
        self.sign('--key', self.legacy, '--init')
        self.key.parent.mkdir(parents=True)
        subprocess.run(['openssl', 'genpkey', '-algorithm', 'Ed25519', '-out', str(self.key)],
                       env=self.env, check=True, capture_output=True)
        self.key.chmod(0o600)
        original = [p.read_bytes() for p in (self.legacy, self.key, self.public)]
        for args in [(), ('--init',), ('--migrate-key',)]:
            self.assertIn('identities differ', self.sign(*args, success=False).stderr)
            self.assertEqual(original, [p.read_bytes() for p in (self.legacy, self.key, self.public)])

    def test_same_existing_destination_migration_is_safe(self):
        before = self.identity(self.sign('--key', self.legacy, '--init'))
        self.key.parent.mkdir(parents=True)
        self.key.write_bytes(self.legacy.read_bytes())
        self.key.chmod(0o600)
        self.assertEqual(before, self.identity(self.sign('--migrate-key')))
        self.assertFalse(self.legacy.exists())

    def test_pinned_identity_without_private_key_is_not_rotated(self):
        self.sign('--init')
        public = self.public.read_bytes()
        self.key.unlink()
        self.assertIn('refusing to generate', self.sign('--init', success=False).stderr)
        self.assertFalse(self.key.exists())
        self.assertEqual(public, self.public.read_bytes())

    def test_purge_requires_explicit_opt_in(self):
        self.seed_data()
        self.purge()
        self.assertTrue(all(p.exists() for p in self.data))

    def test_overlapping_xdg_roots_preserve_identity(self):
        self.env['XDG_DATA_HOME'] = self.env['XDG_CONFIG_HOME']
        self.env['XDG_STATE_HOME'] = self.env['XDG_CONFIG_HOME']
        self.sign('--key', self.legacy, '--init')
        self.seed_data()
        self.purge('--purge')
        self.assertTrue(all(not p.exists() for p in self.data))
        self.sign()

    def test_default_home_fallback_is_independent(self):
        del self.env['XDG_CONFIG_HOME']
        self.sign('--init')
        self.assertTrue((Path(self.env['HOME']) / '.config/mochi-release-signing/release-ed25519.pem').is_file())
        self.assertFalse((Path(self.env['HOME']) / '.config/mochi').exists())

    def test_help_explains_default_migration_and_purge_preservation(self):
        result = self.sign('--help')
        self.assertIn('mochi-release-signing', ''.join(result.stdout.split()))
        self.assertIn('--migrate-key', result.stdout)
        self.assertIn('preserve', self.purge('--help').stdout)


if __name__ == '__main__':
    unittest.main()
