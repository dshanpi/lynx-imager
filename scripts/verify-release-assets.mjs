#!/usr/bin/env node

import { createHash } from 'node:crypto';
import { mkdtempSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { verifyWindowsDeliveryZip } from './verify-windows-delivery-zip.mjs';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(scriptDir, '..');
const allowedAsset = /\.(?:exe|msi|AppImage|deb|rpm|dmg|app\.tar\.gz)(?:\.sig)?$/;
const deltaMetadataPattern =
  /^update-windows-x86_64-from-(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)\.json$/;

function requireValue(condition, message) {
  if (!condition) throw new Error(message);
}

function sha256(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex');
}

export function parseChecksums(text) {
  const result = new Map();
  for (const [index, line] of text.trimEnd().split('\n').entries()) {
    const match = line.match(/^([0-9a-f]{64})  ([^/\\]+)$/);
    requireValue(match, `Malformed SHA256SUMS line ${index + 1}.`);
    requireValue(!result.has(match[2]), `Duplicate checksum entry: ${match[2]}`);
    result.set(match[2], match[1]);
  }
  return result;
}

export function binaryKind(buffer) {
  if (buffer.length >= 0x40 && buffer.subarray(0, 2).equals(Buffer.from('MZ'))) {
    const header = buffer.readUInt32LE(0x3c);
    if (
      header + 6 <= buffer.length &&
      buffer.subarray(header, header + 4).equals(Buffer.from('PE\0\0')) &&
      buffer.readUInt16LE(header + 4) === 0x8664
    ) {
      return 'pe-x86_64';
    }
  }
  if (
    buffer.length >= 20 &&
    buffer.subarray(0, 4).equals(Buffer.from([0x7f, 0x45, 0x4c, 0x46])) &&
    buffer[4] === 2 &&
    buffer[5] === 1 &&
    buffer.readUInt16LE(18) === 0x3e
  ) {
    return 'elf-x86_64';
  }
  if (buffer.subarray(0, 8).toString('ascii') === '!<arch>\n') return 'deb';
  if (buffer.subarray(0, 4).equals(Buffer.from([0xed, 0xab, 0xee, 0xdb]))) return 'rpm';
  return 'unknown';
}

function run(program, args, options = {}) {
  const result = spawnSync(program, args, {
    encoding: 'utf8',
    maxBuffer: 128 * 1024 * 1024,
    ...options,
  });
  requireValue(
    result.status === 0,
    `${program} failed: ${String(result.error?.message || result.stderr || result.stdout || `exit ${result.status}`).trim()}`
  );
  return result.stdout;
}

function verifySignature(artifact, encodedSignature, publicKey) {
  const temporary = mkdtempSync(join(tmpdir(), 'lynx-signature-'));
  try {
    const signature = join(temporary, 'artifact.sig');
    writeFileSync(signature, Buffer.from(encodedSignature.trim(), 'base64'));
    run('minisign', ['-Vm', artifact, '-x', signature, '-P', publicKey]);
  } finally {
    rmSync(temporary, { recursive: true, force: true });
  }
}

export function installerListingHasPath(listing, path) {
  const normalizedPath = path.replaceAll('\\', '/').toLowerCase();
  return listing
    .replaceAll('\\', '/')
    .toLowerCase()
    .split(/\r?\n/)
    .some((line) => line.trimEnd().endsWith(normalizedPath));
}

function verifyWindowsInstaller(path) {
  requireValue(
    readFileSync(path).subarray(0, 2).equals(Buffer.from('MZ')),
    'Windows installer is not a PE executable.'
  );
  run('7z', ['t', path]);
  const listing = run('7z', ['l', '-ba', path]).replaceAll('\\', '/').toLowerCase();
  const required = [
    'lynx-app.exe',
    'libusb-1.0.dll',
    'ch347dlla64.dll',
    'bin/openixcli.exe',
    'bin/openrockcli.exe',
    'bin/uuu.exe',
    'bin/dfu-util.exe',
    'pyembed/python.exe',
    'openocd/bin/openocd.exe',
  ];
  // The MSVC build links WebView2LoaderStatic.lib; the Windows install/launch gate
  // verifies the resulting executable instead of requiring the GNU-only loader DLL.
  for (const name of required) {
    requireValue(installerListingHasPath(listing, name), `Windows installer is missing ${name}.`);
  }
  requireValue(
    !listing.includes('cmsis-packs/'),
    'Windows installer contains forbidden RA6M5 offline resources.'
  );
  for (const name of [
    'site-packages/pyocd/',
    '.lynx-pyocd.json',
    'site-packages/cmsis_pack_manager/',
  ]) {
    requireValue(
      !listing.includes(name),
      `Windows installer contains forbidden RA6M5 offline resource ${name}.`
    );
  }
  const forbidden = ['bin/uuu', 'bin/openixcli', 'bin/openrockcli', 'bin/libusb-1.0.so.0'];
  for (const name of forbidden) {
    requireValue(
      !installerListingHasPath(listing, name),
      `Windows installer contains forbidden resource ${name}.`
    );
  }
  requireValue(
    !listing.includes('@vite/client'),
    'Windows installer contains the Vite development client.'
  );
  const temporary = mkdtempSync(join(tmpdir(), 'lynx-nsis-'));
  try {
    run('7z', ['x', '-y', `-o${temporary}`, path, 'lynx-app.exe']);
    const application = readFileSync(join(temporary, 'lynx-app.exe'));
    requireValue(
      binaryKind(application.subarray(0, 1024)) === 'pe-x86_64',
      'Bundled LYNX application is not PE32+ x86-64.'
    );
    verifyWindowsBundleMarker(application);
  } finally {
    rmSync(temporary, { recursive: true, force: true });
  }
}

export function verifyWindowsBundleMarker(application) {
  requireValue(
    !application.includes(Buffer.from('__TAURI_BUNDLE_TYPE_VAR_UNK')) &&
      application.includes(Buffer.from('__TAURI_BUNDLE_TYPE_VAR_NSS')),
    'Windows updater bundle type must be fully patched to NSIS.'
  );
}

function verifyWindowsDeltaInstaller(path) {
  requireValue(
    readFileSync(path).subarray(0, 2).equals(Buffer.from('MZ')),
    'Windows delta installer is not a PE executable.'
  );
  run('7z', ['t', path]);
  const listing = run('7z', ['l', '-ba', path]).replaceAll('\\', '/').toLowerCase();
  requireValue(
    listing.includes('lynx-delta-helper.exe'),
    'Windows delta installer is missing the native patch helper.'
  );
  requireValue(
    listing.includes('delta-manifest.json'),
    'Windows delta installer is missing its file integrity manifest.'
  );
}

function stableVersionParts(value) {
  const match = String(value).match(/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/);
  requireValue(match, `Invalid stable version: ${value}`);
  return match.slice(1).map(Number);
}

function deltaCompatible(fromValue, toValue) {
  const from = stableVersionParts(fromValue);
  const to = stableVersionParts(toValue);
  const newer = to.some(
    (part, index) =>
      part > from[index] && to.slice(0, index).every((value, prior) => value === from[prior])
  );
  return newer && from[0] === to[0] && (to[0] !== 0 || from[1] === to[1]);
}

function expectedPlatforms(platforms) {
  const expected = new Set();
  if (platforms.has('windows')) {
    expected.add('windows-x86_64');
    expected.add('windows-x86_64-nsis');
  }
  if (platforms.has('linux')) expected.add('linux-x86_64');
  if (platforms.has('macos')) expected.add('darwin-aarch64');
  return expected;
}

export function verifyReleaseAssets(
  directory,
  tag,
  platformList,
  repository = 'dshanpi/lynx-releases'
) {
  requireValue(
    /^v\d+\.\d+\.\d+(?:-[0-9A-Za-z]+(?:[.-][0-9A-Za-z]+)*)?$/.test(tag),
    `Invalid release tag: ${tag}`
  );
  const version = tag.slice(1);
  const windowsZip = `LYNX_${version}_Windows_x64.zip`;
  const repairZip = `LYNX_${version}_Windows_x64_github-update.zip`;
  const platforms = new Set(platformList.split(',').filter(Boolean));
  requireValue(platforms.size > 0, 'At least one release platform is required.');
  requireValue(
    [...platforms].every((item) => ['windows', 'linux', 'macos'].includes(item)),
    'Unknown release platform.'
  );

  const names = readdirSync(directory).sort();
  requireValue(
    names.includes('latest.json') &&
      names.includes('release-set.json') &&
      names.includes('SHA256SUMS.txt'),
    'Release metadata is incomplete.'
  );
  for (const name of names) {
    requireValue(
      name === 'latest.json' ||
        name === 'release-set.json' ||
        name === 'release-set.json.sig' ||
        name === 'SHA256SUMS.txt' ||
        name === repairZip ||
        name === `${repairZip}.sig` ||
        name === windowsZip ||
        name === `${windowsZip}.sig` ||
        deltaMetadataPattern.test(name) ||
        allowedAsset.test(name),
      `Forbidden release asset: ${name}`
    );
    if (
      name !== 'latest.json' &&
      name !== 'release-set.json' &&
      name !== 'release-set.json.sig' &&
      name !== 'SHA256SUMS.txt' &&
      !deltaMetadataPattern.test(name)
    ) {
      requireValue(
        name.includes(version),
        `Asset filename does not contain version ${version}: ${name}`
      );
    }
  }

  const checksums = parseChecksums(readFileSync(join(directory, 'SHA256SUMS.txt'), 'utf8'));
  const checksumTargets = names.filter((name) => name !== 'SHA256SUMS.txt');
  requireValue(
    checksums.size === checksumTargets.length,
    'SHA256SUMS does not cover every release file exactly once.'
  );
  for (const name of checksumTargets) {
    requireValue(
      checksums.get(name) === sha256(join(directory, name)),
      `SHA-256 mismatch: ${name}`
    );
  }

  const manifest = JSON.parse(readFileSync(join(directory, 'latest.json'), 'utf8'));
  requireValue(
    manifest.version === version,
    `latest.json version is ${manifest.version}, expected ${version}.`
  );
  requireValue(
    typeof manifest.notes === 'string' && manifest.notes.trim().length > 20,
    'Release notes are missing.'
  );
  requireValue(Number.isFinite(Date.parse(manifest.pub_date)), 'latest.json pub_date is invalid.');
  const expected = expectedPlatforms(platforms);
  requireValue(
    JSON.stringify(Object.keys(manifest.platforms).sort()) === JSON.stringify([...expected].sort()),
    'latest.json platform set does not match the requested release platforms.'
  );

  const tauri = JSON.parse(readFileSync(join(repoRoot, 'src-tauri/tauri.conf.json'), 'utf8'));
  requireValue(tauri.version === version, 'Tauri version does not match the release assets.');
  const decodedKey = Buffer.from(tauri.plugins.updater.pubkey, 'base64').toString('utf8');
  const publicKey = decodedKey.split(/\r?\n/).find((line) => line.startsWith('RW'));
  requireValue(publicKey, 'Configured updater public key is invalid.');
  for (const zip of [windowsZip, repairZip]) {
    requireValue(
      names.includes(zip) === names.includes(`${zip}.sig`),
      'Windows delivery ZIP and its signature must be present together.'
    );
    if (!names.includes(zip)) continue;
    requireValue(platforms.has('windows'), 'Windows delivery ZIP requires a Windows release.');
    verifySignature(
      join(directory, zip),
      readFileSync(join(directory, `${zip}.sig`), 'utf8'),
      publicKey
    );
    const installer =
      zip === repairZip
        ? `LYNX_${version}_github-update_x64-setup.exe`
        : `LYNX_${version}_x64-setup.exe`;
    verifyWindowsDeliveryZip(join(directory, zip), {
      version,
      installerPath: join(directory, installer),
      signaturePath: join(directory, `${installer}.sig`),
      publicKey: decodedKey,
    });
  }

  const verified = new Set();
  for (const [platform, entry] of Object.entries(manifest.platforms)) {
    const expectedPrefix = `https://github.com/${repository}/releases/download/${tag}/`;
    requireValue(entry.url.startsWith(expectedPrefix), `Unexpected updater URL for ${platform}.`);
    const name = decodeURIComponent(entry.url.slice(expectedPrefix.length));
    requireValue(
      name === basename(name) && names.includes(name),
      `Updater artifact is absent: ${name}`
    );
    const signatureName = `${name}.sig`;
    requireValue(names.includes(signatureName), `Updater signature is absent: ${signatureName}`);
    const encodedSignature = readFileSync(join(directory, signatureName), 'utf8').trim();
    requireValue(
      entry.signature === encodedSignature,
      `latest.json signature differs for ${platform}.`
    );
    if (!verified.has(name)) {
      verifySignature(join(directory, name), encodedSignature, publicKey);
      verified.add(name);
    }
  }

  const releaseSet = JSON.parse(readFileSync(join(directory, 'release-set.json'), 'utf8'));
  requireValue(
    releaseSet.schema_version === 1 || releaseSet.schema_version === 2,
    'release-set.json schema is invalid.'
  );
  if (releaseSet.schema_version === 2) {
    requireValue(names.includes('release-set.json.sig'), 'Signed schema 2 manifest is missing.');
    verifySignature(
      join(directory, 'release-set.json'),
      readFileSync(join(directory, 'release-set.json.sig'), 'utf8').trim(),
      publicKey
    );
  }
  requireValue(releaseSet.product === 'lynx', 'release-set.json product is invalid.');
  requireValue(releaseSet.version === version, 'release-set.json version is invalid.');
  requireValue(
    releaseSet.channel === (version.includes('-') ? 'beta' : 'stable'),
    'release-set.json channel is invalid.'
  );
  requireValue(
    Array.isArray(releaseSet.assets) &&
      releaseSet.assets.length >= expected.size &&
      releaseSet.assets.length <= expected.size + 2,
    'release-set.json asset routes are incomplete.'
  );
  const releaseRoutes = new Set();
  const fullTargets = new Set();
  const fullSizes = new Map();
  const deltaInstallers = new Set();
  for (const asset of releaseSet.assets) {
    const storage = asset.storage || (releaseSet.schema_version === 1 ? 'site' : '');
    requireValue(expected.has(asset.target), `Unexpected release target: ${asset.target}`);
    const route = `${asset.target}:${asset.kind}:${asset.from_version || ''}`;
    requireValue(!releaseRoutes.has(route), `Duplicate release route: ${route}`);
    releaseRoutes.add(route);
    if (asset.kind === 'full') {
      requireValue(!asset.from_version, 'Full assets cannot declare from_version.');
      fullTargets.add(asset.target);
      fullSizes.set(asset.target, asset.size);
    } else {
      requireValue(asset.kind === 'delta', `Unsupported release asset kind: ${asset.kind}`);
      requireValue(
        deltaCompatible(asset.from_version, version),
        `Incompatible delta route: ${asset.from_version} -> ${version}`
      );
      requireValue(
        asset.target === 'windows-x86_64' || asset.target === 'windows-x86_64-nsis',
        'Delta updates are currently supported only for Windows NSIS.'
      );
      requireValue(
        asset.file.toLowerCase().endsWith('-delta.exe'),
        `Windows delta filename is invalid: ${asset.file}`
      );
      requireValue(asset.size <= 32 * 1024 * 1024, 'Windows delta exceeds the 32 MiB ceiling.');
      deltaInstallers.add(asset.file);
    }
    requireValue(
      storage === 'site' || storage === 'external',
      `Release asset storage is invalid: ${asset.file}`
    );
    requireValue(
      asset.kind === 'full' || storage === 'site',
      `Delta asset must remain on the download site: ${asset.file}`
    );
    requireValue(asset.file === basename(asset.file), `Unsafe release asset path: ${asset.file}`);
    requireValue(names.includes(asset.file), `Release asset is absent: ${asset.file}`);
    requireValue(
      statSync(join(directory, asset.file)).size === asset.size,
      `Release asset size mismatch: ${asset.file}`
    );
    requireValue(
      sha256(join(directory, asset.file)) === asset.sha256,
      `Release asset SHA-256 mismatch: ${asset.file}`
    );
    requireValue(
      readFileSync(join(directory, `${asset.file}.sig`), 'utf8').trim() === asset.signature,
      `Release asset signature mismatch: ${asset.file}`
    );
    requireValue(
      Array.isArray(asset.mirrors) &&
        asset.mirrors.length === 1 &&
        asset.mirrors[0].startsWith('https://github.com/'),
      `Release asset mirror is invalid: ${asset.file}`
    );
  }
  requireValue(
    JSON.stringify([...fullTargets].sort()) === JSON.stringify([...expected].sort()),
    'Every target requires exactly one complete package fallback.'
  );
  const externalFulls = releaseSet.assets.filter(
    (entry) => entry.kind === 'full' && entry.storage === 'external'
  );
  const siteFulls = releaseSet.assets.filter(
    (entry) =>
      entry.kind === 'full' &&
      (entry.storage === 'site' || (releaseSet.schema_version === 1 && !entry.storage))
  );
  requireValue(
    externalFulls.length === 0 || siteFulls.length === 0,
    'A release cannot mix site and external full-package storage.'
  );
  for (const asset of releaseSet.assets.filter((entry) => entry.kind === 'delta')) {
    requireValue(
      asset.size < fullSizes.get(asset.target),
      `Delta asset is not smaller than its full fallback: ${asset.file}`
    );
  }
  requireValue(
    deltaInstallers.size <= 1,
    'Only one Windows delta executable is allowed per release.'
  );
  for (const installer of deltaInstallers) {
    verifyWindowsDeltaInstaller(join(directory, installer));
  }

  const expectedDeltaMetadata = new Map();
  for (const asset of releaseSet.assets.filter(
    (entry) => entry.kind === 'delta' && entry.target === 'windows-x86_64'
  )) {
    expectedDeltaMetadata.set(`update-windows-x86_64-from-${asset.from_version}.json`, asset);
  }
  const actualDeltaMetadata = [];
  for (const name of names.filter((name) => deltaMetadataPattern.test(name))) {
    const metadata = JSON.parse(readFileSync(join(directory, name), 'utf8'));
    if (metadata.strategy !== 'full') {
      actualDeltaMetadata.push(name);
      continue;
    }
    const sourceVersion = name.match(deltaMetadataPattern).slice(1).join('.');
    requireValue(
      manifest.strategy === 'full' && deltaCompatible(sourceVersion, version),
      `${name} full alias requires a compatible full update feed.`
    );
    requireValue(
      JSON.stringify(metadata) === JSON.stringify(manifest),
      `${name} full alias differs from latest.json.`
    );
    for (const asset of releaseSet.assets) {
      requireValue(
        asset.kind === 'full' && asset.storage === 'external',
        `${name} full alias requires external full routes.`
      );
      const entry = manifest.platforms[asset.target];
      requireValue(
        entry.url === asset.mirrors[0] &&
          entry.signature === asset.signature &&
          manifest.asset_size === asset.size &&
          manifest.asset_sha256 === asset.sha256,
        `${name} full alias does not match its signed release route.`
      );
    }
  }
  requireValue(
    JSON.stringify(actualDeltaMetadata.sort()) ===
      JSON.stringify([...expectedDeltaMetadata.keys()].sort()),
    'GitHub delta metadata does not match the release-set routes.'
  );
  for (const [name, asset] of expectedDeltaMetadata) {
    const metadata = JSON.parse(readFileSync(join(directory, name), 'utf8'));
    requireValue(metadata.version === version, `${name} version is invalid.`);
    requireValue(metadata.notes === releaseSet.notes, `${name} notes differ from release-set.`);
    requireValue(Number.isFinite(Date.parse(metadata.pub_date)), `${name} pub_date is invalid.`);
    requireValue(metadata.strategy === 'delta', `${name} strategy is invalid.`);
    requireValue(metadata.asset_size === asset.size, `${name} asset_size is invalid.`);
    requireValue(metadata.asset_sha256 === asset.sha256, `${name} asset_sha256 is invalid.`);
    requireValue(metadata.fallback_available === true, `${name} fallback marker is invalid.`);
    requireValue(
      JSON.stringify(Object.keys(metadata.platforms || {}).sort()) ===
        JSON.stringify(['windows-x86_64', 'windows-x86_64-nsis']),
      `${name} platform routes are invalid.`
    );
    for (const entry of Object.values(metadata.platforms)) {
      requireValue(entry.url === asset.mirrors[0], `${name} URL is not the signed delta mirror.`);
      requireValue(entry.signature === asset.signature, `${name} signature is invalid.`);
    }
  }

  if (platforms.has('windows')) {
    const installer = decodeURIComponent(
      new URL(manifest.platforms['windows-x86_64'].url).pathname.split('/').pop()
    );
    requireValue(installer, 'Windows NSIS installer is absent.');
    verifyWindowsInstaller(join(directory, installer));
  }
  if (platforms.has('linux')) {
    const appImage = names.find((name) => name.endsWith('_amd64.AppImage'));
    const deb = names.find((name) => name.endsWith('_amd64.deb'));
    const rpm = names.find((name) => name.endsWith('.x86_64.rpm'));
    requireValue(
      appImage &&
        binaryKind(readFileSync(join(directory, appImage)).subarray(0, 1024)) === 'elf-x86_64',
      'Linux AppImage is absent or has the wrong architecture.'
    );
    requireValue(
      deb && binaryKind(readFileSync(join(directory, deb)).subarray(0, 1024)) === 'deb',
      'Linux DEB is absent or malformed.'
    );
    requireValue(
      rpm && binaryKind(readFileSync(join(directory, rpm)).subarray(0, 1024)) === 'rpm',
      'Linux RPM is absent or malformed.'
    );
  }
  console.log(
    `Release assets verified: ${tag}, platforms=${[...platforms].sort().join(',')}, files=${names.length}.`
  );
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [directory, tag, platforms = 'windows,linux', repository] = process.argv.slice(2);
  requireValue(
    directory && tag,
    'Usage: verify-release-assets.mjs <directory> <vMAJOR.MINOR.PATCH> [windows,linux,macos] [owner/repo]'
  );
  verifyReleaseAssets(resolve(directory), tag, platforms, repository);
}
