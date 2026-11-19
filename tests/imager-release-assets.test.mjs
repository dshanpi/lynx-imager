import assert from 'node:assert/strict';
import { createHash, generateKeyPairSync, randomBytes, sign } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

const repository = 'dshanpi/lynx-imager';
const names = ['test-setup.exe', 'test.deb'];
const packages = [Buffer.from('MZ-fixture'), Buffer.from('!<arch>\nfixture')];

// Use an ephemeral signing key and an isolated copy of the shipping manifest generator.
async function setup(t) {
  const dir = await mkdtemp(join(tmpdir(), 'imager-ota-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  await mkdir(join(dir, 'src-tauri'));
  await mkdir(join(dir, 'scripts'));
  const script = join(dir, 'scripts', 'imager-ota-feed.mjs');
  await copyFile(new URL('../scripts/imager-ota-feed.mjs', import.meta.url), script);
  const { publicKey, privateKey } = generateKeyPairSync('ed25519');
  const keyId = randomBytes(8);
  const packet = (type, data) => Buffer.concat([Buffer.from(type), keyId, data]).toString('base64');
  const encode = (lines) => Buffer.from(lines.join('\n')).toString('base64');
  const pubkey = encode([
    'untrusted comment: fixture',
    packet('Ed', publicKey.export({ format: 'der', type: 'spki' }).subarray(-32)),
  ]);
  const config = JSON.parse(
    await readFile(new URL('../src-tauri/tauri.conf.json', import.meta.url))
  );
  config.version = '1.2.3';
  config.plugins.updater.pubkey = pubkey;
  await writeFile(join(dir, 'src-tauri', 'tauri.conf.json'), JSON.stringify(config));
  for (const [i, name] of names.entries()) {
    const type = i ? 'ED' : 'Ed';
    const bytes = packages[i];
    const sig = sign(null, i ? createHash('blake2b512').update(bytes).digest() : bytes, privateKey);
    const comment = 'timestamp:1';
    const trusted = sign(null, Buffer.concat([sig, Buffer.from(comment)]), privateKey);
    await writeFile(join(dir, name), bytes);
    await writeFile(
      join(dir, name + '.sig'),
      encode([
        'untrusted comment: fixture',
        packet(type, sig),
        'trusted comment: ' + comment,
        trusted.toString('base64'),
      ])
    );
  }
  return {
    dir,
    run: (tag = 'v1.2.3') =>
      spawnSync(process.execPath, [script, tag, dir, ...names], { encoding: 'utf8' }),
  };
}

test('signed installers produce the expected platform URLs, signatures and byte hashes', async (t) => {
  const { dir, run } = await setup(t);
  const result = run();
  assert.equal(result.status, 0, result.stderr);
  const feed = JSON.parse(await readFile(join(dir, 'latest.json')));
  assert.equal(feed.version, '1.2.3');
  for (const [i, platform] of ['windows-x86_64', 'linux-x86_64'].entries()) {
    assert.deepEqual(feed.platforms[platform], {
      url: 'https://github.com/' + repository + '/releases/download/v1.2.3/' + names[i],
      signature: await readFile(join(dir, names[i] + '.sig'), 'utf8'),
    });
  }
  const sums = names.map(
    (name, i) => createHash('sha256').update(packages[i]).digest('hex') + '  ' + name
  );
  assert.equal(await readFile(join(dir, 'SHA256SUMS.txt'), 'utf8'), sums.join('\n') + '\n');
});

for (const [i, name] of names.entries()) {
  test('rejects changed bytes in ' + name + ' before writing the feed', async (t) => {
    const { dir, run } = await setup(t);
    await writeFile(join(dir, name), Buffer.concat([packages[i], Buffer.from([1])]));
    const result = run();
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /Invalid installer signature/);
    await assert.rejects(readFile(join(dir, 'latest.json')), { code: 'ENOENT' });
  });
}

test('rejects a release tag that differs from the application version', async (t) => {
  const { dir, run } = await setup(t);
  assert.notEqual(run('v1.2.4').status, 0);
  await assert.rejects(readFile(join(dir, 'latest.json')), { code: 'ENOENT' });
});
