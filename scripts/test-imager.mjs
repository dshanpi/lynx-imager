// Node 20 CI and Node 24 development both receive explicit test file paths.
// Neither directory discovery nor shell glob expansion is assumed.
import { readdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

const directory = fileURLToPath(new URL('../tests/imager/', import.meta.url));
const files = readdirSync(directory)
  .filter((name) => name.endsWith('.test.mjs'))
  .sort()
  .map((name) => fileURLToPath(new URL('../tests/imager/' + name, import.meta.url)));
if (!files.length) throw new Error('No Imager test files found');
const result = spawnSync(process.execPath, ['--test', ...files], { stdio: 'inherit' });
if (result.error) throw result.error;
process.exit(result.status ?? 1);
