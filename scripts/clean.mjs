// `make clean` (JS side): removes web build outputs. `cargo clean` handles target/.
import { rmSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
for (const dir of [
  ['apps', 'web', 'build'],
  ['apps', 'web', '.svelte-kit']
]) {
  rmSync(join(root, ...dir), { recursive: true, force: true });
  console.info(`clean: removed ${dir.join('/')}`);
}
