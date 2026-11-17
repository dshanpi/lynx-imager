import { createHash } from 'node:crypto';
import { readFileSync, statSync } from 'node:fs';
import { spawnSync } from 'node:child_process';

function requireValue(condition, message) {
  if (!condition) throw new Error(message);
}

function members(version) {
  const installer = `LYNX_${version}_x64-setup.exe`;
  return [installer, `${installer}.sig`, 'README.txt', 'updater-public.minisign', 'SHA256SUMS.txt'];
}

export function validateWindowsDeliveryZipPaths(paths, version) {
  requireValue(/^\d+\.\d+\.\d+$/.test(version), 'ZIP version must be stable semantic version.');
  requireValue(
    JSON.stringify([...paths].sort()) === JSON.stringify(members(version).sort()),
    'Windows delivery ZIP must contain exactly the reviewed flat installation file allowlist.'
  );
}

function sevenZip(args, binary = false) {
  const result = spawnSync('7z', args, {
    encoding: binary ? null : 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
    timeout: 60_000,
    maxBuffer: 128 * 1024 * 1024,
  });
  requireValue(result.status === 0, 'Windows delivery ZIP read/integrity check failed.');
  return result.stdout;
}

export function verifyWindowsDeliveryZip(
  archive,
  { version, installerPath, signaturePath, publicKey }
) {
  const listing = sevenZip(['l', '-ba', '-slt', archive]).replaceAll('\r\n', '\n');
  const paths = [...listing.matchAll(/^Path = (.+)$/gm)].map((match) => match[1].trimEnd());
  validateWindowsDeliveryZipPaths(paths, version);
  requireValue(!/^(Symbolic Link|Hard Link) = /m.test(listing), 'ZIP links are forbidden.');
  const sizes = [...listing.matchAll(/^Size = (\d+)$/gm)].map((match) => Number(match[1]));
  const maxBytes = statSync(installerPath).size + 1024 * 1024;
  requireValue(
    sizes.length === paths.length && sizes.reduce((sum, size) => sum + size, 0) <= maxBytes,
    'Windows delivery ZIP expanded size exceeds the reviewed installer and metadata limit.'
  );
  sevenZip(['t', archive]);
  const contents = new Map(
    paths.map((name) => [name, sevenZip(['e', '-so', archive, name], true)])
  );
  const installer = `LYNX_${version}_x64-setup.exe`;
  requireValue(
    contents.get(installer).equals(readFileSync(installerPath)),
    'ZIP installer bytes differ from the reviewed signed full installer.'
  );
  requireValue(
    contents.get(`${installer}.sig`).equals(readFileSync(signaturePath)),
    'ZIP installer signature differs from the reviewed signature.'
  );
  requireValue(
    contents.get('updater-public.minisign').toString('utf8').trim() === publicKey.trim(),
    'ZIP public key differs from the application updater key.'
  );
  const checked = new Set();
  for (const line of contents.get('SHA256SUMS.txt').toString('utf8').trimEnd().split('\n')) {
    const match = line.match(/^([0-9a-f]{64})  ([^/\\]+)$/);
    requireValue(
      match && match[2] !== 'SHA256SUMS.txt' && contents.has(match[2]) && !checked.has(match[2]),
      'ZIP inner checksum entry is malformed, unexpected or duplicated.'
    );
    requireValue(
      createHash('sha256').update(contents.get(match[2])).digest('hex') === match[1],
      'ZIP inner file checksum mismatch.'
    );
    checked.add(match[2]);
  }
  requireValue(checked.size === paths.length - 1, 'ZIP inner checksums do not cover all files.');
}
