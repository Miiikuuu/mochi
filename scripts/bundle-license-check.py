#!/usr/bin/env python3
"""Browser dependency closure must fail closed on unaccounted inputs or notices."""
from pathlib import Path
import json
import shutil
import tempfile
import unittest
from bundle_inventory import validate, copy_licenses
ROOT = Path(__file__).resolve().parents[1]


class BundleChecks(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)/'plugin'
        shutil.copytree(ROOT/'plugins/mochi-chatgpt/ui', self.root/'ui')

    def tearDown(self):
        self.temp.cleanup()

    def test_ext_apps_and_copy_identical_inventory(self):
        inventory = copy_licenses(self.root, Path(self.temp.name)/'doc')
        self.assertTrue(any(p['package']=='@modelcontextprotocol/ext-apps' and p['version']=='2.0.0' for p in inventory['packages']))
        self.assertEqual((self.root/'ui/bundled-js-licenses.json').read_bytes(), (Path(self.temp.name)/'doc/bundled-js-licenses.json').read_bytes())

    def test_unrecorded_input_rejected(self):
        path = self.root/'ui/connection.metafile.json'
        value = json.loads(path.read_text());value['inputs']['node_modules/unrecorded/index.js']={}
        path.write_text(json.dumps(value))
        with self.assertRaisesRegex(ValueError,'no license record'):validate(self.root)

    def test_removed_record_rejected(self):
        path = self.root/'ui/bundled-js-licenses.json'
        value = json.loads(path.read_text());value['packages'].pop();path.write_text(json.dumps(value))
        with self.assertRaises(ValueError):validate(self.root)

    def test_altered_bundle_rejected(self):
        path = self.root/'ui/connection.bundle.html';path.write_text('altered')
        with self.assertRaisesRegex(ValueError,'checksum'):validate(self.root)

    def test_missing_license_rejected(self):
        record = validate(self.root)['packages'][0]
        (self.root/record['license_files'][0]['path']).unlink()
        with self.assertRaises(OSError):validate(self.root)

    def test_installed_doc_notices_validate_without_source_notices(self):
        doc = Path(self.temp.name)/'doc'
        original = copy_licenses(self.root, doc)
        shutil.rmtree(self.root/'ui/bundle-licenses')
        self.assertEqual(validate(self.root, doc/'dependency-licenses/bundle'), original)
        first = original['packages'][0]['license_files'][0]['path']
        (doc/'dependency-licenses/bundle'/Path(first).relative_to('ui/bundle-licenses')).write_text('tampered')
        with self.assertRaisesRegex(ValueError, 'checksum'):
            validate(self.root, doc/'dependency-licenses/bundle')


if __name__=='__main__':unittest.main()
