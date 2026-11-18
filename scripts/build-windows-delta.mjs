#!/usr/bin/env node

import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(scriptDir, '..');
const helperRoot = join(repoRoot, 'tools', 'lynx-delta-helper');
const semverPattern = /^(?:v)?(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;

function requireValue(condition, message) {
  if (!condition) throw new Error(message);
}

function run(program, args, options = {}) {
  const result = spawnSync(program, args, {
    cwd: repoRoot,
    encoding: 'utf8',
    maxBuffer: 256 * 1024 * 1024,
    ...options,
  });
  requireValue(
    result.status === 0,
    `${program} failed (${result.status ?? 'spawn'}): ${(result.stderr || result.stdout).trim()}`
  );
  return result.stdout.trim();
}

export function normalizeStableVersion(value) {
  const match = String(value || '').match(semverPattern);
  requireValue(match, `Invalid stable semantic version: ${value}`);
  return `${match[1]}.${match[2]}.${match[3]}`;
}

export function compareStableVersions(leftValue, rightValue) {
  const left = normalizeStableVersion(leftValue).split('.').map(Number);
  const right = normalizeStableVersion(rightValue).split('.').map(Number);
  for (let index = 0; index < left.length; index += 1) {
    if (left[index] !== right[index]) return left[index] - right[index];
  }
  return 0;
}

export function deltaVersionsCompatible(fromValue, toValue) {
  const from = normalizeStableVersion(fromValue).split('.').map(Number);
  const to = normalizeStableVersion(toValue).split('.').map(Number);
  const newer = to.some(
    (part, index) =>
      part > from[index] && to.slice(0, index).every((value, prior) => value === from[prior])
  );
  return newer && from[0] === to[0] && (to[0] !== 0 || from[1] === to[1]);
}

export function selectPreviousCompatibleRelease(releases, currentValue) {
  const current = normalizeStableVersion(currentValue);
  const candidates = [];
  for (const release of Array.isArray(releases) ? releases : []) {
    if (!release || release.isDraft || release.isPrerelease) continue;
    try {
      const version = normalizeStableVersion(release.tagName);
      if (deltaVersionsCompatible(version, current)) {
        candidates.push({ tag: release.tagName, version });
      }
    } catch {
      // Ignore non-semver tags in the binary-only mirror repository.
    }
  }
  candidates.sort((left, right) => compareStableVersions(right.version, left.version));
  return candidates[0];
}

function extractInstaller(installer, destination) {
  mkdirSync(destination, { recursive: true });
  run('7z', [
    'x',
    '-y',
    `-o${destination}`,
    installer,
    '-x!$PLUGINSDIR/*',
    '-x!uninstall.exe',
    '-x!Uninstall.exe',
  ]);
  for (const required of [
    'libusb-1.0.dll',
    'CH347DLLA64.dll',
    'bin/openixcli.exe',
  ]) {
    requireValue(
      existsSync(join(destination, required)),
      `${basename(installer)} does not contain ${required}`
    );
  }
  requireValue(
    ['lynx-imager.exe', 'lynx-app.exe'].some((name) => existsSync(join(destination, name))),
    `${basename(installer)} does not contain a LYNX application executable`
  );
}

export function buildWindowsDelta({ fromInstaller, toInstaller, fromVersion, toVersion, output }) {
  fromInstaller = resolve(fromInstaller);
  toInstaller = resolve(toInstaller);
  output = resolve(output);
  fromVersion = normalizeStableVersion(fromVersion);
  toVersion = normalizeStableVersion(toVersion);
  requireValue(
    deltaVersionsCompatible(fromVersion, toVersion),
    `Delta versions are incompatible: ${fromVersion} -> ${toVersion}`
  );
  requireValue(existsSync(fromInstaller), `Previous full installer is absent: ${fromInstaller}`);
  requireValue(existsSync(toInstaller), `Current full installer is absent: ${toInstaller}`);
  requireValue(!existsSync(output), `Refusing to overwrite existing delta installer: ${output}`);
  mkdirSync(dirname(output), { recursive: true });

  const temporary = mkdtempSync(join(tmpdir(), 'lynx-delta-build-'));
  try {
    const oldTree = join(temporary, 'old');
    const newTree = join(temporary, 'new');
    const bundle = join(temporary, 'bundle');
    extractInstaller(fromInstaller, oldTree);
    extractInstaller(toInstaller, newTree);

    run('cargo', ['build', '--release', '--manifest-path', join(helperRoot, 'Cargo.toml')]);
    const hostHelper = join(helperRoot, 'target', 'release', 'lynx-delta-helper');
    requireValue(existsSync(hostHelper), 'Host delta helper build did not produce an executable.');
    run(hostHelper, ['create', oldTree, newTree, bundle, fromVersion, toVersion]);

    run('cargo', [
      'build',
      '--release',
      '--target',
      'x86_64-pc-windows-gnu',
      '--manifest-path',
      join(helperRoot, 'Cargo.toml'),
    ]);
    const windowsHelper = join(
      helperRoot,
      'target',
      'x86_64-pc-windows-gnu',
      'release',
      'lynx-delta-helper.exe'
    );
    requireValue(
      existsSync(windowsHelper),
      'Windows delta helper build did not produce an executable.'
    );

    const versionParts = toVersion.split('.');
    const fileVersion = `${versionParts.join('.')}.0`;
    run('makensis', [
      `-DDELTA_BUNDLE_DIR=${bundle}`,
      `-DDELTA_HELPER_EXE=${windowsHelper}`,
      `-DOUTPUT_FILE=${output}`,
      `-DFROM_VERSION=${fromVersion}`,
      `-DPRODUCT_VERSION=${toVersion}`,
      `-DPRODUCT_FILE_VERSION=${fileVersion}`,
      join(scriptDir, 'windows-delta-installer.nsi'),
    ]);
    requireValue(
      readFileSync(output).subarray(0, 2).equals(Buffer.from('MZ')),
      'Generated delta installer is not a PE executable.'
    );
    run('7z', ['t', output]);

    const deltaSize = statSync(output).size;
    const fullSize = statSync(toInstaller).size;
    const configuredLimit = Number(process.env.LYNX_DELTA_MAX_BYTES || 32 * 1024 * 1024);
    requireValue(
      Number.isSafeInteger(configuredLimit) && configuredLimit > 0,
      'LYNX_DELTA_MAX_BYTES must be a positive integer.'
    );
    if (deltaSize >= fullSize || deltaSize > configuredLimit) {
      rmSync(output, { force: true });
      return {
        created: false,
        deltaSize,
        fullSize,
        fromVersion,
        toVersion,
        reason: `delta is not economical (limit=${configuredLimit})`,
      };
    }
    return { created: true, deltaSize, fullSize, fromVersion, toVersion, output };
  } catch (error) {
    if (existsSync(output)) rmSync(output, { force: true });
    throw error;
  } finally {
    rmSync(temporary, { recursive: true, force: true });
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [fromInstaller, toInstaller, fromVersion, toVersion, output] = process.argv.slice(2);
  requireValue(
    fromInstaller && toInstaller && fromVersion && toVersion && output,
    'Usage: build-windows-delta.mjs <previous-full.exe> <current-full.exe> <from-version> <to-version> <output-delta.exe>'
  );
  const result = buildWindowsDelta({
    fromInstaller,
    toInstaller,
    fromVersion,
    toVersion,
    output,
  });
  if (result.created) {
    console.log(
      `Windows delta ready: ${result.fromVersion} -> ${result.toVersion}, ` +
        `${result.deltaSize} bytes (full ${result.fullSize} bytes). Sign the .exe before publishing.`
    );
  } else {
    console.log(
      `Windows delta skipped: ${result.fromVersion} -> ${result.toVersion}, ` +
        `${result.deltaSize} bytes (full ${result.fullSize} bytes); ${result.reason}.`
    );
  }
}
