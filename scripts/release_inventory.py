"""Inventory the selected Cargo graph and actually installed npm packages."""
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tomllib


def inventory(root, stage, binary, spine=False):
    doc = stage/'usr/share/doc/mochi'
    args = ['cargo','tree','--locked','-p','mochi-desktop','--edges','normal,build','--prefix','none','--format','{p}|{l}']
    if spine:
        args += ['--features','spine']
    tree = subprocess.check_output(args,cwd=root,text=True)
    if not spine and 'mochi-spine ' in tree:
        raise ValueError('Public renderer dependency graph contains Spine.')
    cargo = {}
    for line in tree.splitlines():
        match = re.fullmatch(r'([^ ]+) v([^ ]+)(?: .*?)?\|(.*?)(?: \(\*\))?', line)
        if not match:
            raise ValueError('Unrecognized Cargo inventory record: '+line)
        name, version, license = match.groups()
        cargo[(name,version)] = license.replace('/', ' OR ')
    metadata = json.loads(subprocess.check_output(['cargo','metadata','--locked','--format-version','1'],cwd=root,text=True))
    packages=[]
    cargo_records=[]
    for package in sorted(metadata['packages'],key=lambda p:(p['name'],p['version'])):
        key=(package['name'],package['version'])
        if key not in cargo:
            continue
        source=Path(package['manifest_path']).parent
        license = package.get('license') or ('LicenseRef-Spine-Runtimes' if key[0]=='mochi-spine' else 'NOASSERTION')
        if license == 'NOASSERTION':
            raise ValueError('Dependency license requires review: '+key[0])
        target=doc/'dependency-licenses/cargo'/(key[0]+'-'+key[1])
        for item in sorted(source.rglob('*')):
            if item.is_file() and item.name.upper().startswith(('LICENSE','COPYING','NOTICE','COPYRIGHT')):
                target.mkdir(parents=True,exist_ok=True)
                notice = target/item.relative_to(source)
                notice.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(item,notice)
        if key[0].startswith('rquickjs-') and license == 'MIT':
            provider = next(p for p in metadata['packages'] if p['name'] == 'rquickjs' and p['version'] == key[1] and p.get('repository') == package.get('repository'))
            target.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(Path(provider['manifest_path']).parent/'LICENSE', target/'RQUICKJS-LICENSE')
        if package.get('source') and not target.exists():
            # Some published crates omit their workspace-level license file.
            # Select the explicitly offered Apache option, preserving authors
            # and declared expression separately; never invent a new grant.
            if license in ('Apache-2.0', 'MIT OR Apache-2.0', 'Apache-2.0 OR MIT', 'Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT'):
                target.mkdir(parents=True)
                shutil.copyfile(root/'LICENSE',target/'Apache-2.0')
            elif license == 'Apache-2.0 WITH LLVM-exception':
                # Cranelift subcrates omit their workspace license in crates.io.
                # Use the complete upstream text from this same locked graph.
                provider = next(p for p in metadata['packages'] if p['name'] == 'cranelift-codegen' and p.get('license') == license)
                target.mkdir(parents=True)
                shutil.copyfile(Path(provider['manifest_path']).parent/'LICENSE', target/'LICENSE')
            elif license == 'CC0-1.0':
                target.mkdir(parents=True)
                shutil.copyfile('/usr/share/common-licenses/CC0-1.0',target/'CC0-1.0')
            else:
                raise ValueError('Missing Cargo dependency license text: '+key[0])
        if target.exists():
            (target/'attribution.json').write_text(json.dumps({'authors':package.get('authors',[]),'repository':package.get('repository'),'declared_license':license},sort_keys=True,indent=2)+'\n')
        record={'name':key[0], 'version':key[1], 'license':license, 'source':package.get('source') or 'Mochi original source'}
        cargo_records.append(record)
        packages.append(spdx_package('cargo',*key,license))
    from npm_inventory import copy_licenses as copy_npm_licenses
    npm_records = copy_npm_licenses(stage/'usr/share/mochi/plugins', doc)
    for data in npm_records:
        packages.append(spdx_package('npm',data['name'],data['version'],data['license']))
    from bundle_inventory import copy_licenses
    bundled = copy_licenses(root/'plugins/mochi-chatgpt', doc)
    bundle_id = 'SPDXRef-ConnectionBundle'
    bundle_relations = []
    for item in bundled['packages']:
        package = spdx_package('npm', item['package'], item['version'], item['license'])
        packages.append(package)
        bundle_relations.append({'spdxElementId': bundle_id, 'relationshipType': 'CONTAINS', 'relatedSpdxElement': package['SPDXID']})
    unique={p['SPDXID']:p for p in packages}
    version=tomllib.loads((root/'Cargo.toml').read_text())['workspace']['package']['version']
    epoch=int(os.environ.get('SOURCE_DATE_EPOCH','0'))
    from datetime import datetime,timezone
    timestamp=datetime.fromtimestamp(epoch,timezone.utc).isoformat().replace('+00:00','Z')
    sbom={'spdxVersion':'SPDX-2.3','dataLicense':'CC0-1.0','SPDXID':'SPDXRef-DOCUMENT',
          'name':'Mochi '+version+' '+('private-full' if spine else 'no-spine'),
          'documentNamespace':'https://mochi.invalid/sbom/'+hashlib.sha256(json.dumps(sorted(unique)).encode()).hexdigest(),
          'creationInfo':{'created':timestamp,'creators':['Tool: Mochi release_inventory.py']},
          'packages':list(unique.values()),
          'relationships':[{'spdxElementId':'SPDXRef-DOCUMENT','relationshipType':'DESCRIBES','relatedSpdxElement':key} for key in unique]}
    sbom['files'] = [{'SPDXID': bundle_id, 'fileName': './usr/share/mochi/plugins/mochi-chatgpt/ui/connection.bundle.html',
                      'checksums': [{'algorithm': 'SHA256', 'checksumValue': bundled['bundle_sha256']}],
                      'licenseConcluded': 'NOASSERTION', 'copyrightText': 'NOASSERTION'}]
    sbom['relationships'] += [{'spdxElementId': 'SPDXRef-DOCUMENT', 'relationshipType': 'DESCRIBES', 'relatedSpdxElement': bundle_id}, *bundle_relations]
    if spine:
        sbom['hasExtractedLicensingInfos']=[{'licenseId':'LicenseRef-Spine-Runtimes','extractedText':(root/'crates/mochi-spine/vendor/spine-c-3.8/LICENSE').read_text()}]
    for filename,value in (('sbom.spdx.json',sbom),('cargo-licenses.json',cargo_records),('npm-licenses.json',npm_records)):
        (doc/filename).write_text(json.dumps(value,sort_keys=True,indent=2)+'\n')
    provenance={'format':1,'version':version,'spine':spine,'binary_sha256':hashlib.sha256((stage/'usr/libexec/mochi/bin/mochi-desktop').read_bytes()).hexdigest(),
                'cargo_lock_sha256':hashlib.sha256((root/'Cargo.lock').read_bytes()).hexdigest(),
                'source_tree_sha256':source_digest(root),
                'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),
                'node':subprocess.check_output(['node','--version'],text=True).strip(),
                'note':'Staging provenance, not a public release or redistribution authorization.'}
    (doc/'provenance.json').write_text(json.dumps(provenance,sort_keys=True,indent=2)+'\n')


