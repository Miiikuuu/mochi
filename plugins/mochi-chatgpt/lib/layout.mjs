import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
export function pythonHelper(name) {
  const source = fileURLToPath(new URL('../../../platform/linux/'+name, import.meta.url));
  return existsSync(source) ? source : fileURLToPath(new URL('../../../../../libexec/mochi/platform/linux/'+name, import.meta.url));
}
