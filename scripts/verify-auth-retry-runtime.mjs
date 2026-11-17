import { ensureDesktopIdle } from './lib/desktop-test-safety.mjs';
// Real Tauri OAuth lifecycle check. Uses an isolated profile and loopback callback.
// No browser authorization, token exchange, course writes, or hardware operations.
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, readFile, writeFile } from 'node:fs/promises';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';

const root = process.cwd();
const out = process.env.LYNX_AUTH_TEST_OUTPUT;
assert.ok(
  out && path.isAbsolute(out),
  'LYNX_AUTH_TEST_OUTPUT must be an absolute evidence directory'
);
const mode = process.env.LYNX_AUTH_TEST_MODE || 'cancel';
assert.ok(['cancel', 'replace'].includes(mode));
const port = Number(process.env.LYNX_AUTH_TEST_DRIVER_PORT || 4482);
const nativePort = Number(process.env.LYNX_AUTH_TEST_NATIVE_PORT || 4481);
const callbackPort = Number(process.env.LYNX_AUTH_TEST_CALLBACK_PORT || 48932);
for (const p of [port, nativePort, callbackPort])
  assert.ok(Number.isInteger(p) && p > 1024 && p < 65536);
assert.equal(new Set([port, nativePort, callbackPort]).size, 3);
const binary =
  process.env.LYNX_AUTH_TEST_BINARY || path.join(root, 'src-tauri/target/debug/lynx-app');
await mkdir(out, { recursive: true });
const profile = await mkdtemp(path.join(out, 'profile-'));
const result = {
  mode,
  head: execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim(),
  binarySha256: createHash('sha256')
    .update(await readFile(binary))
    .digest('hex'),
  oauthAuthorizationRequests: false,
  checks: [],
};
const driver = spawn(
  path.join(os.homedir(), '.cargo/bin/tauri-driver'),
  ['--port', String(port), '--native-port', String(nativePort)],
  {
    cwd: root,
    env: {
      ...process.env,
      XDG_CONFIG_HOME: profile + '/config',
      XDG_DATA_HOME: profile + '/data',
      XDG_CACHE_HOME: profile + '/cache',
      XDG_STATE_HOME: profile + '/state',
      LYNX_TEST_ALLOW_PARALLEL_INSTANCE: '1',
      LYNX_TEST_AUTH_KEYRING_NAMESPACE: path.basename(profile),
      ASK100_REDIRECT_URI: `http://127.0.0.1:${callbackPort}/auth/100ask/callback`,
    },
    stdio: ['ignore', 'inherit', 'inherit'],
  }
);
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
async function req(method, route, body) {
  const r = await fetch(`http://127.0.0.1:${port}${route}`, {
    method,
    headers: { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(40000),
  });
  const v = await r.json();
  if (!r.ok || v.value?.error) throw Error(JSON.stringify(v));
  return v.value;
}
let session;
async function ipc(command, args = {}) {
  const r = await req('POST', `/session/${session}/execute/async`, {
    script:
      'const done=arguments[arguments.length-1];window.__TAURI_INTERNALS__.invoke(arguments[0],arguments[1]).then(value=>done({ok:true,value})).catch(error=>done({ok:false,error:String(error)}));',
    args: [command, args],
  });
  if (!r.ok) throw Error(r.error);
  return r.value;
}
async function script(source) {
  return req('POST', `/session/${session}/execute/sync`, { script: source, args: [] });
}
function listening() {
  return new Promise((resolve) => {
    const socket = net.connect({ host: '127.0.0.1', port: callbackPort });
    socket.once('connect', () => {
      socket.destroy();
      resolve(true);
    });
    socket.once('error', () => resolve(false));
    socket.setTimeout(1500, () => {
      socket.destroy();
      resolve(false);
    });
  });
}
function check(name, actual, expected) {
  result.checks.push({ name, actual, expected, pass: actual === expected });
}
try {
  assert.equal(await listening(), false, 'callback port must be unused');
  for (let i = 0; i < 100; i++) {
    try {
      await req('GET', '/status');
      break;
    } catch {
      await delay(100);
    }
  }
  const created = await req('POST', '/session', {
    capabilities: { alwaysMatch: { 'tauri:options': { application: binary } } },
  });
  session = created.sessionId || created.id;
  await writeFile(
    path.join(out, 'control.json'),
    JSON.stringify({
      session,
      port,
      nativePort,
      callbackPort,
      profile,
      driverPid: driver.pid,
      wrapperPid: process.pid,
    })
  );
  await script(
    'window.__authTimeoutMessages=[];window.alert=(message)=>window.__authTimeoutMessages.push(String(message));return true;'
  );
  const start = performance.now();
  await ipc('auth_login_start');
  check('first listener active', await listening(), true);
  await delay(20000);
  if (mode === 'cancel') {
    await ipc('auth_cancel_login');
    check('cancel releases first listener', await listening(), false);
  }
  await ipc('auth_login_start');
  const retry = performance.now();
  check('retry listener active', await listening(), true);
  console.log('Waiting for the actual original 300-second deadline.');
  await delay(Math.max(0, 303000 - (performance.now() - start)));
  result.retryElapsedAtOldDeadlineSeconds = (performance.now() - retry) / 1000;
  check('observation precedes retry deadline', result.retryElapsedAtOldDeadlineSeconds < 295, true);
  check('old timeout preserves retry listener', await listening(), true);
  const oldMessages = await script('return window.__authTimeoutMessages;');
  check('old timeout emits no retry error', oldMessages.length, 0);
  await delay(Math.max(0, 303000 - (performance.now() - retry)));
  check('current timeout closes its listener', await listening(), false);
  const messages = await script('return window.__authTimeoutMessages;');
  check('current timeout emits exactly one error', messages.length, 1);
  check(
    'current error is login timeout',
    messages.length === 1 && messages[0].includes('登录超时'),
    true
  );
  await ipc('auth_login_start');
  check('login can restart after timeout', await listening(), true);
  await ipc('auth_cancel_login');
  check('explicit cancel releases restarted listener', await listening(), false);
  check('no authenticated session was created', (await ipc('auth_get_user')) === null, true);
} catch (error) {
  result.error = String(error);
} finally {
  if (session) {
    try {
      await ipc('auth_cancel_login');
      result.desktopIdle = await ensureDesktopIdle(ipc);
      result.hardwareSafety = result.desktopIdle.hardware;
      if (result.desktopIdle) {
        await req('DELETE', `/session/${session}`);
        driver.kill('SIGTERM');
        result.cleanedUp = true;
      }
    } catch (error) {
      result.cleanupError = String(error);
    }
  }
  result.pass = !result.error && result.cleanedUp === true && result.checks.every((c) => c.pass);
  await writeFile(path.join(out, 'result.json'), JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result));
  if (!result.pass) process.exitCode = 1;
}
