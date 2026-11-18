import { spawn, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import process from 'node:process';

const root = path.resolve(import.meta.dirname, '..');
const binary = path.join(root, 'src-tauri', 'target', 'debug', 'lynx-imager');
const evidenceRoot = path.resolve(
  process.env.LYNX_SSH_RUNTIME_EVIDENCE_DIR ||
    path.join(root, 'artifacts', 'ssh-terminal-gate', 'linux')
);
const isolatedRoot = await mkdtemp(path.join(os.tmpdir(), 'lynx-ssh-runtime-'));
const elementKey = 'element-6066-11e4-a52e-4f735466cecf';
let driver;
let sshd;
let sessionId = '';
let driverOrigin = '';

function fail(message) {
  throw new Error(`SSH terminal runtime gate failed: ${message}`);
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

async function waitFor(description, operation, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs;
  let lastError;
  while (Date.now() < deadline) {
    try {
      const result = await operation();
      if (result) return result;
    } catch (error) {
      lastError = error;
    }
    await new Promise((resolve) => setTimeout(resolve, 120));
  }
  fail(`${description} timed out${lastError ? `: ${lastError.message}` : ''}`);
}

async function waitForPort(port) {
  return waitFor(
    `port ${port}`,
    () =>
      new Promise((resolve) => {
        const socket = net.createConnection({ host: '127.0.0.1', port });
        socket.once('connect', () => {
          socket.destroy();
          resolve(true);
        });
        socket.once('error', () => resolve(false));
      })
  );
}

async function request(method, endpoint, body, timeoutMs = 30_000) {
  const response = await fetch(`${driverOrigin}${endpoint}`, {
    method,
    headers: body === undefined ? undefined : { 'content-type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(timeoutMs),
  });
  const payload = await response.json().catch(() => ({}));
  if (!response.ok || (payload?.value?.error && payload?.value?.ok !== false)) {
    fail(`${method} ${endpoint}: ${payload?.value?.message || response.statusText}`);
  }
  return payload.value;
}

async function execute(script) {
  return request('POST', `/session/${sessionId}/execute/sync`, { script, args: [] });
}

async function invokeTauri(command, args) {
  const result = await request('POST', `/session/${sessionId}/execute/async`, {
    script: `
      const done = arguments[arguments.length - 1];
      window.__TAURI_INTERNALS__.invoke(arguments[0], arguments[1])
        .then((value) => done({ ok: true, value }))
        .catch((error) => done({ ok: false, error: typeof error === 'string' ? error : JSON.stringify(error) }));
    `,
    args: [command, args],
  });
  if (!result?.ok) fail(`${command}: ${result?.error || 'unknown IPC error'}`);
  return result.value;
}

async function find(testId) {
  const value = await request('POST', `/session/${sessionId}/element`, {
    using: 'css selector',
    value: `[data-testid="${testId}"]`,
  });
  return value[elementKey];
}

async function findCss(selector) {
  const value = await request('POST', `/session/${sessionId}/element`, {
    using: 'css selector',
    value: selector,
  });
  return value[elementKey];
}

async function typeIntoElement(elementId, text) {
  await request('POST', `/session/${sessionId}/element/${elementId}/click`, {});
  await request('POST', `/session/${sessionId}/element/${elementId}/value`, {
    text,
    value: [...text],
  });
}

async function click(testId) {
  await waitFor(`${testId} element`, () => find(testId));
  const result = await execute(`
    const element = [...document.querySelectorAll('[data-testid="${testId}"]')].find((candidate) => {
      if (!(candidate instanceof HTMLElement)) return false;
      const style = getComputedStyle(candidate);
      const rect = candidate.getBoundingClientRect();
      return style.display !== 'none' && style.visibility !== 'hidden' &&
        Number.parseFloat(style.opacity) > 0 && rect.width > 0 && rect.height > 0;
    });
    if (!(element instanceof HTMLElement) || element.matches(':disabled')) return false;
    element.click();
    return true;
  `);
  if (!result) fail(`${testId} is not clickable`);
}

async function stop(child) {
  if (!child || child.exitCode != null) return;
  const exited = new Promise((resolve) => child.once('exit', resolve));
  child.kill('SIGTERM');
  await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, 3_000))]);
  if (child.exitCode == null) child.kill('SIGKILL');
}

