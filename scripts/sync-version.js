import { readFileSync, writeFileSync } from 'fs';
import { fileURLToPath } from 'url';
import { dirname, join } from 'path';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

function writeIfChanged(path, next, label) {
  const current = readFileSync(path, 'utf-8');
  if (current === next) {
    console.log(`Version ${label} is already current`);
    return;
  }
  writeFileSync(path, next);
  console.log(`Synced version ${packageJson.version} to ${label}`);
}

const packageJson = JSON.parse(readFileSync(join(__dirname, '..', 'package.json'), 'utf-8'));
const packageLockPath = join(__dirname, '..', 'package-lock.json');
const packageLock = JSON.parse(readFileSync(packageLockPath, 'utf-8'));
packageLock.version = packageJson.version;
if (packageLock.packages?.['']) packageLock.packages[''].version = packageJson.version;
writeIfChanged(packageLockPath, `${JSON.stringify(packageLock, null, 2)}\n`, 'package-lock.json');

const tauriConfPath = join(__dirname, '..', 'src-tauri', 'tauri.conf.json');
const tauriConf = JSON.parse(readFileSync(tauriConfPath, 'utf-8'));

tauriConf.version = packageJson.version;
writeIfChanged(tauriConfPath, JSON.stringify(tauriConf, null, 2), 'tauri.conf.json');

const cargoTomlPath = join(__dirname, '..', 'src-tauri', 'Cargo.toml');
let cargoToml = readFileSync(cargoTomlPath, 'utf-8');
cargoToml = cargoToml.replace(/^(version\s*=\s*)"[^"]*"/m, `$1"${packageJson.version}"`);
writeIfChanged(cargoTomlPath, cargoToml, 'Cargo.toml');

const cargoLockPath = join(__dirname, '..', 'src-tauri', 'Cargo.lock');
const cargoLock = readFileSync(cargoLockPath, 'utf-8');
const rootPackage = /(\[\[package\]\]\r?\nname = "lynx-app"\r?\nversion = ")[^"]+("\r?\n)/g;
if ([...cargoLock.matchAll(rootPackage)].length !== 1) {
  throw new Error('Cargo.lock must contain exactly one lynx-app package');
}
writeIfChanged(
  cargoLockPath,
  cargoLock.replace(rootPackage, `$1${packageJson.version}$2`),
  'Cargo.lock'
);

const installerPath = join(__dirname, 'windows-installer.nsi');
let installer = readFileSync(installerPath, 'utf-8');
const fileVersion = `${packageJson.version.split('-', 1)[0]}.0`;
installer = installer
  .replace(
    /(!define OUTPUT_FILE "\.\.\\release\\windows\\LYNX_)[^_]+(_x64-setup\.exe")/,
    `$1${packageJson.version}$2`
  )
  .replace(/(!define PRODUCT_VERSION ")[^"]+("\s*)/, `$1${packageJson.version}$2`)
  .replace(/(!define PRODUCT_FILE_VERSION ")[^"]+("\s*)/, `$1${fileVersion}$2`);
writeIfChanged(installerPath, installer, 'windows-installer.nsi');
