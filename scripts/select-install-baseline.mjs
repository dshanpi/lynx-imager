import { readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { normalizeStableVersion, selectPreviousCompatibleRelease } from './build-windows-delta.mjs';

export function selectInstallBaseline(releases, currentValue) {
  if (!Array.isArray(releases)) throw new Error('Release inventory must be an array');
  const current = normalizeStableVersion(currentValue);
  if (current === '0.1.0') return { mode: 'fresh', tag: null, version: null };
  const previous = selectPreviousCompatibleRelease(releases, current);
  if (!previous) throw new Error(`Missing compatible stable install baseline for ${current}`);
  return { mode: 'upgrade', ...previous };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv.length !== 5) {
    throw new Error('Usage: select-install-baseline.mjs VERSION RELEASES_JSON OUTPUT_JSON');
  }
  const [, , version, input, output] = process.argv;
  const plan = selectInstallBaseline(JSON.parse(readFileSync(input, 'utf8')), version);
  writeFileSync(output, `${JSON.stringify(plan, null, 2)}\n`);
}
