import { build } from 'esbuild';
import { readFile, writeFile, readdir, mkdir } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';

const root = dirname(fileURLToPath(import.meta.url));
const check = process.argv.includes('--check');
const script = await build({ absWorkingDir: root, entryPoints: ['ui/connection.mjs'], bundle: true,
  write: false, metafile: true, format: 'esm', minify: true, target: 'es2022', legalComments: 'inline' });
const template = await readFile(resolve(root, 'ui/connection.html'), 'utf8');
const bundle = template.replace('/* MOCHI_BUNDLE */', () => script.outputFiles[0].text.replaceAll('</script', '<\\/script'));
const output = 'ui/connection.bundle.html';
const packages = new Map();
const artifacts = new Map([[output, bundle]]);
for (const input of Object.keys(script.metafile.inputs).sort()) {
  if (['ui/connection.mjs', 'lib/relay.mjs'].includes(input)) continue; // Explicit original Mochi inputs.
  if (!input.startsWith('node_modules/')) throw new Error(`Non-Mochi bundle input has no license record: ${input}`);
  let directory = dirname(resolve(root, input));
  let metadata;
  while (directory.startsWith(resolve(root, 'node_modules') + '/')) {
    try {
      const candidate = JSON.parse(await readFile(resolve(directory, 'package.json'), 'utf8'));
      if (candidate.name && candidate.version) { metadata = candidate; break; }
    } catch (error) { if (error.code !== 'ENOENT') throw error; }
    directory = dirname(directory);
  }
  if (!metadata || typeof metadata.license !== 'string' || !metadata.license.trim())
    throw new Error(`Bundle input lacks package/license metadata: ${input}`);
  const key = metadata.name + '@' + metadata.version;
  if (!packages.has(key)) {
    const names = (await readdir(directory)).filter(name => /^(LICENSE|LICENCE|COPYING|NOTICE|COPYRIGHT)([.-]|$)/i.test(name)).sort();
    if (!names.some(name => /^(LICENSE|LICENCE|COPYING)([.-]|$)/i.test(name)))
      throw new Error(`Bundle package lacks license text: ${key}`);
    const licenses = [];
    for (const name of names) {
      const content = await readFile(resolve(directory, name), 'utf8');
      const path = `ui/bundle-licenses/${key.replaceAll('/', '__')}/${name}`;
      artifacts.set(path, content);
      licenses.push({ path, sha256: createHash('sha256').update(content).digest('hex') });
    }
    packages.set(key, { package: metadata.name, version: metadata.version, license: metadata.license,
      source_files: [], bundle_output: output, license_files: licenses });
  }
  packages.get(key).source_files.push(input);
}
const inventory = { format: 1, bundle_output: output,
  bundle_sha256: createHash('sha256').update(bundle).digest('hex'),
  packages: [...packages.values()].sort((a, b) => a.package.localeCompare(b.package, 'en')) };
artifacts.set('ui/connection.metafile.json', JSON.stringify(script.metafile, null, 2) + '\n');
artifacts.set('ui/bundled-js-licenses.json', JSON.stringify(inventory, null, 2) + '\n');
for (const [path, content] of artifacts) {
  if (check) {
    if (await readFile(resolve(root, path), 'utf8') !== content)
      throw new Error(`Committed bundle/license artifact differs from lockfile build: ${path}`);
  } else {
    await mkdir(dirname(resolve(root, path)), { recursive: true });
    await writeFile(resolve(root, path), content);
  }
}
console.log(`${check ? 'Verified' : 'Built'} ${output}: ${packages.size} licensed bundle dependencies`);
