import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const [mode, tag, dir, windows, linux] = process.argv.slice(2);
assert(
  ['stage', 'publish'].includes(mode) && linux,
  'Expected stage|publish, tag, directory, exe, deb'
);
const root = fileURLToPath(new URL('../', import.meta.url));
const git = (...args) => execFileSync('git', args, { cwd: root, encoding: 'utf8' }).trim();
const repo = 'dshanpi/lynx-imager';
assert(
  /^(?:https:\/\/github\.com\/|git@github\.com:)dshanpi\/lynx-imager(?:\.git)?$/.test(
    git('remote', 'get-url', 'origin')
  )
);
execFileSync(process.execPath, [join(root, 'scripts/validate-release-version.mjs'), tag], {
  stdio: 'inherit',
});
execFileSync(
  process.execPath,
  [join(root, 'scripts/imager-ota-feed.mjs'), tag, dir, windows, linux],
  { stdio: 'inherit' }
);
const commit = git('rev-parse', 'HEAD');
assert.equal(
  git('ls-remote', 'origin', 'refs/heads/main').split(/\s/)[0],
  commit,
  'Push reviewed release sources to main first'
);
assert.equal(git('status', '--porcelain'), '', 'Release source tree must be clean');
let token = process.env.GH_TOKEN || process.env.GITHUB_TOKEN;
if (!token) {
  const answer = execFileSync('git', ['-c', 'credential.interactive=false', 'credential', 'fill'], {
    input: 'protocol=https\nhost=github.com\n\n',
    encoding: 'utf8',
    env: { ...process.env, GIT_TERMINAL_PROMPT: '0', GCM_INTERACTIVE: 'never' },
    stdio: ['pipe', 'pipe', 'ignore'],
  });
  token = answer
    .split(/\r?\n/)
    .find((x) => x.startsWith('password='))
    ?.slice(9);
}
assert(token, 'GitHub release credential unavailable');
const headers = {
  Authorization: 'Bearer ' + token,
  Accept: 'application/vnd.github+json',
  'User-Agent': 'LYNX-Imager-Publisher',
};
async function api(path, method = 'GET', body) {
  const response = await fetch('https://api.github.com/repos/' + repo + path, {
    method,
    headers: { ...headers, 'Content-Type': 'application/json' },
    body: body ? JSON.stringify(body) : undefined,
  });
  assert(
    response.ok,
    `${method} ${path}: HTTP ${response.status} ${(await response.clone().text()).slice(0, 300)}`
  );
  return response.json();
}
let release = (await api('/releases?per_page=100')).find((r) => r.tag_name === tag);
if (!release) {
  assert.equal(mode, 'stage', 'Stage release first');
  release = await api('/releases', 'POST', {
    tag_name: tag,
    target_commitish: commit,
    name: 'LYNX Imager ' + tag,
    body: await readFile(join(root, 'docs/releases', tag + '.md'), 'utf8'),
    draft: true,
    prerelease: tag.includes('-'),
  });
}
assert(release.draft, 'Release is already public');
assert.equal(release.target_commitish, commit);
const files = [windows, linux, windows + '.sig', linux + '.sig', 'latest.json', 'SHA256SUMS.txt'];
assert(
  release.assets.every((a) => files.includes(a.name)),
  'Unexpected draft asset'
);
for (const name of files) {
  const bytes = await readFile(join(dir, name));
  const digest = 'sha256:' + createHash('sha256').update(bytes).digest('hex');
  let asset = release.assets.find((a) => a.name === name);
  if (!asset) {
    assert.equal(mode, 'stage', 'Missing staged asset ' + name);
    const url = new URL(release.upload_url.split('{')[0]);
    assert.equal(url.origin, 'https://uploads.github.com');
    url.searchParams.set('name', name);
    const response = await fetch(url, {
      method: 'POST',
      headers: { ...headers, 'Content-Type': 'application/octet-stream' },
      body: bytes,
    });
    assert(response.ok, 'Upload ' + name + ': HTTP ' + response.status);
    asset = await response.json();
  }
  assert.equal(asset.size, bytes.length, name);
  assert.equal(asset.digest, digest, name);
  console.log('Verified GitHub asset', name);
}
if (mode === 'publish')
  release = await api('/releases/' + release.id, 'PATCH', {
    draft: false,
    prerelease: tag.includes('-'),
    make_latest: tag.includes('-') ? 'false' : 'true',
  });
console.log(release.html_url);
