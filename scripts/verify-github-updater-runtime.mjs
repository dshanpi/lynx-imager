// Private Ubuntu desktop acceptance: real Tauri updater, public GitHub feed.
// Downloads and verifies the Windows payload; NEVER installs or restarts it.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, readFile, writeFile, realpath } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { createServer } from 'node:net';
import { ensureDesktopIdle } from './lib/desktop-test-safety.mjs';

const out = process.env.LYNX_UPDATER_OUTPUT;
assert.ok(out && path.isAbsolute(out));
await mkdir(out, { recursive: true });
const binary = await realpath(process.env.LYNX_UPDATER_BINARY);
const currentVersion = process.env.LYNX_UPDATER_CURRENT_VERSION;
const expectedVersion = process.env.LYNX_UPDATER_EXPECT_VERSION || '0.1.1';
assert.ok(['0.1.0', '0.1.1'].includes(expectedVersion));
assert.ok(['0.1.0', '0.1.1'].includes(currentVersion));
const downloadDeadlineMs = Number(process.env.LYNX_UPDATER_DOWNLOAD_DEADLINE_MS || 300000);
assert.ok(
  Number.isInteger(downloadDeadlineMs) &&
    downloadDeadlineMs >= 300000 &&
    downloadDeadlineMs <= 1800000
);
const data = await mkdtemp(path.join(os.tmpdir(), 'lynx-github-updater-'));
const result = {
  currentVersion,
  expectedVersion,
  binarySha256: createHash('sha256')
    .update(await readFile(binary))
    .digest('hex'),
  platform: 'Ubuntu',
  updaterTarget: 'windows-x86_64',
  publicNetwork: true,
  mockedIpc: false,
  installExecuted: false,
  nativeWindowsAcceptance: 'NOT VERIFIED',
  downloadDeadlineMs,
  data,
};
const env = {
  ...process.env,
  GDK_BACKEND: 'x11',
  XDG_CONFIG_HOME: data + '/config',
  XDG_DATA_HOME: data + '/data',
  XDG_CACHE_HOME: data + '/cache',
  XDG_STATE_HOME: data + '/state',
  LYNX_TEST_ALLOW_PARALLEL_INSTANCE: '1',
  LYNX_TEST_DISABLE_HARDWARE_AUTOSTART: '1',
  LYNX_TEST_AUTH_KEYRING_NAMESPACE: path.basename(data),
};
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
async function freePort() {
  const server = createServer();
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const port = server.address().port;
  await new Promise((resolve) => server.close(resolve));
  return port;
}
let driver, session, origin;
async function req(method, route, body) {
  const response = await fetch(origin + route, {
    method,
    headers: { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(40000),
  });
  const value = await response.json();
  if (!response.ok || (value.value?.error && value.value?.message))
    throw Error(JSON.stringify(value));
  return value.value;
}
const execute = (script, args = []) =>
  req('POST', `/session/${session}/execute/sync`, { script, args });
async function wait(fn, label, timeout = 30000) {
  const end = Date.now() + timeout;
  let error;
  while (Date.now() < end) {
    try {
      const value = await fn();
      if (value) return value;
    } catch (e) {
      error = e;
    }
    await delay(250);
  }
  throw Error(label + ': ' + (error?.message || 'timeout'));
}
async function ipc(command, args = {}) {
  const value = await req('POST', `/session/${session}/execute/async`, {
    script: `const done=arguments[arguments.length-1];window.__TAURI_INTERNALS__.invoke(arguments[0],arguments[1]).then(value=>done({ok:true,value})).catch(error=>done({ok:false,error:String(error)}));`,
    args: [command, args],
  });
  assert.ok(value.ok, value.error);
  return value.value;
}
try {
  const port = await freePort(),
    nativePort = await freePort();
  origin = `http://127.0.0.1:${port}`;
  driver = spawn(
    path.join(os.homedir(), '.cargo/bin/tauri-driver'),
    ['--port', String(port), '--native-port', String(nativePort)],
    { env, stdio: ['ignore', 'ignore', 'ignore'] }
  );
  await wait(() => req('GET', '/status'), 'driver ready');
  const created = await req('POST', '/session', {
    capabilities: { alwaysMatch: { 'tauri:options': { application: binary } } },
  });
  session = created.sessionId || created.id;
  await req('POST', `/session/${session}/timeouts`, { script: 35000 });
  await writeFile(
    path.join(out, 'control.json'),
    JSON.stringify({ origin, session, data, driverPid: driver.pid })
  );
  await wait(
    () =>
      execute(
        'return !!window.__TAURI_INTERNALS__ && !!document.querySelector("[data-testid=app-sidebar]");'
      ),
    'desktop ready'
  );
  result.origin = await execute('return location.origin;');
  assert.equal(result.origin, 'tauri://localhost');
  result.observedCurrentVersion = await ipc('plugin:app|version');
  assert.equal(result.observedCurrentVersion, currentVersion, 'Running updater version differs');
  result.safetyBefore = await wait(() => ensureDesktopIdle(ipc), 'startup hardware idle');
  result.update = await ipc('plugin:updater|check', { target: 'windows-x86_64', timeout: 30000 });
  if (currentVersion === expectedVersion) {
    assert.equal(result.update, null, 'same version must not offer an update');
  } else {
    assert.equal(result.update.currentVersion, '0.1.0');
    assert.equal(result.update.version, '0.1.1');
    assert.equal(result.update.rawJson.strategy, 'full');
    assert.equal(
      result.update.rawJson.platforms['windows-x86_64'].url,
      'https://github.com/dshanpi/lynx-releases/releases/download/v0.1.1/LYNX_0.1.1_x64-setup.exe'
    );
    await execute(
      `
      window.__updaterDownload={started:false,finished:false,total:0,received:0,done:false,error:null};
      const s=window.__updaterDownload;
      const callback=window.__TAURI_INTERNALS__.transformCallback(raw=>{
        const e=raw.message;if(!e)return;
        if(e.event==='Started'){s.started=true;s.total=e.data.contentLength;}
        if(e.event==='Progress')s.received+=e.data.chunkLength;
        if(e.event==='Finished')s.finished=true;
      });
      window.__TAURI_INTERNALS__.invoke('plugin:updater|download',{
        rid:arguments[0],onEvent:'__CHANNEL__:'+callback
      }).then(rid=>{s.downloadRid=rid;s.done=true;}).catch(e=>{s.error=String(e);s.done=true;});
    `,
      [result.update.rid]
    );
    result.download = await wait(
      async () => {
        const state = await execute('return window.__updaterDownload;');
        return state.done && state;
      },
      'signed download',
      downloadDeadlineMs
    );
    assert.equal(result.download.error, null);
    assert.equal(result.download.started, true);
    assert.equal(result.download.finished, true);
    assert.equal(result.download.received, result.update.rawJson.asset_size);
    assert.equal(typeof result.download.downloadRid, 'number');
    await ipc('plugin:resources|close', { rid: result.download.downloadRid });
    await ipc('plugin:resources|close', { rid: result.update.rid });
    result.signatureVerifiedByNativeUpdater = true;
  }
  const badProxy = await req('POST', `/session/${session}/execute/async`, {
    script: `const done=arguments[arguments.length-1];window.__TAURI_INTERNALS__.invoke('plugin:updater|check',{target:'windows-x86_64',proxy:'http://127.0.0.1:1',timeout:3000}).then(value=>done({unexpectedSuccess:true,value})).catch(error=>done({error:String(error)}));`,
    args: [],
  });
  assert.ok(
    badProxy.error && !badProxy.unexpectedSuccess,
    'network failure must be an error, never already latest'
  );
  result.networkFailure = badProxy;
  result.pass = true;
} catch (error) {
  result.pass = false;
  result.error = String(error.stack || error);
  process.exitCode = 1;
} finally {
  if (session) {
    try {
      result.safetyBeforeClose = await ensureDesktopIdle(ipc);
      await req('DELETE', `/session/${session}`);
      driver.kill('SIGTERM');
    } catch (error) {
      result.cleanupError = String(error);
      process.exitCode = 1;
    }
  } else if (driver) driver.kill('SIGTERM');
  await writeFile(path.join(out, 'result.json'), JSON.stringify(result, null, 2) + '\n');
  console.log(JSON.stringify(result, null, 2));
}
