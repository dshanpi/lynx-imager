import { spawn } from 'node:child_process';
import { mkdir, writeFile } from 'node:fs/promises';
import net from 'node:net';
import path from 'node:path';

const root = path.resolve(import.meta.dirname, '..');
const binary = path.join(root, 'src-tauri', 'target', 'debug', 'lynx-app');
const outputRoot = path.join(root, 'artifacts', 'live-terminal-workspace');
const elementKey = 'element-6066-11e4-a52e-4f735466cecf';
let driver;
let sessionId = '';
let origin = '';

function fail(message) {
  throw new Error(`Live terminal workspace inspection failed: ${message}`);
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
  if (!port) fail('could not allocate driver port');
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
  if (!response.ok || payload?.value?.error) {
    fail(`${method} ${endpoint}: ${payload?.value?.message || response.statusText}`);
  }
  return payload.value;
}

async function waitFor(label, operation, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs;
  let lastError;
  while (Date.now() < deadline) {
    try {
      const value = await operation();
      if (value) return value;
    } catch (error) {
      lastError = error;
    }
    await new Promise((resolve) => setTimeout(resolve, 150));
  }
  fail(`${label} timed out${lastError ? `: ${lastError.message}` : ''}`);
}

async function execute(script) {
  return request('POST', `/session/${sessionId}/execute/sync`, { script, args: [] });
}

async function invoke(command, args) {
  const result = await request('POST', `/session/${sessionId}/execute/async`, {
    script: `
      const done = arguments[arguments.length - 1];
      window.__TAURI_INTERNALS__.invoke(arguments[0], arguments[1])
        .then((value) => done({ ok: true, value }))
        .catch((error) => done({ ok: false, error: String(error) }));
    `,
    args: [command, args],
  });
  if (!result?.ok) fail(`${command}: ${result?.error || 'unknown IPC error'}`);
  return result.value;
}

async function clickVisible(selector, text = '') {
  const clicked = await execute(`
    const candidate = [...document.querySelectorAll(${JSON.stringify(selector)})].find((item) => {
      if (!(item instanceof HTMLElement)) return false;
      const rect = item.getBoundingClientRect();
      const style = getComputedStyle(item);
      return (!${JSON.stringify(text)} || item.textContent?.includes(${JSON.stringify(text)})) &&
        rect.width > 0 && rect.height > 0 && style.visibility !== 'hidden' && style.display !== 'none';
    });
    if (!candidate || candidate.matches(':disabled')) return false;
    candidate.click();
    return true;
  `);
  if (!clicked) fail(`could not click ${selector} ${text}`);
}

async function screenshot(name) {
  const encoded = await request('GET', `/session/${sessionId}/screenshot`);
  const file = path.join(outputRoot, name);
  await writeFile(file, Buffer.from(encoded, 'base64'));
  return file;
}

async function stop(child) {
  if (!child || child.exitCode != null) return;
  child.kill('SIGTERM');
  await new Promise((resolve) => setTimeout(resolve, 1_000));
  if (child.exitCode == null) child.kill('SIGKILL');
}

