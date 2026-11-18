import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(scriptDir, '..');
const tag = process.argv[2] || process.env.GITHUB_REF_NAME || '';

if (!/^v\d+\.\d+(?:\.\d+)?(?:-[0-9A-Za-z]+(?:[.-][0-9A-Za-z]+)*)?$/.test(tag)) {
  throw new Error(`Release tag must use vMAJOR.MINOR[.PATCH][-PRERELEASE]; received "${tag}".`);
}

const numericTag = tag.slice(1);
const [core, prerelease] = numericTag.split('-', 2);
const normalizedCore = core.split('.').length === 2 ? `${core}.0` : core;
const expected = prerelease ? `${normalizedCore}-${prerelease}` : normalizedCore;
const packageVersion = JSON.parse(readFileSync(join(repoRoot, 'package.json'), 'utf8')).version;
const packageLock = JSON.parse(readFileSync(join(repoRoot, 'package-lock.json'), 'utf8'));
const tauriVersion = JSON.parse(
  readFileSync(join(repoRoot, 'src-tauri', 'tauri.conf.json'), 'utf8')
).version;
const cargoToml = readFileSync(join(repoRoot, 'src-tauri', 'Cargo.toml'), 'utf8');
const cargoVersion = cargoToml.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
const cargoLock = readFileSync(join(repoRoot, 'src-tauri', 'Cargo.lock'), 'utf8');
const cargoLockVersion = cargoLock.match(
  /\[\[package\]\]\r?\nname = "lynx-imager"\r?\nversion = "([^"]+)"/
)?.[1];
const installer = readFileSync(join(repoRoot, 'scripts', 'windows-installer.nsi'), 'utf8');
const installerVersion = installer.match(/!define PRODUCT_VERSION "([^"]+)"/)?.[1];

const versions = {
  'package.json': packageVersion,
  'package-lock.json': packageLock.version,
  'package-lock.json root package': packageLock.packages?.['']?.version,
  'src-tauri/Cargo.toml': cargoVersion,
  'src-tauri/Cargo.lock root package': cargoLockVersion,
  'src-tauri/tauri.conf.json': tauriVersion,
  'scripts/windows-installer.nsi': installerVersion,
};
const mismatches = Object.entries(versions).filter(([, version]) => version !== expected);
if (mismatches.length) {
  throw new Error(
    `Release tag ${tag} does not match: ` +
      mismatches.map(([file, version]) => `${file}=${version || '<missing>'}`).join(', ')
  );
}

const fileVersion = installer.match(/!define PRODUCT_FILE_VERSION "([^"]+)"/)?.[1];
if (fileVersion !== `${normalizedCore}.0`) {
  throw new Error(
    `NSIS PRODUCT_FILE_VERSION=${fileVersion || '<missing>'}, expected ${normalizedCore}.0`
  );
}

console.log(
  `Release version ${expected} is consistent across package, Cargo, Tauri and NSIS metadata.`
);
