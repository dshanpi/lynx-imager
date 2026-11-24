import assert from 'node:assert/strict';
import { generateKeyPairSync, randomBytes, sign } from 'node:crypto';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import test from 'node:test';

test('manual macOS signature rejects tampered bytes, wrong keys and edited comments', () => {
  const dir = mkdtempSync(join(tmpdir(), 'imager-signature-'));
  try {
    const { privateKey, publicKey } = generateKeyPairSync('ed25519');
    const keyId = randomBytes(8);
    const data = Buffer.from('manual distribution fixture, not a real DMG');
    const raw = sign(null, data, privateKey);
    const comment = 'timestamp:1700000000';
    const base64Text = (lines) => Buffer.from(lines.join('\n') + '\n').toString('base64');
    const key = Buffer.concat([
      Buffer.from('Ed'),
      keyId,
      publicKey.export({ type: 'spki', format: 'der' }).subarray(-32),
    ]);
    const signature = [
      'untrusted comment: test only',
      Buffer.concat([Buffer.from('Ed'), keyId, raw]).toString('base64'),
      'trusted comment: ' + comment,
      sign(null, Buffer.concat([raw, Buffer.from(comment)]), privateKey).toString('base64'),
    ];
    const asset = join(dir, 'candidate.dmg');
    const sig = asset + '.sig';
    const pub = join(dir, 'key.pub');
    writeFileSync(asset, data);
    writeFileSync(sig, base64Text(signature));
    writeFileSync(pub, base64Text(['untrusted comment: public key', key.toString('base64')]));
    const check = () =>
      spawnSync(process.execPath, ['scripts/verify-macos-manual-signature.mjs', asset, sig, pub], {
        encoding: 'utf8',
      });
    assert.equal(check().status, 0);
    writeFileSync(asset, Buffer.concat([data, Buffer.from('tampered')]));
    assert.notEqual(check().status, 0);
    writeFileSync(asset, data);
    const edited = [...signature];
    edited[2] += ' changed';
    writeFileSync(sig, base64Text(edited));
    assert.notEqual(check().status, 0);
    writeFileSync(sig, base64Text(signature));
    key[3] ^= 1;
    writeFileSync(pub, base64Text(['untrusted comment: public key', key.toString('base64')]));
    assert.notEqual(check().status, 0);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
