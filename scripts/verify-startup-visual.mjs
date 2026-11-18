import { ensureDesktopIdle } from './lib/desktop-test-safety.mjs';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import process from 'node:process';

const repositoryRoot = path.resolve(import.meta.dirname, '..');
const platformName = process.platform === 'win32' ? 'windows' : process.platform;
const binaryName = process.platform === 'win32' ? 'lynx-imager.exe' : 'lynx-imager';
const application = process.env.LYNX_VISUAL_APPLICATION
  ? path.resolve(process.env.LYNX_VISUAL_APPLICATION)
  : path.join(repositoryRoot, 'src-tauri', 'target', 'debug', binaryName);
const evidenceRoot = path.resolve(
  process.env.LYNX_VISUAL_EVIDENCE_DIR ||
    path.join(repositoryRoot, 'artifacts', 'visual-gate', 'startup', platformName)
);
const isolatedRoot = await mkdtemp(path.join(os.tmpdir(), 'lynx-startup-visual-'));
const driverPort = Number(process.env.LYNX_TAURI_DRIVER_PORT || 4444);
const driverOrigin = `http://127.0.0.1:${driverPort}`;
const scenarios = [
  { width: 1280, height: 880, collapsed: false },
  { width: 1280, height: 880, collapsed: true },
  { width: 1090, height: 880, collapsed: false },
  { width: 1090, height: 880, collapsed: true },
];
const elementKey = 'element-6066-11e4-a52e-4f735466cecf';
let driver;
let sessionId = '';
let expectedScreenshotInsets;

function fail(message) {
  throw new Error(`Startup visual gate failed: ${message}`);
}

