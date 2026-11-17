import { createHash } from 'node:crypto';
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { basename, join, relative } from 'node:path';

const [
  inputRoot,
  outputRoot,
  repository,
  tag,
  platformList = 'windows,linux,macos',
  notesFile,
  deltaFromVersion,
  storageMode = 'site-full',
] = process.argv.slice(2);
if (
  !inputRoot ||
  !outputRoot ||
  !repository ||
  !/^v\d+\.\d+(?:\.\d+)?(?:-[0-9A-Za-z]+(?:[.-][0-9A-Za-z]+)*)?$/.test(tag || '')
) {
  throw new Error(
    'Usage: node prepare-release-assets.mjs <input> <output> <owner/repo> ' +
      '<vMAJOR.MINOR[.PATCH][-PRERELEASE]> [windows,linux,macos] [notes-file] ' +
      '[windows-delta-from-version] [site-full|external-full|github-full]'
  );
}
if (!['site-full', 'external-full', 'github-full'].includes(storageMode)) {
  throw new Error(`Invalid download-site storage mode: ${storageMode}`);
}
const supportedPlatforms = new Set(['windows', 'linux', 'macos']);
const requestedPlatforms = new Set(platformList.split(',').filter(Boolean));
if (
  !requestedPlatforms.size ||
  [...requestedPlatforms].some((platform) => !supportedPlatforms.has(platform))
) {
  throw new Error(`Invalid release platform list: ${platformList}`);
}
const numericTag = tag.slice(1);
const [releaseCore, releasePrerelease] = numericTag.split('-', 2);
const normalizedCore = releaseCore.split('.').length === 2 ? `${releaseCore}.0` : releaseCore;
const releaseVersion = releasePrerelease
  ? `${normalizedCore}-${releasePrerelease}`
  : normalizedCore;
