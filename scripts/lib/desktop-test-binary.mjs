import assert from 'node:assert/strict';
import { constants } from 'node:fs';
import { copyFile, readFile, chmod } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import path from 'node:path';
const sha256 = (bytes) => createHash('sha256').update(bytes).digest('hex');
// Isolated Linux test binary only. Keep resources beside the executable.
export async function createImmutableDesktopTestBinary(source, namespace) {
  assert.ok(path.isAbsolute(source));
  assert.match(namespace, /^[a-zA-Z0-9_-]{1,100}$/);
  const expected = sha256(await readFile(source));
  const binary = path.join(path.dirname(source), path.basename(source) + '-test-' + namespace);
  await copyFile(source, binary, constants.COPYFILE_EXCL);
  await chmod(binary, 0o755);
  const actual = sha256(await readFile(binary));
  assert.equal(actual, expected, 'Build changed while snapshotting; no application launched');
  return { binary, sha256: actual };
}
