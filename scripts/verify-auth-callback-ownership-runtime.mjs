import { ensureDesktopIdle } from './lib/desktop-test-safety.mjs';
// Real Tauri loopback callback ownership check. It never opens the OAuth URL or exchanges a token.
import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, readFile, writeFile } from 'node:fs/promises';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';

const root = process.cwd();
const out = process.env.LYNX_AUTH_CALLBACK_TEST_OUTPUT;
assert.ok(out && path.isAbsolute(out), 'LYNX_AUTH_CALLBACK_TEST_OUTPUT must be absolute');
const driverPort = Number(process.env.LYNX_AUTH_CALLBACK_DRIVER_PORT || 4486);
const nativePort = Number(process.env.LYNX_AUTH_CALLBACK_NATIVE_PORT || 4485);
const callbackPort = Number(process.env.LYNX_AUTH_CALLBACK_PORT || 48934);
assert.equal(new Set([driverPort, nativePort, callbackPort]).size, 3);
const binary = process.env.LYNX_AUTH_CALLBACK_BINARY || path.join(root, 'src-tauri/target/debug/lynx-imager');
await mkdir(out, { recursive: true });
const profile = await mkdtemp(path.join(out, 'profile-'));
const result = {
  head: execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim(),
  binarySha256: createHash('sha256').update(await readFile(binary)).digest('hex'),
  oauthAuthorizationRequests: false,
  tokenExchangeRequests: false,
  checks: [],
};
const driver = spawn(
  path.join(os.homedir(), '.cargo/bin/tauri-driver'),
  ['--port', String(driverPort), '--native-port', String(nativePort)],
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
  },
);
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
async function req(method, route, body) {
  const response = await fetch(`http://127.0.0.1:${driverPort}${route}`, {
    method,
    headers: { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(40000),
  });
  const value = await response.json();
  if (!response.ok || value.value?.error) throw Error(JSON.stringify(value));
  return value.value;
}
let session;
async function ipc(command, args = {}) {
  const response = await req('POST', `/session/${session}/execute/async`, {
    script:
      'const done=arguments[arguments.length-1];window.__TAURI_INTERNALS__.invoke(arguments[0],arguments[1]).then(value=>done({ok:true,value})).catch(error=>done({ok:false,error:String(error)}));',
    args: [command, args],
  });
  if (!response.ok) throw Error(response.error);
  return response.value;
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
async function start() {
  const login = await ipc('auth_login_start');
  const state = new URL(login.authorizeUrl).searchParams.get('state');
  assert.ok(state && state.length >= 16);
  check('callback listener active', await listening(), true);
  return state;
}
async function callback(query) {
  const response = await fetch(
    `http://127.0.0.1:${callbackPort}/auth/100ask/callback?${new URLSearchParams(query)}`,
    { signal: AbortSignal.timeout(10000) },
  );
  check('callback response is bounded HTML', response.status === 200 && (await response.text()).length < 4096, true);
  await delay(250);
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
    JSON.stringify({ session, driverPort, nativePort, callbackPort, profile, driverPid: driver.pid }),
  );
  await script('window.__authCallbackMessages=[];window.alert=(message)=>window.__authCallbackMessages.push(String(message));return true;');

  let state = await start();
  await callback({ code: 'never-exchanged', state: 'wrong-state-for-local-test' });
  check('wrong state preserves current listener', await listening(), true);
  check('wrong state emits no terminal event', (await script('return window.__authCallbackMessages.length;')), 0);
  if (!(await listening())) state = await start();
  await callback({ state });
  check('missing code preserves current listener', await listening(), true);
  check('missing code emits no terminal event', (await script('return window.__authCallbackMessages.length;')), 0);
  if (!(await listening())) state = await start();
  await callback({ error: 'access_denied', state: 'wrong-error-state' });
  check('unowned error preserves current listener', await listening(), true);
  check('unowned error emits no terminal event', (await script('return window.__authCallbackMessages.length;')), 0);
  if (!(await listening())) state = await start();
  await callback({ error: 'access_denied', state });
  check('owned denial closes current listener', await listening(), false);
  const messages = await script('return window.__authCallbackMessages;');
  check('owned denial emits one terminal error', messages.length, 1);
  check('terminal error describes authorization denial', messages.length === 1 && messages[0].includes('access_denied'), true);
  check('no authenticated session created', (await ipc('auth_get_user')) === null, true);
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
  result.pass = !result.error && result.cleanedUp === true && result.checks.every((check) => check.pass);
  await writeFile(path.join(out, 'result.json'), JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result));
  if (!result.pass) process.exitCode = 1;
}