def source_digest(root):
    digest=hashlib.sha256()
    for base in ('crates','examples/desktop','platform/linux','scripts','packaging','plugins','assets/bubbles'):
        for path in sorted((root/base).rglob('*')):
            if path.is_file() and not path.is_symlink() and path.suffix in ('.rs','.toml','.py','.wgsl','.json','.js','.mjs','.ts','.html','.css','.svg') and not {'__pycache__','node_modules'}.intersection(path.parts):
                digest.update(str(path.relative_to(root)).encode()+b'\0'+path.read_bytes())
    for name in ('Cargo.toml', 'Cargo.lock', 'LICENSE', 'NOTICE'):
        path = root/name
        if path.is_file():
            digest.update(name.encode()+b'\0'+path.read_bytes())
    return digest.hexdigest()


def spdx_package(ecosystem,name,version,license):
    identity=hashlib.sha256((ecosystem+':'+name+'@'+version).encode()).hexdigest()[:24]
    return {'name':name,'SPDXID':'SPDXRef-'+identity,'versionInfo':version,
            'downloadLocation':'NOASSERTION','filesAnalyzed':False,
            'licenseConcluded':'NOASSERTION','licenseDeclared':license.replace('/', ' OR '),
            'copyrightText':'NOASSERTION',
            'externalRefs':[{'referenceCategory':'PACKAGE-MANAGER','referenceType':'purl','referenceLocator':'pkg:'+ecosystem+'/'+name.replace('@','%40')+'@'+version}]}
