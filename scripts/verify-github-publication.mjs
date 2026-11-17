#!/usr/bin/env node
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { resolve, join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { verifyReleaseAssets, parseChecksums } from './verify-release-assets.mjs';

const repository = 'dshanpi/lynx-releases';
const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const feed = `https://github.com/${repository}/releases/latest/download/latest.json`;
const digest = (bytes) => 'sha256:' + createHash('sha256').update(bytes).digest('hex');

export function validatePublication({ tag, expected, release, latest, publicationChecksums }) {
  assert.match(tag, /^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/);
  assert.ok(expected.length, 'Reviewed asset set is empty.');
  for (const value of [release, latest]) {
    assert.equal(value.tag_name, tag, 'Public target/Latest tag differs.');
    assert.equal(value.draft, false, 'Stable release is draft.');
    assert.equal(value.prerelease, false, 'Stable release is prerelease.');
  }
  assert.ok(Number.isSafeInteger(release.id) && release.id > 0);
  assert.equal(latest.id, release.id, 'Latest does not identify the intended release.');
  const actual = new Map(release.assets.map((a) => [a.name, a]));
  assert.equal(actual.size, release.assets.length, 'Duplicate public asset names.');
  assert.ok(
    ![...actual.keys()].some((name) => name.includes('github-update')),
    'Special repair names are not normal public delivery.'
  );
  assert.equal(
    new Set(expected.map((a) => a.name)).size,
    expected.length,
    'Duplicate reviewed names.'
  );
  if (publicationChecksums) {
    const checksums = parseChecksums(publicationChecksums.toString());
    assert.equal(
      checksums.size,
      actual.size - 1,
      'Public checksum list must cover every non-checksum asset.'
    );
    for (const asset of actual.values()) {
      if (asset.name === 'SHA256SUMS.txt') continue;
      assert.equal(
        'sha256:' + checksums.get(asset.name),
        asset.digest,
        `Global checksum differs: ${asset.name}`
      );
    }
    const reviewed = expected.find((a) => a.name === 'SHA256SUMS.txt');
    assert.equal(
      reviewed.digest,
      digest(publicationChecksums),
      'Global checksum bytes were not reviewed.'
    );
  }
  for (const item of expected) {
    const asset = actual.get(item.name);
    assert.ok(asset, `Missing public asset: ${item.name}`);
    assert.match(item.digest, /^sha256:[0-9a-f]{64}$/);
    assert.equal(asset.digest, item.digest, `Public bytes differ: ${item.name}`);
    assert.ok(Number.isSafeInteger(item.size) && item.size > 0);
    assert.equal(asset.size, item.size, `Public size differs: ${item.name}`);
    assert.equal(
      asset.browser_download_url,
      `https://github.com/${repository}/releases/download/${tag}/${encodeURIComponent(item.name)}`,
      `Noncanonical URL: ${item.name}`
    );
  }
}

// API failures/invalid JSON must throw. No fallback to cached evidence or "latest".
export function readApi(path, runner = spawnSync) {
  const result = runner('gh', ['api', `repos/${repository}/${path}`], {
    encoding: 'utf8',
    timeout: 60000,
    maxBuffer: 8 * 1024 * 1024,
    env: { ...process.env, GH_HOST: 'github.com', GH_DEBUG: '' },
  });
  if (result.error || result.status !== 0)
    throw new Error('GitHub API unavailable; publication NOT VERIFIED.');
  return JSON.parse(result.stdout);
}

export function verifyPublication(directory, tag, publicChecksumPath) {
  const config = JSON.parse(readFileSync(join(repoRoot, 'src-tauri/tauri.conf.json')));
  const pkg = JSON.parse(readFileSync(join(repoRoot, 'package.json')));
  assert.deepEqual(
    config.plugins.updater.endpoints,
    [feed],
    'Only the canonical GitHub full feed is allowed.'
  );
  assert.equal(tag, 'v' + config.version, 'Source and intended release differ.');
  assert.equal(pkg.version, config.version, 'Source versions differ.');
  // Existing verifier checks unchanged-key signatures, checksums, ZIP members,
  // manifest/release-set versions, full routes, and the packaged NSIS marker.
  verifyReleaseAssets(directory, tag, 'windows', repository);
  const manifest = JSON.parse(readFileSync(join(directory, 'latest.json')));
  const name = `LYNX_${config.version}_x64-setup.exe`;
  assert.equal(
    manifest.strategy,
    'full',
    'GitHub static hosting cannot select a full fallback by header.'
  );
  for (const platform of ['windows-x86_64', 'windows-x86_64-nsis']) {
    assert.equal(
      manifest.platforms[platform].url,
      `https://github.com/${repository}/releases/download/${tag}/${name}`,
      'Active Windows feed must select the normal canonical full installer.'
    );
  }
  assert.ok(
    !readdirSync(directory).some((n) => n.includes('github-update')),
    'Special repair names are not normal delivery.'
  );
  const expected = readdirSync(directory).map((name) => {
    const path = join(directory, name);
    assert.ok(statSync(path).isFile(), 'Only a reviewed flat release directory is allowed.');
    const bytes = readFileSync(path);
    return { name, size: bytes.length, digest: digest(bytes) };
  });
  // Mixed releases retain unrelated historical Linux assets. Their global public
  // SHA list differs from the exact Windows staging list. Require the actual
  // reviewed global list, bind its bytes to the API, and check every public entry.
  const publicationChecksums = readFileSync(
    publicChecksumPath || join(directory, 'SHA256SUMS.txt')
  );
  const checksum = expected.find((a) => a.name === 'SHA256SUMS.txt');
  checksum.size = publicationChecksums.length;
  checksum.digest = digest(publicationChecksums);
  const release = readApi(`releases/tags/${tag}`);
  const latest = readApi('releases/latest');
  validatePublication({ tag, expected, release, latest, publicationChecksums });
  // Re-read the active feed identity to detect concurrent replacement during this check.
  const again = readApi('releases/latest');
  validatePublication({ tag, expected, release: again, latest: again, publicationChecksums });
  assert.equal(again.id, release.id);
  return {
    checkedAt: new Date().toISOString(),
    pass: true,
    tag,
    releaseId: release.id,
    reviewedAssets: expected.length,
    publishedAssets: release.assets.length,
    scope:
      'Signed staged files equal GitHub API asset digests and stable Latest. Client transport/install/restart are separate.',
  };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const [directory, tag, publicChecksumPath] = process.argv.slice(2);
    assert.ok(
      directory && tag && [4, 5].includes(process.argv.length),
      'Usage: node scripts/verify-github-publication.mjs <verified-flat-directory> <vMAJOR.MINOR.PATCH> [reviewed-global-SHA256SUMS.txt]'
    );
    console.log(
      JSON.stringify(verifyPublication(resolve(directory), tag, publicChecksumPath), null, 2)
    );
  } catch (error) {
    console.error('GitHub publication gate FAILED:', error.message);
    process.exitCode = 1;
  }
}
