import { spawn } from 'node:child_process';
import net from 'node:net';
import path from 'node:path';

const root = path.resolve(import.meta.dirname, '..');
const binary = path.join(root, 'src-tauri', 'target', 'debug', 'lynx-app');
let driver;
let frontend;
let sessionId = '';
let origin = '';

function fail(message) {
  throw new Error(`CH347 monitor HIL failed: ${message}`);
}

async function freePort() {
  const server = net.createServer();
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  const address = server.address();
  const port = typeof address === 'object' && address ? address.port : 0;
  await new Promise((resolve) => server.close(resolve));
  if (!port) fail('could not allocate a local port');
  return port;
}

async function request(method, endpoint, body, timeoutMs = 30_000) {
  const response = await fetch(`${origin}${endpoint}`, {
    method,
    headers: body === undefined ? undefined : { 'content-type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(timeoutMs),
  });
  const payload = await response.json().catch(() => ({}));
  if (!response.ok) {
    fail(`${method} ${endpoint}: ${payload?.value?.message || response.statusText}`);
  }
  return payload.value;
}

async function waitFor(label, operation, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs;
  let lastError;
  while (Date.now() < deadline) {
    try {
      const result = await operation();
      if (result) return result;
    } catch (error) {
      lastError = error;
    }
    await new Promise((resolve) => setTimeout(resolve, 150));
  }
  fail(`${label} timed out${lastError ? `: ${lastError.message}` : ''}`);
}

async function invoke(command, args) {
  const value = await request('POST', `/session/${sessionId}/execute/async`, {
    script: `
      const done = arguments[arguments.length - 1];
      window.__TAURI_INTERNALS__.invoke(arguments[0], arguments[1])
        .then((value) => done({ ok: true, value }))
        .catch((error) => done({ ok: false, error: String(error) }));
    `,
    args: [command, args],
  });
  if (!value?.ok) fail(`${command}: ${value?.error || 'unknown IPC error'}`);
  return value.value;
}

async function stop(child) {
  if (!child || child.exitCode != null) return;
  child.kill('SIGTERM');
  await new Promise((resolve) => setTimeout(resolve, 1_000));
  if (child.exitCode == null) child.kill('SIGKILL');
}

if (process.env.LYNX_HIL_CH347_MONITOR !== '1') {
  fail(
    'set LYNX_HIL_CH347_MONITOR=1 to confirm LCD refresh and read-only INA226 monitoring'
  );
}

try {
  const driverPort = await freePort();
  origin = `http://127.0.0.1:${driverPort}`;
  const existingFrontend = await fetch('http://127.0.0.1:3030').catch(() => null);
  if (!existingFrontend?.ok) {
    frontend = spawn('npm', ['run', 'dev'], {
      cwd: root,
      env: process.env,
      stdio: 'inherit',
    });
    await waitFor('Vite frontend startup', async () => {
      const response = await fetch('http://127.0.0.1:3030').catch(() => null);
      return response?.ok;
    });
  }
  driver = spawn('tauri-driver', ['--port', String(driverPort)], {
    cwd: root,
    env: { ...process.env, LYNX_TEST_ALLOW_PARALLEL_INSTANCE: '1' },
    stdio: 'inherit',
  });
  await waitFor('tauri-driver startup', async () => {
    const response = await fetch(`${origin}/status`).catch(() => null);
    return response?.ok;
  });
  const session = await request(
    'POST',
    '/session',
    { capabilities: { alwaysMatch: { 'tauri:options': { application: binary } } } },
    120_000
  );
  sessionId = session.sessionId || session.id;
  if (!sessionId) fail('tauri-driver returned no session id');
  await waitFor('LYNX desktop', async () => {
    const ready = await request('POST', `/session/${sessionId}/execute/sync`, {
      script: `return Boolean(window.__TAURI_INTERNALS__);`,
      args: [],
    });
    return ready;
  });
  const devices = await invoke('ch347_list_devices');
  if (!Array.isArray(devices) || devices.length === 0) fail('no CH347F device found');
  const device = devices[0];
  await invoke('hardware_monitor_start', {
    request: { deviceIndex: device.index, serialNumber: device.serialNumber || '' },
  });
  const snapshot = await waitFor(
    'two native monitor refreshes',
    async () => {
      const current = await invoke('hardware_monitor_status');
      return current.state === 'running' && current.sequence >= 2 ? current : null;
    },
    90_000
  );
  if (
    snapshot.channels.length !== 4 ||
    snapshot.channels.some((channel) => channel.status !== 'ok') ||
    snapshot.channels.some((channel, index) => channel.address !== 0x40 + index) ||
    snapshot.relays !== 'board-controlled'
  ) {
    fail(`read-only INA226 channels did not all pass: ${JSON.stringify(snapshot)}`);
  }
  const stopped = await invoke('hardware_monitor_stop');
  if (stopped.state !== 'stopped' || stopped.relays !== 'board-controlled') {
    fail(`monitor did not shut down cleanly: ${JSON.stringify(stopped)}`);
  }
  process.stdout.write('CH347 monitor HIL: PASS\n');
} finally {
  if (sessionId) {
    await invoke('hardware_monitor_stop').catch(() => undefined);
    await request('DELETE', `/session/${sessionId}`).catch(() => undefined);
  }
  await stop(driver);
  await stop(frontend);
}