try {
  await mkdir(outputRoot, { recursive: true });
  const port = await freePort();
  origin = `http://127.0.0.1:${port}`;
  driver = spawn('tauri-driver', ['--port', String(port)], {
    cwd: root,
    env: { ...process.env, LYNX_TEST_ALLOW_PARALLEL_INSTANCE: '1' },
    stdio: ['ignore', 'inherit', 'inherit'],
  });
  await waitFor('driver startup', async () => {
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
  if (!sessionId) fail('driver returned no session id');
  await request('POST', `/session/${sessionId}/window/rect`, {
    x: 0,
    y: 0,
    width: 1920,
    height: 1080,
  });
  await waitFor('desktop', () =>
    execute(`return Boolean(document.querySelector('[data-testid="app-sidebar"]'));`)
  );

  const devices = await invoke('workbench_list_devices');
  const device = devices.find((item) => /A133/i.test(item.name));
  if (!device?.id) fail('no saved A133 device found');
  const projects = await invoke('workbench_list_projects');
  const project = projects.find((item) => item.deviceId === device.id);
  if (!project?.id) fail(`A133 device ${device.id} has no bound project`);

  await clickVisible('[data-testid="sidebar-tool-workbench"]');
  await waitFor('A133 device card', () =>
    execute(
      `return [...document.querySelectorAll('.board-item')].some((item) => /A133/i.test(item.textContent || ''));`
    )
  );
  await clickVisible('.board-item-select', 'A133');
  await waitFor('workspace button', () =>
    execute(
      `return Boolean(document.querySelector('[data-testid="linux-open-terminal-workspace"]'));`
    )
  );
  const headerMetrics = await execute(`
    const button = document.querySelector('[data-testid="linux-open-terminal-workspace"]');
    const icon = button?.querySelector('svg');
    const uniformControlSelectors = [
      '[data-testid="linux-header-device-state"]',
      '[data-testid="linux-header-flash-state"]',
      '[data-testid="linux-header-new-ssh"]',
      '[data-testid="linux-open-terminal-workspace"]',
      '[data-testid="linux-header-device-settings"]',
    ];
    const bounds = (element) => {
      const rect = element?.getBoundingClientRect();
      return rect ? { x: rect.x, y: rect.y, width: rect.width, height: rect.height,
        right: rect.right, bottom: rect.bottom } : null;
    };
    const style = button ? getComputedStyle(button) : null;
    return { button: bounds(button), icon: bounds(icon), display: style?.display,
      overflow: style?.overflow, padding: style?.padding, gap: style?.gap,
      uniformControls: uniformControlSelectors.map((selector) => bounds(document.querySelector(selector))) };
  `);
  if (
    !headerMetrics.button ||
    !headerMetrics.icon ||
    headerMetrics.display !== 'flex' ||
    headerMetrics.overflow !== 'hidden' ||
    headerMetrics.icon.width !== 16 ||
    headerMetrics.icon.height !== 16 ||
    headerMetrics.icon.x < headerMetrics.button.x ||
    headerMetrics.icon.right > headerMetrics.button.right ||
    headerMetrics.icon.y < headerMetrics.button.y ||
    headerMetrics.icon.bottom > headerMetrics.button.bottom ||
    headerMetrics.uniformControls.length !== 5 ||
    headerMetrics.uniformControls.some(
      (control) => !control || control.width < 64 || control.width > 128 || control.height !== 28
    ) ||
    new Set(headerMetrics.uniformControls.map((control) => Math.round(control.width))).size < 2
  ) {
    fail(`Linux header controls are not uniformly sized: ${JSON.stringify(headerMetrics)}`);
  }
  const headerScreenshot = await screenshot('a133-workbench-header.png');
  await clickVisible('[data-testid="linux-open-terminal-workspace"]');
  await waitFor('four-pane workspace', () =>
    execute(
      `return document.querySelector('[data-testid="linux-terminal-workspace"]')?.dataset.layoutProfile === 'four-pane';`
    )
  );
  const beforeFile = await screenshot('a133-four-pane-before-input.png');

  let sessions = await invoke('workbench_ssh_terminal_list', {
    projectId: project.id,
    deviceId: device.id,
  });
  if (!sessions.length) {
    await clickVisible('.sdk-terminal-workspace-actions button', '新建 Shell');
    sessions = await waitFor(
      'SSH session creation',
      async () => {
        const items = await invoke('workbench_ssh_terminal_list', {
          projectId: project.id,
          deviceId: device.id,
        });
        return items.length ? items : false;
      },
      60_000
    );
  }
  const terminal = sessions[0];
  await waitFor(
    'SSH connected',
    async () => {
      const snapshot = await invoke('workbench_ssh_terminal_snapshot', {
        terminalId: terminal.terminalId,
        projectId: project.id,
        deviceId: device.id,
      });
      return snapshot.state === 'connected' ? snapshot : false;
    },
    60_000
  );
  await waitFor(
    'SSH connected UI',
    () => execute(`return document.body.innerText.includes('终端在线');`),
    60_000
  );
  const before = await invoke('workbench_ssh_terminal_snapshot', {
    terminalId: terminal.terminalId,
    projectId: project.id,
    deviceId: device.id,
  });
  const inputElement = await waitFor('SSH xterm textarea', async () => {
    const value = await request('POST', `/session/${sessionId}/element`, {
      using: 'css selector',
      value: '.sdk-terminal-xterm textarea',
    });
    return value?.[elementKey] || false;
  });
  await request('POST', `/session/${sessionId}/element/${inputElement}/click`, {});
  await new Promise((resolve) => setTimeout(resolve, 750));
  const inputText = '123456';
  let after = before;
  for (const [index, character] of [...inputText].entries()) {
    await request('POST', `/session/${sessionId}/element/${inputElement}/value`, {
      text: character,
      value: [character],
    });
    after = await waitFor(`SSH input accounting ${index + 1}/${inputText.length}`, async () => {
      const snapshot = await invoke('workbench_ssh_terminal_snapshot', {
        terminalId: terminal.terminalId,
        projectId: project.id,
        deviceId: device.id,
      });
      return snapshot.sentBytes - before.sentBytes >= index + 1 ? snapshot : false;
    });
  }
  await new Promise((resolve) => setTimeout(resolve, 500));
  const rendered = await execute(
    `return document.querySelector('.sdk-terminal-xterm .xterm-rows')?.textContent || '';`
  );
  const backendText = new TextDecoder().decode(new Uint8Array(after.data));
  const report = {
    device: { id: device.id, name: device.name },
    project: { id: project.id, name: project.name },
    headerMetrics,
    headerScreenshot,
    terminalId: terminal.terminalId,
    input: inputText,
    sentByteDelta: after.sentBytes - before.sentBytes,
    backendBannerOccurrences: backendText.split('[LYNX] managed shell session').length - 1,
    managedBannerOccurrences: rendered.split('[LYNX] managed shell session').length - 1,
    backendTail: backendText.slice(-400),
    renderedTail: rendered.slice(-400),
    beforeScreenshot: beforeFile,
    afterScreenshot: await screenshot('a133-four-pane-after-input.png'),
  };
  await writeFile(path.join(outputRoot, 'report.json'), `${JSON.stringify(report, null, 2)}\n`);
  process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
} finally {
  if (sessionId) {
    await request('DELETE', `/session/${sessionId}`).catch(() => undefined);
  }
  await stop(driver);
}
