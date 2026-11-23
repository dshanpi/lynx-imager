import { existsSync, readFileSync } from 'fs';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'url';
import { dirname, join } from 'path';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const configPath = join(scriptDir, '..', 'src-tauri', 'tauri.conf.json');
const config = JSON.parse(readFileSync(configPath, 'utf8'));
const mainWindow = config.app?.windows?.[0];
const hostTarget = {
  'linux-x64': 'linux-x86_64',
  'win32-x64': 'windows-x86_64',
  'darwin-arm64': 'darwin-aarch64',
}[`${process.platform}-${process.arch}`];
const target = process.env.LYNX_RUNTIME_TARGET || hostTarget;
const openocdExecutable = {
  'linux-x86_64': 'bin/openocd',
  'windows-x86_64': 'bin/openocd.exe',
  'darwin-aarch64': 'bin/openocd',
}[target];

if (!mainWindow) {
  throw new Error('Packaging aborted: the Tauri main window is not configured.');
}

if (mainWindow.visible === false) {
  throw new Error(
    'Packaging aborted: the Tauri main window is hidden and no startup visibility is guaranteed.'
  );
}

if (!target || !openocdExecutable) {
  throw new Error(
    `Packaging aborted: unsupported runtime target ${target || `${process.platform}-${process.arch}`}.`
  );
}

// The local Windows test package reuses the hardware-accepted executable.
// The pinned source alone cannot detect a previous binary left in this cache.
if (target === 'windows-x86_64') {
  const openix = join(scriptDir, '..', 'src-tauri', 'resources', 'binaries', 'openixcli.exe');
  const expected = '790347ec815bdd691f4b061d327a05277e671355f21ad6b5366ff2469277b284';
  const actual = createHash('sha256').update(readFileSync(openix)).digest('hex');
  if (actual !== expected) {
    throw new Error(`Windows bundle stopped: openixcli.exe SHA-256 ${actual}; expected ${expected}. See src-tauri/resources/binaries/README.md.`);
  }
}

const openocdRoot = join(scriptDir, '..', 'src-tauri', 'resources', 'runtime', target, 'openocd');
const markerPath = join(openocdRoot, '.lynx-runtime.json');
if (!existsSync(join(openocdRoot, openocdExecutable)) || !existsSync(markerPath)) {
  throw new Error(`Packaging aborted: verified OpenOCD runtime is missing for ${target}.`);
}
const marker = JSON.parse(readFileSync(markerPath, 'utf8'));
if (marker.target !== target || marker.version !== '0.12.0-7') {
  throw new Error(
    `Packaging aborted: OpenOCD runtime marker mismatch for ${target}: ${JSON.stringify(marker)}.`
  );
}

console.log(`Package configuration validated for ${target}; OpenOCD ${marker.version} is ready.`);
