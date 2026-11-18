// Generate a GitHub-hosted updater manifest only from locally verified installer bytes.
import assert from 'node:assert/strict';
import { createHash, createPublicKey, verify } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';
import { basename, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const [tag, directory, windowsFile, linuxFile] = process.argv.slice(2);
assert(
  directory && windowsFile && linuxFile,
  'Expected tag, directory, NSIS filename and deb filename'
);
const config = JSON.parse(await readFile(join(root, 'src-tauri/tauri.conf.json'), 'utf8'));
const repo = 'dshanpi/lynx-imager';
assert.equal(config.identifier, 'com.dshanpi.lynximager');
assert.deepEqual(config.plugins.updater.endpoints, [
  `https://raw.githubusercontent.com/${repo}/main/updates/latest.json`,
]);
assert.match(tag, /^v\d+\.\d+(?:\.\d+)?(?:-[a-z]+\.?\d+)?$/);
const normalized = tag
  .slice(1)
  .replace(/^(\d+\.\d+)(?=-|$)/, '$1.0')
  .replace(/-rc(\d+)$/, '-rc.$1');
assert.equal(normalized, config.version);
const lines = (base64) => Buffer.from(base64, 'base64').toString('utf8').trimEnd().split(/\r?\n/);
const key = Buffer.from(lines(config.plugins.updater.pubkey)[1], 'base64');
assert.equal(key.length, 42);
const pub = createPublicKey({
  key: Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), key.subarray(10)]),
  format: 'der',
  type: 'spki',
});
const platforms = {};
const sums = [];
for (const [target, name, extension] of [
  ['windows-x86_64', windowsFile, '.exe'],
  ['linux-x86_64', linuxFile, '.deb'],
]) {
  assert.equal(basename(name), name);
  assert(name.endsWith(extension));
  const data = await readFile(join(directory, name));
  if (extension === '.deb') assert.equal(data.subarray(0, 8).toString(), '!<arch>\n');
  else assert.equal(data.subarray(0, 2).toString(), 'MZ');
  const signature = (await readFile(join(directory, name + '.sig'), 'utf8')).trim();
  const [, raw, comment, globalSig] = lines(signature);
  const sig = Buffer.from(raw, 'base64');
  assert.equal(sig.length, 74);
  assert(sig.subarray(2, 10).equals(key.subarray(2, 10)), 'Wrong Imager signing key');
  const algorithm = sig.subarray(0, 2).toString();
  assert(['Ed', 'ED'].includes(algorithm));
  const message = algorithm === 'Ed' ? data : createHash('blake2b512').update(data).digest();
  assert(verify(null, message, pub, sig.subarray(10)), 'Invalid installer signature');
  assert(comment.startsWith('trusted comment: '));
  assert(
    verify(
      null,
      Buffer.concat([sig.subarray(10), Buffer.from(comment.slice(17))]),
      pub,
      Buffer.from(globalSig, 'base64')
    ),
    'Invalid trusted comment signature'
  );
  platforms[target] = {
    url: `https://github.com/${repo}/releases/download/${tag}/${encodeURIComponent(name)}`,
    signature,
  };
  sums.push(createHash('sha256').update(data).digest('hex') + '  ' + name);
}
const manifest = {
  version: config.version,
  notes: `LYNX Imager ${tag}。完整变更见 GitHub Release。`,
  platforms,
};
await writeFile(join(directory, 'latest.json'), JSON.stringify(manifest, null, 2) + '\n');
await writeFile(join(directory, 'SHA256SUMS.txt'), sums.join('\n') + '\n');
console.log('Verified both installer signatures with the embedded public key.');
