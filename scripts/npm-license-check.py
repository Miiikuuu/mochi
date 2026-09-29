#!/usr/bin/env python3
"""Regression checks for runtime npm inventory and packaged license closure."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from npm_inventory import copy_licenses, installed_packages, prune_development_workspaces


class NpmLicenseChecks(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.plugins = self.root/'plugins'
        self.plugin = self.plugins/'bridge'
        self.plugin.mkdir(parents=True)
        self.doc = self.root/'doc'
        self.lock = {'lockfileVersion': 3, 'packages': {}}

    def package(self, path='node_modules/fast-uri', name='fast-uri', version='3.1.8', license='BSD-3-Clause'):
        root = self.plugin/path
        root.mkdir(parents=True)
        (root/'package.json').write_text(json.dumps({'name': name, 'version': version, 'license': license}))
        (root/'LICENSE').write_text('Fixture license for '+name+'\n')
        (root/'index.js').write_text('module.exports = "runtime fixture";\n')
        self.lock['packages'][path] = {'version': version, 'license': license}
        self.write_lock()
        return root

    def write_lock(self):
        (self.plugin/'package-lock.json').write_text(json.dumps(self.lock))

    def benchmark(self, package):
        root = package/'benchmark'
        root.mkdir()
        (root/'package.json').write_text(json.dumps({'name': 'benchmark', 'version': '1.0.0', 'private': True, 'license': 'ISC'}))
        (root/'benchmark.mjs').write_text('// fixture, not runtime\n')
        return root

    def test_nested_benchmark_is_not_dependency_but_blocks_shipping(self):
        package = self.package(); self.benchmark(package)
        self.assertEqual(len(installed_packages(self.plugin)), 1)
        with self.assertRaisesRegex(ValueError, 'Nested npm workspace license'):
            copy_licenses(self.plugins, self.doc)

    def test_pruning_preserves_runtime_and_upstream_license(self):
        package = self.package(); self.benchmark(package)
        before = {p.name: p.read_bytes() for p in package.iterdir() if p.is_file()}
        self.assertEqual(prune_development_workspaces(self.plugin), ['node_modules/fast-uri/benchmark'])
        self.assertEqual(before, {p.name: p.read_bytes() for p in package.iterdir() if p.is_file()})
        records = copy_licenses(self.plugins, self.doc)
        self.assertEqual([r['name'] for r in records], ['fast-uri'])
        for notice in records[0]['license_files']:
            self.assertEqual(hashlib.sha256((self.doc/notice['path']).read_bytes()).hexdigest(), notice['sha256'])
        self.assertEqual(prune_development_workspaces(self.plugin), [])

    def test_new_fast_uri_version_requires_review(self):
        self.benchmark(self.package(version='4.0.0'))
        with self.assertRaisesRegex(ValueError, 'renewed review'):
            prune_development_workspaces(self.plugin)

    def test_unreviewed_nested_workspace_is_not_silently_pruned(self):
        package = self.package(name='other'); self.benchmark(package)
        self.assertEqual(prune_development_workspaces(self.plugin), [])
        with self.assertRaisesRegex(ValueError, 'requires review'):
            copy_licenses(self.plugins, self.doc)

    def test_scoped_and_nested_installed_packages(self):
        self.package('node_modules/@scope/parent', '@scope/parent', '1.0.0')
        self.package('node_modules/@scope/parent/node_modules/child', 'child', '2.0.0')
        self.assertEqual({r['name'] for r in copy_licenses(self.plugins, self.doc)}, {'@scope/parent', 'child'})

    def test_named_internal_module_with_same_license_is_not_dependency(self):
        package = self.package()
        (package/'internal').mkdir()
        (package/'internal/package.json').write_text(json.dumps({'name': 'internal', 'version': '1.0.0', 'license': 'BSD-3-Clause'}))
        self.assertEqual(len(copy_licenses(self.plugins, self.doc)), 1)

    def test_missing_license_fails(self):
        (self.package()/'LICENSE').unlink()
        with self.assertRaisesRegex(ValueError, 'Missing npm dependency license text'):
            copy_licenses(self.plugins, self.doc)

    def test_child_license_does_not_cover_parent(self):
        (self.package()/'LICENSE').unlink()
        self.package('node_modules/fast-uri/node_modules/child', 'child')
        with self.assertRaisesRegex(ValueError, 'Missing npm dependency license text'):
            copy_licenses(self.plugins, self.doc)

    def test_empty_license_fails(self):
        (self.package()/'LICENSE').write_text('')
        with self.assertRaisesRegex(ValueError, 'Invalid npm license file'):
            copy_licenses(self.plugins, self.doc)

    def test_unlocked_package_fails(self):
        self.package(); self.lock['packages'].clear(); self.write_lock()
        with self.assertRaisesRegex(ValueError, 'does not match lockfile'):
            copy_licenses(self.plugins, self.doc)

    def test_changed_version_fails(self):
        self.package(); self.lock['packages']['node_modules/fast-uri']['version'] = '0.0.1'; self.write_lock()
        with self.assertRaisesRegex(ValueError, 'does not match lockfile'):
            copy_licenses(self.plugins, self.doc)

    def test_changed_license_fails(self):
        self.package(); self.lock['packages']['node_modules/fast-uri']['license'] = 'ISC'; self.write_lock()
        with self.assertRaisesRegex(ValueError, 'license differs'):
            copy_licenses(self.plugins, self.doc)

    def test_dependency_without_lockfile_fails(self):
        self.package(); (self.plugin/'package-lock.json').unlink()
        with self.assertRaisesRegex(ValueError, 'no lockfile'):
            copy_licenses(self.plugins, self.doc)

    def test_same_dependency_in_two_plugins_keeps_both_notices(self):
        self.package()
        self.plugin = self.plugins/'another'; self.plugin.mkdir()
        self.package()
        records = copy_licenses(self.plugins, self.doc)
        self.assertEqual(len(records), 2)
        self.assertNotEqual(records[0]['license_files'][0]['path'], records[1]['license_files'][0]['path'])

    def test_symlinked_package_fails(self):
        self.package()
        (self.plugin/'node_modules/linked').symlink_to('fast-uri')
        with self.assertRaisesRegex(ValueError, 'Symlinked npm package'):
            copy_licenses(self.plugins, self.doc)


if __name__ == '__main__':
    unittest.main()
