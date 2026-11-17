import { ensureDesktopIdle } from './lib/desktop-test-safety.mjs';
// Real Tauri cancellation check with a delayed, loopback-only OAuth API.
// It never opens the authorization URL or contacts the production OAuth service.
import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, readFile, writeFile } from 'node:fs/promises';
import http from 'node:http';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';

const root = process.cwd();
const out = process.env.LYNX_AUTH_INFLIGHT_OUTPUT;
assert.ok(out && path.isAbsolute(out), 'LYNX_AUTH_INFLIGHT_OUTPUT must be absolute');
const mode = process.env.LYNX_AUTH_INFLIGHT_MODE || 'cancel';
assert.ok(['cancel', 'timeout'].includes(mode));
const driverPort = Number(process.env.LYNX_AUTH_INFLIGHT_DRIVER_PORT || 4488);
const nativePort = Number(process.env.LYNX_AUTH_INFLIGHT_NATIVE_PORT || 4487);
const callbackPort = Number(process.env.LYNX_AUTH_INFLIGHT_CALLBACK_PORT || 48936);
const apiPort = Number(process.env.LYNX_AUTH_INFLIGHT_API_PORT || 48937);
assert.equal(new Set([driverPort, nativePort, callbackPort, apiPort]).size, 4);
const binary = process.env.LYNX_AUTH_INFLIGHT_BINARY || path.join(root, 'src-tauri/target/debug/lynx-app');
const tokenDelayMs = Number(process.env.LYNX_AUTH_INFLIGHT_TOKEN_DELAY_MS || 2500);
const cancelLimitMs = Number(process.env.LYNX_AUTH_INFLIGHT_CANCEL_LIMIT_MS || 1000);
const oauthHttpTimeoutMs = Number(process.env.LYNX_AUTH_INFLIGHT_HTTP_TIMEOUT_MS || 350);
await mkdir(out, { recursive: true });
const profile = await mkdtemp(path.join(out, 'profile-'));
let tokenRequestResolve;
const tokenRequestSeen = new Promise((resolve) => { tokenRequestResolve = resolve; });
const result = {
  head: execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim(),
  binarySha256: createHash('sha256').update(await readFile(binary)).digest('hex'),
  productionOAuthRequests: false,
  mode,
  tokenDelayMs,
  cancelLimitMs,
  tokenRequests: 0,
  userinfoRequests: 0,
  checks: [],
};
const api = http.createServer((request, response) => {
  if (request.url === '/api/oauth/token' && request.method === 'POST') {
    result.tokenRequests += 1;
    tokenRequestResolve();
    request.resume();
    setTimeout(() => {
      if (response.destroyed) return;
      response.writeHead(mode === 'timeout' ? 503 : 200, { 'Content-Type': 'application/json' });
      response.end(mode === 'timeout' ? 'delayed local failure' : JSON.stringify({ access_token: 'local-delayed-token', token_type: 'Bearer', expires_in: 3600 }));
    }, tokenDelayMs);
    return;
  }
  if (request.url === '/api/oauth/userinfo' && request.method === 'GET') {
    result.userinfoRequests += 1;
    response.writeHead(200, { 'Content-Type': 'application/json' });
    response.end(JSON.stringify({ id: 63, username: 'local', name: 'Local OAuth Test', email: 'local@example.invalid', email_verified: true, avatar_url: '' }));
    return;
  }
  response.writeHead(404).end();
});
await new Promise((resolve, reject) => {
  api.once('error', reject);
  api.listen(apiPort, '127.0.0.1', resolve);
});
const driver = spawn(path.join(os.homedir(), '.cargo/bin/tauri-driver'), ['--port', String(driverPort), '--native-port', String(nativePort)], {
  cwd: root,
  env: {
    ...process.env,
    XDG_CONFIG_HOME: profile + '/config',
    XDG_DATA_HOME: profile + '/data',
    XDG_CACHE_HOME: profile + '/cache',
    XDG_STATE_HOME: profile + '/state',
    LYNX_TEST_ALLOW_PARALLEL_INSTANCE: '1',
    LYNX_TEST_AUTH_KEYRING_NAMESPACE: path.basename(profile),
    LYNX_TEST_OAUTH_API_BASE_URL: `http://127.0.0.1:${apiPort}/`,
    ...(mode === 'timeout' ? { LYNX_TEST_OAUTH_HTTP_TIMEOUT_MS: String(oauthHttpTimeoutMs) } : {}),
    ASK100_REDIRECT_URI: `http://127.0.0.1:${callbackPort}/auth/100ask/callback`,
  },
  stdio: ['ignore', 'inherit', 'inherit'],
});
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
    script: 'const done=arguments[arguments.length-1];window.__TAURI_INTERNALS__.invoke(arguments[0],arguments[1]).then(value=>done({ok:true,value})).catch(error=>done({ok:false,error:String(error)}));',
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
    socket.once('connect', () => { socket.destroy(); resolve(true); });
    socket.once('error', () => resolve(false));
    socket.setTimeout(1500, () => { socket.destroy(); resolve(false); });
  });
}
function check(name, actual, expected) {
  result.checks.push({ name, actual, expected, pass: actual === expected });
}
try {
  assert.equal(await listening(), false, 'callback port must be unused');
  for (let i = 0; i < 100; i += 1) {
    try { await req('GET', '/status'); break; } catch { await delay(100); }
  }
  const created = await req('POST', '/session', { capabilities: { alwaysMatch: { 'tauri:options': { application: binary } } } });
  session = created.sessionId || created.id;
  await writeFile(path.join(out, 'control.json'), JSON.stringify({ session, driverPort, nativePort, callbackPort, apiPort, profile, driverPid: driver.pid }));
  let appUrl = '';
  for (let i = 0; i < 100; i += 1) {
    appUrl = await script('return window.location.href;');
    if (/^(tauri|https?):\/\//.test(appUrl)) break;
    await delay(100);
  }
  result.appUrl = appUrl;
  check('application origin is ready', /^(tauri|https?):\/\//.test(appUrl), true);
  await script('window.__authInflightMessages=[];window.alert=(message)=>window.__authInflightMessages.push(String(message));return true;');
  await script('window.__authInflightEvents=[];window.__authInflightHandler=window.__TAURI_INTERNALS__.transformCallback(event=>window.__authInflightEvents.push(event.payload));return true;');
  await ipc('plugin:event|listen', { event: 'auth-login-result', target: { kind: 'Any' }, handler: await script('return window.__authInflightHandler;') });

  const login = await ipc('auth_login_start');
  const state = new URL(login.authorizeUrl).searchParams.get('state');
  assert.ok(state && state.length >= 16);
  check('callback listener active', await listening(), true);
  // The backend listener is ready before React has necessarily registered its
  // auth result event handler in a fresh WebView. Keep request timing below
  // measured from the observed token request, after UI event readiness.
  await delay(750);
  const callback = fetch(`http://127.0.0.1:${callbackPort}/auth/100ask/callback?${new URLSearchParams({ code: 'local-delayed-code', state })}`, { signal: AbortSignal.timeout(tokenDelayMs + 10000) }).then(async (response) => ({ status: response.status, text: await response.text() })).catch((error) => ({ error: String(error) }));
  await Promise.race([tokenRequestSeen, delay(10000).then(() => { throw Error('delayed token request was not observed'); })]);
  check('one delayed token request reached loopback stub', result.tokenRequests, 1);

  if (mode === 'cancel') {
    const cancelStarted = performance.now();
    await ipc('auth_cancel_login');
    result.cancelElapsedMs = Math.round(performance.now() - cancelStarted);
    check('cancel returns within responsiveness limit', result.cancelElapsedMs < cancelLimitMs, true);
    check('cancel closes callback listener', await listening(), false);
    await callback;
    await delay(tokenDelayMs + 250);
    check('cancelled callback creates no authenticated session', (await ipc('auth_get_user')) === null, true);
    check('cancelled callback emits no terminal alert', (await script('return window.__authInflightMessages.length;')), 0);
    check('cancelled callback emits no terminal event', (await script('return window.__authInflightEvents.length;')), 0);
  } else {
    const requestStarted = performance.now();
    const callbackResult = await callback;
    result.httpFailureElapsedMs = Math.round(performance.now() - requestStarted);
    result.callbackStatus = callbackResult.status || 0;
    check('OAuth request fails before delayed service response', result.httpFailureElapsedMs < 1500, true);
    check('OAuth request observes configured minimum window', result.httpFailureElapsedMs >= 200, true);
    check('timed out callback returns bounded HTML', callbackResult.status === 200 && callbackResult.text.length < 4096, true);
    check('timed out callback renders login failure', callbackResult.text.includes('登录失败'), true);
    check('HTTP timeout closes callback listener', await listening(), false);
    check('HTTP timeout creates no authenticated session', (await ipc('auth_get_user')) === null, true);
    await delay(250);
    const messages = await script('return window.__authInflightMessages;');
    result.terminalAlertCount = messages.length;
    const events = await script('return window.__authInflightEvents;');
    check('HTTP timeout emits exactly one terminal event', events.length, 1);
    check('HTTP timeout terminal event is failure', events.length === 1 && events[0].success === false && Boolean(events[0].error), true);
  }

  await ipc('auth_login_start');
  check('new login reuses released callback port', await listening(), true);
  const retryCancelStarted = performance.now();
  await ipc('auth_cancel_login');
  result.retryCancelElapsedMs = Math.round(performance.now() - retryCancelStarted);
  check('retry cancel remains prompt', result.retryCancelElapsedMs < cancelLimitMs, true);
  check('retry listener closes', await listening(), false);
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
  if (!session) driver.kill('SIGTERM');
  api.closeAllConnections();
  await new Promise((resolve) => api.close(resolve));
  result.pass = !result.error && result.cleanedUp === true && result.checks.every((check) => check.pass);
  await writeFile(path.join(out, 'result.json'), JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result));
  if (!result.pass) process.exitCode = 1;
}