async function request(method, endpoint, body, timeoutMs = 30_000) {
  const response = await fetch(`${driverOrigin}${endpoint}`, {
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

async function waitFor(description, operation, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs;
  let lastError;
  let lastResult;
  while (Date.now() < deadline) {
    try {
      const result = await operation();
      if (result) return result;
      lastResult = result;
    } catch (error) {
      lastError = error;
    }
    await new Promise((resolve) => setTimeout(resolve, 150));
  }
  const detail =
    lastError?.message || (lastResult ? JSON.stringify(lastResult) : 'condition false');
  fail(`${description} timed out: ${detail}`);
}

async function find(testId) {
  const value = await request('POST', `/session/${sessionId}/element`, {
    using: 'css selector',
    value: `[data-testid="${testId}"]`,
  });
  return value[elementKey];
}

async function click(testId) {
  const id = await waitFor(`${testId} element`, () => find(testId));
  await request('POST', `/session/${sessionId}/element/${id}/click`, {});
}

async function execute(script, args = []) {
  return request('POST', `/session/${sessionId}/execute/sync`, { script, args });
}

async function executeAsync(script, args = []) {
  return request('POST', `/session/${sessionId}/execute/async`, { script, args });
}

async function waitForCompositorFrame() {
  await executeAsync(`
    const done = arguments[arguments.length - 1];
    requestAnimationFrame(() => requestAnimationFrame(done));
  `);
}

function pngDimensions(buffer) {
  const signature = '89504e470d0a1a0a';
  if (buffer.length < 33 || buffer.subarray(0, 8).toString('hex') !== signature) {
    fail('WebDriver screenshot is not a valid PNG');
  }
  return { width: buffer.readUInt32BE(16), height: buffer.readUInt32BE(20) };
}

async function captureScenario(scenario) {
  const requestedRect = {
    x: 0,
    y: 0,
    width: scenario.width,
    height: scenario.height,
  };
  await request('POST', `/session/${sessionId}/window/rect`, requestedRect);
  const windowRect = await waitFor('window resize', async () => {
    const rect = await request('GET', `/session/${sessionId}/window/rect`);
    if (
      Math.abs(rect.width - scenario.width) <= 1 &&
      Math.abs(rect.height - scenario.height) <= 1
    ) {
      return rect;
    }
    // WebKitWebDriver occasionally drops a resize immediately after taking a
    // screenshot. Re-submit the same bounded request, but keep the exact-size
    // assertion: a clamped or permanently wrong window still fails with the
    // last observed dimensions in the diagnostic.
    await request('POST', `/session/${sessionId}/window/rect`, requestedRect);
    throw new Error(
      `requested ${scenario.width}x${scenario.height}, observed ${rect.width}x${rect.height}`
    );
  });

  const currentCollapsed = await execute(
    'return document.querySelector("[data-testid=app-sidebar]").classList.contains("sidebar--collapsed");'
  );
  if (currentCollapsed !== scenario.collapsed) await click('sidebar-toggle');

  const expectedSidebarWidth = scenario.collapsed ? 56 : 224;
  const geometry = await waitFor('stable non-overlapping layout', async () => {
    const result = await execute(`
      const sidebar = document.querySelector('[data-testid=app-sidebar]').getBoundingClientRect();
      const backdropElement = document.querySelector('[data-testid=mcu-settings-backdrop]');
      const dialogElement = document.querySelector('[data-testid=mcu-settings-dialog]');
      const backdrop = backdropElement.getBoundingClientRect();
      const dialog = dialogElement.getBoundingClientRect();
      const backdropStyle = getComputedStyle(backdropElement);
      const dialogStyle = getComputedStyle(dialogElement);
      return {
        viewport: { width: window.innerWidth, height: window.innerHeight, dpr: window.devicePixelRatio },
        sidebar: { left: sidebar.left, right: sidebar.right, width: sidebar.width },
        backdrop: { left: backdrop.left, right: backdrop.right, width: backdrop.width },
        dialog: { left: dialog.left, right: dialog.right, top: dialog.top, bottom: dialog.bottom },
        visibility: {
          backdrop: backdropStyle.visibility,
          dialog: dialogStyle.visibility,
          backdropOpacity: Number.parseFloat(backdropStyle.opacity),
          dialogOpacity: Number.parseFloat(dialogStyle.opacity),
        },
        documentScrollWidth: document.documentElement.scrollWidth,
        bodyScrollWidth: document.body.scrollWidth,
      };
    `);
    const epsilon = 1;
    const valid =
      Math.abs(result.sidebar.width - expectedSidebarWidth) <= epsilon &&
      result.sidebar.right <= result.backdrop.left + epsilon &&
      result.dialog.left + epsilon >= result.sidebar.right &&
      result.dialog.right <= result.viewport.width + epsilon &&
      result.dialog.top >= -epsilon &&
      result.dialog.bottom <= result.viewport.height + epsilon &&
      result.visibility.backdrop === 'visible' &&
      result.visibility.dialog === 'visible' &&
      result.visibility.backdropOpacity >= 0.99 &&
      result.visibility.dialogOpacity >= 0.99 &&
      result.documentScrollWidth <= result.viewport.width &&
      result.bodyScrollWidth <= result.viewport.width;
    if (!valid) throw new Error(`invalid layout geometry: ${JSON.stringify(result)}`);
    return result;
  });

  const state = scenario.collapsed ? 'collapsed' : 'expanded';
  const baseName = `mcu-settings-${scenario.width}x${scenario.height}-${state}`;
  await waitForCompositorFrame();
  await request('GET', `/session/${sessionId}/screenshot`);
  await waitForCompositorFrame();
  const encoded = await request('GET', `/session/${sessionId}/screenshot`);
  const png = Buffer.from(encoded, 'base64');
  const dimensions = pngDimensions(png);
  if (png.length < 20_000) fail(`${baseName}.png is suspiciously small (${png.length} bytes)`);
  const screenshotCssSize = {
    width: dimensions.width / geometry.viewport.dpr,
    height: dimensions.height / geometry.viewport.dpr,
  };
  if (!Number.isInteger(screenshotCssSize.width) || !Number.isInteger(screenshotCssSize.height)) {
    fail(`${baseName}.png dimensions are not aligned to DPR ${geometry.viewport.dpr}`);
  }
  const screenshotInsets = {
    width: windowRect.width - screenshotCssSize.width,
    height: windowRect.height - screenshotCssSize.height,
  };
  if (
    screenshotInsets.width < 0 ||
    screenshotInsets.width > 80 ||
    screenshotInsets.height < 0 ||
    screenshotInsets.height > 80
  ) {
    fail(
      `${baseName}.png has invalid window decoration insets ${JSON.stringify(screenshotInsets)}`
    );
  }
  if (!expectedScreenshotInsets) expectedScreenshotInsets = screenshotInsets;
  if (
    screenshotInsets.width !== expectedScreenshotInsets.width ||
    screenshotInsets.height !== expectedScreenshotInsets.height
  ) {
    fail(
      `${baseName}.png decoration insets ${JSON.stringify(screenshotInsets)} differ from ${JSON.stringify(expectedScreenshotInsets)}`
    );
  }
  if (
    geometry.dialog.right > screenshotCssSize.width ||
    geometry.dialog.bottom > screenshotCssSize.height
  ) {
    fail(
      `${baseName}.png clips the settings dialog: dialog=${JSON.stringify(geometry.dialog)}, ` +
        `screenshotCssSize=${JSON.stringify(screenshotCssSize)}, windowRect=${JSON.stringify(windowRect)}`
    );
  }
  await writeFile(path.join(evidenceRoot, `${baseName}.png`), png);
  return {
    ...scenario,
    screenshot: `${baseName}.png`,
    windowRect,
    screenshotDimensions: dimensions,
    screenshotCssSize,
    screenshotInsets,
    bytes: png.length,
    sha256: createHash('sha256').update(png).digest('hex'),
    geometry,
  };
}

async function safetyInvoke(command, args) {
  const result = await executeAsync(
    `
    const done=arguments[arguments.length-1];
    window.__TAURI_INTERNALS__.invoke(arguments[0],arguments[1]).then(value=>done({ok:true,value})).catch(error=>done({ok:false,error:String(error)}));
  `,
    [command, args]
  );
  if (!result?.ok) throw new Error(result?.error || 'Safety IPC failed');
  return result.value;
}

async function cleanup() {
  if (sessionId) {
    const idle = await waitFor('hardware idle before visual cleanup', () =>
      ensureDesktopIdle(safetyInvoke)
    );
    await writeFile(
      path.join(evidenceRoot, 'safety-before-close.json'),
      JSON.stringify(idle, null, 2)
    );
    await request('DELETE', `/session/${sessionId}`).catch(() => undefined);
    sessionId = '';
  }
  if (driver && driver.exitCode == null) {
    const exited = new Promise((resolve) => driver.once('exit', resolve));
    driver.kill('SIGTERM');
    await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, 3_000))]);
  }
  let cleanupError;
  for (let attempt = 0; attempt < 10; attempt += 1) {
    try {
      await rm(isolatedRoot, { recursive: true, force: true });
      cleanupError = undefined;
      break;
    } catch (error) {
      cleanupError = error;
      await new Promise((resolve) => setTimeout(resolve, 200));
    }
  }
  if (cleanupError) throw cleanupError;
}