const stableVersionPattern = /^(?:v)?(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
const deltaFromMatch = deltaFromVersion?.match(stableVersionPattern);
if (deltaFromVersion && (!deltaFromMatch || releasePrerelease)) {
  throw new Error('Windows delta releases require stable MAJOR.MINOR.PATCH versions.');
}
const normalizedDeltaFrom = deltaFromMatch
  ? `${deltaFromMatch[1]}.${deltaFromMatch[2]}.${deltaFromMatch[3]}`
  : undefined;
if (normalizedDeltaFrom) {
  const from = normalizedDeltaFrom.split('.').map(Number);
  const to = releaseVersion.split('.').map(Number);
  const newer = to.some(
    (part, index) =>
      part > from[index] && to.slice(0, index).every((value, prior) => value === from[prior])
  );
  if (!newer || from[0] !== to[0] || (to[0] === 0 && from[1] !== to[1])) {
    throw new Error(`Incompatible delta versions: ${normalizedDeltaFrom} -> ${releaseVersion}`);
  }
}
const releaseNotes =
  notesFile && existsSync(notesFile)
    ? readFileSync(notesFile, 'utf8').trim()
    : `DshanPI LYNX｜芯桥 ${tag}`;

function walk(path) {
  return readdirSync(path).flatMap((name) => {
    const child = join(path, name);
    return statSync(child).isDirectory() ? walk(child) : [child];
  });
}

function platformOf(path) {
  const normalized = relative(inputRoot, path).replaceAll('\\', '/').toLowerCase();
  if (normalized.startsWith('windows/')) return 'windows';
  if (normalized.startsWith('linux/')) return 'linux';
  if (normalized.startsWith('macos/')) return 'macos';
  return undefined;
}

function isReleaseAsset(path) {
  const name = basename(path);
  const platform = platformOf(path);
  if (!platform || !requestedPlatforms.has(platform)) return false;
  if (platform === 'windows') {
    return (
      /\.(exe|msi)(\.sig)?$/i.test(name) ||
      name === `LYNX_${releaseVersion}_Windows_x64.zip` ||
      name === `LYNX_${releaseVersion}_Windows_x64.zip.sig` ||
      name === `LYNX_${releaseVersion}_Windows_x64_github-update.zip` ||
      name === `LYNX_${releaseVersion}_Windows_x64_github-update.zip.sig`
    );
  }
  if (platform === 'linux') return /\.(AppImage|deb|rpm)(\.sig)?$/i.test(name);
  if (platform === 'macos') return /\.dmg$/i.test(name) || /\.app\.tar\.gz(\.sig)?$/i.test(name);
  return false;
}

function publishedName(path) {
  const name = basename(path);
  if (platformOf(path) === 'macos' && /\.app\.tar\.gz(?:\.sig)?$/i.test(name)) {
    const signatureSuffix = name.toLowerCase().endsWith('.sig') ? '.sig' : '';
    return `LYNX_${releaseVersion}_aarch64.app.tar.gz${signatureSuffix}`;
  }
  return name;
}

function updaterPair(files, platform) {
  const candidates = files.filter(
    (path) =>
      platformOf(path) === platform &&
      path.toLowerCase().endsWith('.sig') &&
      !basename(path).toLowerCase().includes('-delta.')
  );
  const signature =
    platform === 'windows'
      ? candidates.find(
          (path) => basename(path) === `LYNX_${releaseVersion}_github-update_x64-setup.exe.sig`
        ) || candidates.find((path) => path.toLowerCase().endsWith('.exe.sig'))
      : platform === 'linux'
        ? candidates.find((path) => path.endsWith('.AppImage.sig'))
        : candidates.find((path) => path.endsWith('.app.tar.gz.sig'));
  if (!signature) throw new Error(`Missing signed updater artifact for ${platform}.`);
  const artifact = signature.slice(0, -4);
  if (!existsSync(artifact)) {
    throw new Error(`Updater signature has no matching artifact: ${signature}`);
  }
  return { artifact, signature };
}

function windowsDeltaPair(files) {
  if (storageMode === 'github-full') return undefined;
  const candidates = files.filter(
    (path) =>
      platformOf(path) === 'windows' &&
      path.toLowerCase().endsWith('.exe.sig') &&
      basename(path).toLowerCase().includes('-delta.')
  );
  if (!normalizedDeltaFrom) {
    if (candidates.length) {
      throw new Error('A delta artifact exists but windows-delta-from-version was not provided.');
    }
    return undefined;
  }
  if (!requestedPlatforms.has('windows') || candidates.length !== 1) {
    throw new Error('Exactly one signed Windows delta artifact is required.');
  }
  const signature = candidates[0];
  const artifact = signature.slice(0, -4);
  if (!existsSync(artifact)) {
    throw new Error(`Delta signature has no matching artifact: ${signature}`);
  }
  const escapedFrom = normalizedDeltaFrom.replaceAll('.', '\\.');
  const escapedTo = releaseVersion.replaceAll('.', '\\.');
  const expectedName = new RegExp(`^LYNX_${escapedTo}_from_${escapedFrom}_x64-delta\\.exe$`, 'i');
  if (!expectedName.test(basename(artifact))) {
    throw new Error(
      `Delta filename must be LYNX_${releaseVersion}_from_${normalizedDeltaFrom}_x64-delta.exe`
    );
  }
  return { artifact, signature };
}

function assetUrl(path) {
  return (
    `https://github.com/${repository}/releases/download/${encodeURIComponent(tag)}/` +
    encodeURIComponent(publishedName(path))
  );
}

const files = walk(inputRoot);
const selected = files.filter(isReleaseAsset);
if (!selected.length) throw new Error(`No release assets found below ${inputRoot}.`);

rmSync(outputRoot, { recursive: true, force: true });
mkdirSync(outputRoot, { recursive: true });
const names = new Set();
for (const source of selected) {
  const name = publishedName(source);
  if (names.has(name)) throw new Error(`Duplicate release asset name: ${name}`);
  names.add(name);
  copyFileSync(source, join(outputRoot, name));
}

const entry = ({ artifact, signature }) => ({
  signature: readFileSync(signature, 'utf8').trim(),
  url: assetUrl(artifact),
});
const releaseEntry = (target, pair, kind = 'full', fromVersion) => {
  const data = readFileSync(pair.artifact);
  return {
    target,
    kind,
    storage: kind === 'delta' || storageMode === 'site-full' ? 'site' : 'external',
    ...(fromVersion ? { from_version: fromVersion } : {}),
    file: publishedName(pair.artifact),
    size: data.length,
    sha256: createHash('sha256').update(data).digest('hex'),
    signature: readFileSync(pair.signature, 'utf8').trim(),
    mirrors: [assetUrl(pair.artifact)],
  };
};
const manifestPlatforms = {};
const releaseAssets = [];
let windowsDelta;
if (requestedPlatforms.has('windows')) {
  const windowsPair = updaterPair(files, 'windows');
  const deltaPair = windowsDeltaPair(files);
  const windowsEntry = entry(windowsPair);
  manifestPlatforms['windows-x86_64'] = windowsEntry;
  manifestPlatforms['windows-x86_64-nsis'] = windowsEntry;
  releaseAssets.push(releaseEntry('windows-x86_64', windowsPair));
  releaseAssets.push(releaseEntry('windows-x86_64-nsis', windowsPair));
  if (deltaPair) {
    windowsDelta = deltaPair;
    releaseAssets.push(releaseEntry('windows-x86_64', deltaPair, 'delta', normalizedDeltaFrom));
    releaseAssets.push(
      releaseEntry('windows-x86_64-nsis', deltaPair, 'delta', normalizedDeltaFrom)
    );
  }
}
if (requestedPlatforms.has('linux')) {
  const linuxPair = updaterPair(files, 'linux');
  manifestPlatforms['linux-x86_64'] = entry(linuxPair);
  releaseAssets.push(releaseEntry('linux-x86_64', linuxPair));
}
if (requestedPlatforms.has('macos')) {
  const macosPair = updaterPair(files, 'macos');
  manifestPlatforms['darwin-aarch64'] = entry(macosPair);
  releaseAssets.push(releaseEntry('darwin-aarch64', macosPair));
}
const publishedAt = new Date().toISOString();
const manifest = {
  version: releaseVersion,
  notes: releaseNotes,
  pub_date: publishedAt,
  platforms: manifestPlatforms,
};
if (storageMode === 'github-full') {
  const full = releaseAssets.find(
    (asset) => asset.target === 'windows-x86_64' && asset.kind === 'full'
  );
  if (!full) throw new Error('GitHub full strategy requires a Windows signed full package.');
  Object.assign(manifest, { strategy: 'full', asset_size: full.size, asset_sha256: full.sha256 });
}
writeFileSync(join(outputRoot, 'latest.json'), `${JSON.stringify(manifest, null, 2)}\n`);
if (storageMode === 'github-full' && normalizedDeltaFrom) {
  writeFileSync(
    join(outputRoot, `update-windows-x86_64-from-${normalizedDeltaFrom}.json`),
    `${JSON.stringify(manifest, null, 2)}\n`
  );
}
if (windowsDelta) {
  const deltaEntry = entry(windowsDelta);
  const deltaManifest = {
    version: releaseVersion,
    notes: releaseNotes,
    pub_date: publishedAt,
    strategy: 'delta',
    asset_size: statSync(windowsDelta.artifact).size,
    asset_sha256: createHash('sha256').update(readFileSync(windowsDelta.artifact)).digest('hex'),
    fallback_available: true,
    platforms: {
      'windows-x86_64': deltaEntry,
      'windows-x86_64-nsis': deltaEntry,
    },
  };
  const deltaMetadataName = `update-windows-x86_64-from-${normalizedDeltaFrom}.json`;
  writeFileSync(join(outputRoot, deltaMetadataName), `${JSON.stringify(deltaManifest, null, 2)}\n`);
}
const releaseSet = {
  schema_version: 2,
  product: 'lynx',
  channel: releasePrerelease ? 'beta' : 'stable',
  version: releaseVersion,
  published_at: publishedAt,
  notes: releaseNotes,
  assets: releaseAssets,
};
writeFileSync(join(outputRoot, 'release-set.json'), `${JSON.stringify(releaseSet, null, 2)}\n`);

const checksumFiles = readdirSync(outputRoot).sort();
const checksums = checksumFiles.map((name) => {
  const digest = createHash('sha256')
    .update(readFileSync(join(outputRoot, name)))
    .digest('hex');
  return `${digest}  ${name}`;
});
writeFileSync(join(outputRoot, 'SHA256SUMS.txt'), `${checksums.join('\n')}\n`);

console.log(`Prepared ${selected.length} release assets and signed metadata for ${tag}.`);
