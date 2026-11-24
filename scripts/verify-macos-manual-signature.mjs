// Verify manual macOS distribution bytes. This key is not the Windows/Linux OTA root.
import assert from 'node:assert/strict';
import { createHash, createPublicKey, verify } from 'node:crypto';
import { readFile } from 'node:fs/promises';

const [asset, signatureFile, publicKeyFile] = process.argv.slice(2);
assert(asset && signatureFile && publicKeyFile, 'Expected asset, .sig and public-key files');
const lines = (text) =>
  Buffer.from(text.trim(), 'base64').toString('utf8').trimEnd().split(/\r?\n/);
const key = Buffer.from(lines(await readFile(publicKeyFile, 'utf8'))[1], 'base64');
assert.equal(key.length, 42);
const pub = createPublicKey({
  key: Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), key.subarray(10)]),
  format: 'der',
  type: 'spki',
});
const [, raw, comment, globalSignature] = lines(await readFile(signatureFile, 'utf8'));
const signature = Buffer.from(raw, 'base64');
assert.equal(signature.length, 74);
assert(signature.subarray(2, 10).equals(key.subarray(2, 10)), 'Wrong distribution signing key');
const algorithm = signature.subarray(0, 2).toString();
assert(['Ed', 'ED'].includes(algorithm), 'Unsupported signature algorithm');
const data = await readFile(asset);
const message = algorithm === 'Ed' ? data : createHash('blake2b512').update(data).digest();
assert(verify(null, message, pub, signature.subarray(10)), 'Invalid asset signature');
assert(comment.startsWith('trusted comment: '));
assert(
  verify(
    null,
    Buffer.concat([signature.subarray(10), Buffer.from(comment.slice(17))]),
    pub,
    Buffer.from(globalSignature, 'base64')
  ),
  'Invalid trusted comment'
);
console.log(
  'PASS: manual macOS distribution signature (not Apple notarization or OTA enrollment).'
);