try {
  const applicationBytes = await readFile(application);
  await rm(evidenceRoot, { recursive: true, force: true });
  await mkdir(evidenceRoot, { recursive: true });
  const isolatedEnv = {
    ...process.env,
    XDG_CONFIG_HOME: path.join(isolatedRoot, 'config'),
    XDG_CACHE_HOME: path.join(isolatedRoot, 'cache'),
    XDG_DATA_HOME: path.join(isolatedRoot, 'data'),
    XDG_STATE_HOME: path.join(isolatedRoot, 'state'),
    APPDATA: path.join(isolatedRoot, 'appdata'),
    LOCALAPPDATA: path.join(isolatedRoot, 'localappdata'),
    LYNX_TEST_ALLOW_PARALLEL_INSTANCE: '1',
    LYNX_TEST_AUTH_KEYRING_NAMESPACE: path.basename(isolatedRoot),
    LYNX_TEST_SUPERVISOR_KEYRING_NAMESPACE: path.basename(isolatedRoot),
  };
  driver = spawn('tauri-driver', ['--port', String(driverPort)], {
    cwd: repositoryRoot,
    env: isolatedEnv,
    stdio: ['ignore', 'inherit', 'inherit'],
  });
  driver.once('exit', (code) => {
    if (code && !sessionId) process.stderr.write(`tauri-driver exited early with code ${code}\n`);
  });
  await waitFor('tauri-driver startup', async () => {
    const response = await fetch(`${driverOrigin}/status`);
    return response.ok;
  });
  const session = await request(
    'POST',
    '/session',
    {
      capabilities: { alwaysMatch: { 'tauri:options': { application } } },
    },
    120_000
  );
  sessionId = session.sessionId || session.id;
  if (!sessionId) fail('tauri-driver did not return a session id');

  await waitFor('LYNX desktop UI startup', () => find('app-sidebar'), 30_000);
  await click('sidebar-tool-mcu-workbench');
  await click('mcu-new-board');
  await waitFor('MCU settings dialog', () => find('mcu-settings-dialog'));

  const results = [];
  for (const scenario of scenarios) results.push(await captureScenario(scenario));
  const screenshotHashes = new Set(results.map((result) => result.sha256));
  if (screenshotHashes.size !== results.length)
    fail('two or more scenario screenshots are identical');

  const manifest = {
    schemaVersion: 1,
    generatedAt: new Date().toISOString(),
    platform: platformName,
    application,
    applicationSha256: createHash('sha256').update(applicationBytes).digest('hex'),
    scenarios: results,
  };
  await writeFile(
    path.join(evidenceRoot, 'evidence.json'),
    `${JSON.stringify(manifest, null, 2)}\n`
  );
  process.stdout.write(
    `Startup visual gate passed: ${results.length} Tauri screenshots and geometry evidence saved to ${evidenceRoot}.\n`
  );
} finally {
  await cleanup();
}