try {
  await rm(evidenceRoot, { recursive: true, force: true });
  await mkdir(evidenceRoot, { recursive: true });
  const sshPort = await freePort();
  const driverPort = await freePort();
  driverOrigin = `http://127.0.0.1:${driverPort}`;
  const hostKey = path.join(isolatedRoot, 'ssh-host-ed25519');
  const clientKey = path.join(isolatedRoot, 'ssh-client-ed25519');
  for (const key of [hostKey, clientKey]) {
    const generated = spawnSync('ssh-keygen', ['-q', '-t', 'ed25519', '-N', '', '-f', key]);
    if (generated.status !== 0) fail(`ssh-keygen failed for ${path.basename(key)}`);
  }
  const authorizedKeys = path.join(isolatedRoot, 'authorized_keys');
  await writeFile(authorizedKeys, await readFile(`${clientKey}.pub`));
  const sshdConfig = path.join(isolatedRoot, 'sshd_config');
  await writeFile(
    sshdConfig,
    [
      `Port ${sshPort}`,
      'ListenAddress 127.0.0.1',
      `HostKey ${hostKey}`,
      `AuthorizedKeysFile ${authorizedKeys}`,
      `PidFile ${path.join(isolatedRoot, 'sshd.pid')}`,
      'PubkeyAuthentication yes',
      'PasswordAuthentication no',
      'KbdInteractiveAuthentication no',
      'UsePAM no',
      'PermitRootLogin no',
      'StrictModes no',
      'LogLevel ERROR',
      `AllowUsers ${os.userInfo().username}`,
    ].join('\n') + '\n'
  );
  const sshdLookup = spawnSync('which', ['sshd'], { encoding: 'utf8' });
  const sshdExecutable = sshdLookup.stdout.trim();
  if (sshdLookup.status !== 0 || !path.isAbsolute(sshdExecutable)) {
    fail('could not resolve the absolute sshd path');
  }
  sshd = spawn(sshdExecutable, ['-D', '-e', '-f', sshdConfig], {
    stdio: ['ignore', 'ignore', 'inherit'],
  });
  await waitForPort(sshPort);

  const env = {
    ...process.env,
    XDG_CONFIG_HOME: path.join(isolatedRoot, 'config'),
    XDG_CACHE_HOME: path.join(isolatedRoot, 'cache'),
    XDG_DATA_HOME: path.join(isolatedRoot, 'data'),
    XDG_STATE_HOME: path.join(isolatedRoot, 'state'),
    APPDATA: path.join(isolatedRoot, 'appdata'),
    LOCALAPPDATA: path.join(isolatedRoot, 'localappdata'),
    LYNX_TEST_ALLOW_PARALLEL_INSTANCE: '1',
    LYNX_TEST_AUTH_KEYRING_NAMESPACE: path.basename(isolatedRoot),
  };
  driver = spawn('tauri-driver', ['--port', String(driverPort)], {
    cwd: root,
    env,
    stdio: ['ignore', 'inherit', 'inherit'],
  });
  await waitFor('tauri-driver startup', async () => {
    const response = await fetch(`${driverOrigin}/status`).catch(() => null);
    return response?.ok;
  });
  const session = await request(
    'POST',
    '/session',
    {
      capabilities: { alwaysMatch: { 'tauri:options': { application: binary } } },
    },
    120_000
  );
  sessionId = session.sessionId || session.id;
  if (!sessionId) fail('tauri-driver did not return a session id');
  await request('POST', `/session/${sessionId}/window/rect`, {
    x: 0,
    y: 0,
    width: 1600,
    height: 900,
  });
  await waitFor('desktop startup', () => find('app-sidebar'), 30_000);

  const deviceId = await invokeTauri('workbench_save_device', {
    device: {
      id: null,
      boardUid: '',
      name: 'SSH Runtime Board',
      boardImagePath: '',
      chipVendor: 'allwinner',
      serialPort: '',
      serialVid: 0,
      serialPid: 0,
      serialNumber: '',
      serialInterfaceNumber: null,
      usbBinding: '',
      adbSerial: '',
      baudRate: 115200,
      flasherKind: 'openixcli',
      flasherPath: '',
      flasherArgs: '',
      powerController: '',
      powerControllerVid: 0,
      powerControllerPid: 0,
      powerControllerSerial: '',
      powerControllerInterfaceNumber: null,
      powerBaudRate: 115200,
      powerProfileJson: '',
      powerChannel: '',
      bootChannel: '',
      resetChannel: '',
      powerOffWaitMs: 1000,
      powerOnWaitMs: 2000,
      serialReconnectTimeoutMs: 30000,
      bootTimeoutMs: 60000,
    },
  });
  const host = {
    id: null,
    name: 'Isolated SSH Runtime Host',
    address: '127.0.0.1',
    port: sshPort,
    username: os.userInfo().username,
    authType: 'private_key',
    privateKeyPath: clientKey,
    hostFingerprint: '',
    defaultWorkDir: isolatedRoot,
    aiProvider: 'codex',
    aiExecutable: 'codex',
    aiArgsJson: '[]',
  };
  let untrustedRejected = false;
  try {
    await invokeTauri('workbench_test_ssh', {
      host,
      credentials: {},
      allowUntrusted: true,
    });
  } catch (error) {
    if (!String(error).includes('SSH_TRUST_CONFIRMATION_REQUIRED')) throw error;
    untrustedRejected = true;
  }
  if (!untrustedRejected) fail('an untrusted SSH connection was accepted');
  const hostPublicKey = (await readFile(`${hostKey}.pub`, 'utf8')).trim().split(/\s+/)[1];
  const expectedFingerprint = `SHA256:${createHash('sha256')
    .update(Buffer.from(hostPublicKey, 'base64'))
    .digest('base64')
    .replace(/=+$/, '')}`;
  const trust = await invokeTauri('workbench_test_ssh', {
    host: { ...host, hostFingerprint: expectedFingerprint },
    credentials: {},
    allowUntrusted: false,
  });
  if (!trust.trusted || trust.fingerprint !== expectedFingerprint) {
    fail('the connection did not verify the generated SSH server key');
  }
  const trustedHost = { ...host, hostFingerprint: trust.fingerprint };
  const hostId = await invokeTauri('workbench_save_host', {
    host: trustedHost,
    connectionTestSessionId: trust.auditSessionId,
  });
  const projectId = await invokeTauri('workbench_save_project', {
    project: {
      id: null,
      name: 'SSH Runtime Project',
      hostId,
      deviceId,
      sdkDir: isolatedRoot,
      buildCommand: 'true',
      firmwarePath: '',
      localDownloadDir: isolatedRoot,
      verifySha256: true,
      localGatewayPort: await freePort(),
      remoteForwardPort: 18765,
      mcpServerName: 'lynx-ssh-runtime',
      aiProvider: 'codex',
    },
  });
  const opened = await invokeTauri('workbench_ssh_terminal_open', {
    projectId,
    deviceId,
    mode: 'shell',
    cols: 120,
    rows: 32,
  });
  const connected = await waitFor('SSH PTY connection', async () => {
    const snapshot = await invokeTauri('workbench_ssh_terminal_snapshot', {
      terminalId: opened.terminalId,
      projectId,
      deviceId,
    });
    return snapshot.state === 'connected' ? snapshot : false;
  });
  const marker = 'LYNX_SSH_TERMINAL_OK';
  const input = new TextEncoder().encode(`printf '${marker}\\n'\n`);
  const gateSurfaceId = `runtime-gate:${opened.terminalId}`;
  await invokeTauri('workbench_ssh_terminal_claim_input', {
    terminalId: opened.terminalId,
    projectId,
    deviceId,
    surfaceId: gateSurfaceId,
  });
  await invokeTauri('workbench_ssh_terminal_write', {
    terminalId: opened.terminalId,
    projectId,
    deviceId,
    surfaceId: gateSurfaceId,
    data: [...input],
  });
  const echoed = await waitFor('SSH PTY command output', async () => {
    const snapshot = await invokeTauri('workbench_ssh_terminal_snapshot', {
      terminalId: opened.terminalId,
      projectId,
      deviceId,
    });
    return new TextDecoder().decode(new Uint8Array(snapshot.data)).includes(marker)
      ? snapshot
      : false;
  });
  await invokeTauri('workbench_ssh_terminal_resize', {
    terminalId: opened.terminalId,
    projectId,
    deviceId,
    cols: 100,
    rows: 28,
  });

  await request('POST', `/session/${sessionId}/refresh`, {});
  await waitFor('desktop reload', () => find('app-sidebar'), 30_000);
  await waitFor('runtime board reload', () =>
    execute(`return document.body.innerText.includes('SSH Runtime Board');`)
  );
  await click('linux-tab-sdk');
  await waitFor('SSH terminal online UI', () =>
    execute(`return document.body.innerText.includes('SSH 终端在线');`)
  );
  const beforeUiInput = await invokeTauri('workbench_ssh_terminal_snapshot', {
    terminalId: opened.terminalId,
    projectId,
    deviceId,
  });
  await click('linux-open-terminal-workspace');
  await waitFor('workspace SSH xterm input', () => findCss('.sdk-terminal-xterm textarea'));
  const terminalInput = await findCss('.sdk-terminal-xterm textarea');
  const uiMarker = 'LYNX_SSH_UI_INPUT_ONCE';
  // WebDriver may consume the backslash escape before xterm receives it. The
  // marker itself is enough to prove a complete single keyboard write, so keep
  // this command free of transport-sensitive escape characters.
  const uiCommand = `printf '${uiMarker}'`;
  await typeIntoElement(terminalInput, `${uiCommand}\uE007`);
  const afterUiInput = await waitFor('single workspace SSH keyboard write', async () => {
    const snapshot = await invokeTauri('workbench_ssh_terminal_snapshot', {
      terminalId: opened.terminalId,
      projectId,
      deviceId,
    });
    const output = new TextDecoder().decode(new Uint8Array(snapshot.data));
    return output.includes(uiMarker) ? snapshot : false;
  });
  const expectedUiInputBytes = new TextEncoder().encode(uiCommand).length + 1;
  const actualUiInputBytes = afterUiInput.sentBytes - beforeUiInput.sentBytes;
  if (actualUiInputBytes !== expectedUiInputBytes) {
    fail(
      `workspace keyboard input duplicated: expected ${expectedUiInputBytes} bytes, sent ${actualUiInputBytes}`
    );
  }
  const retainedTerminalText = new TextDecoder().decode(new Uint8Array(afterUiInput.data));
  const retainedBannerOccurrences =
    retainedTerminalText.split('[LYNX] managed shell session').length - 1;
  if (retainedBannerOccurrences !== 1) {
    fail(`managed session banner retained ${retainedBannerOccurrences} times instead of once`);
  }
  await execute(`
    const viewport = document.querySelector('.sdk-terminal-xterm .xterm-viewport');
    if (!(viewport instanceof HTMLElement)) return false;
    viewport.scrollTop = 0;
    viewport.dispatchEvent(new Event('scroll'));
    return true;
  `);
  const renderedTerminalText = await waitFor('managed session banner rendering', () =>
    execute(`
      return document.querySelector('.sdk-terminal-xterm .xterm-rows')?.textContent || '';
    `).then((text) => (text.includes('[LYNX] managed shell session') ? text : false))
  );
  const renderedBannerOccurrences =
    renderedTerminalText.split('[LYNX] managed shell session').length - 1;
  if (renderedBannerOccurrences !== 1) {
    fail(`managed session banner rendered ${renderedBannerOccurrences} times instead of once`);
  }
  const encoded = await request('GET', `/session/${sessionId}/screenshot`);
  const png = Buffer.from(encoded, 'base64');
  const screenshot = 'ssh-terminal-connected.png';
  await writeFile(path.join(evidenceRoot, screenshot), png);

  const clearResult = await invokeTauri('workbench_ssh_terminal_clear', {
    terminalId: opened.terminalId,
    projectId,
    deviceId,
  });
  const cleared = await invokeTauri('workbench_ssh_terminal_snapshot', {
    terminalId: opened.terminalId,
    projectId,
    deviceId,
  });
  const clearedText = new TextDecoder().decode(new Uint8Array(cleared.data));
  if (clearedText.includes(marker)) fail('terminal clear retained pre-clear output');
  if (cleared.outputEpoch !== clearResult.outputEpoch) fail('terminal clear epoch did not persist');
  await invokeTauri('workbench_ssh_terminal_close', {
    terminalId: opened.terminalId,
    projectId,
    deviceId,
  });
  const remaining = await invokeTauri('workbench_ssh_terminal_list', { projectId, deviceId });
  if (remaining.length !== 0) fail('terminal close did not remove the scoped session');

  const evidence = {
    version: 1,
    isolatedData: true,
    target: 'ephemeral-local-sshd',
    trustedFingerprint: trust.fingerprint,
    terminalId: opened.terminalId,
    connectedState: connected.state,
    receivedBytes: echoed.receivedBytes,
    sentBytes: echoed.sentBytes,
    markerObserved: true,
    workspaceKeyboardInput: {
      expectedBytes: expectedUiInputBytes,
      sentBytes: actualUiInputBytes,
      duplicated: false,
    },
    retainedBannerOccurrences,
    renderedBannerOccurrences,
    resize: { cols: 100, rows: 28 },
    clearVerified: true,
    closeVerified: true,
    screenshot,
    screenshotSha256: createHash('sha256').update(png).digest('hex'),
    applicationSha256: createHash('sha256')
      .update(await readFile(binary))
      .digest('hex'),
  };
  await writeFile(
    path.join(evidenceRoot, 'evidence.json'),
    `${JSON.stringify(evidence, null, 2)}\n`
  );
  process.stdout.write(`SSH terminal runtime gate passed: ${opened.terminalId}\n`);
} finally {
  if (sessionId) {
    await request('DELETE', `/session/${sessionId}`).catch(() => undefined);
    sessionId = '';
  }
  await stop(driver);
  await stop(sshd);
  await rm(isolatedRoot, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
}
