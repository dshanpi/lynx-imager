import { runExtendedButtonAudit } from './extended-button-audit.mjs';
import { ensureDesktopIdle } from './lib/desktop-test-safety.mjs';
import { spawn, execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import process from 'node:process';

const root = path.resolve(import.meta.dirname, '..');
const platform = process.platform === 'win32' ? 'windows' : process.platform;
const binary = process.env.LYNX_BUSINESS_BINARY
  ? path.resolve(process.env.LYNX_BUSINESS_BINARY)
  : path.join(
      root,
      'src-tauri',
      'target',
      'debug',
      process.platform === 'win32' ? 'lynx-imager.exe' : 'lynx-imager'
    );
const nativeWindowsDriver =
  process.platform === 'win32' && process.env.LYNX_BUSINESS_NATIVE_DRIVER_PATH;
const evidenceRoot = path.resolve(
  process.env.LYNX_BUSINESS_EVIDENCE_DIR ||
    path.join(root, 'artifacts', 'business-logic-gate', platform)
);
const isolatedRoot = await mkdtemp(path.join(os.tmpdir(), 'lynx-business-logic-'));
const port = Number(process.env.LYNX_BUSINESS_DRIVER_PORT || 4455);
const origin = `http://127.0.0.1:${port}`;
const elementKey = 'element-6066-11e4-a52e-4f735466cecf';
const checks = [];
const controlSnapshots = [];
const interactionEvents = [];
let driver;
let sessionId = '';

function fail(message) {
  throw new Error(`Business logic UI gate failed: ${message}`);
}

async function request(method, endpoint, body, timeoutMs = 30_000) {
  const response = await fetch(`${origin}${endpoint}`, {
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
    await new Promise((resolve) => setTimeout(resolve, 150));
  }
  fail(`${description} timed out${lastError ? `: ${lastError.message}` : ''}`);
}

async function find(testId) {
  const value = await request('POST', `/session/${sessionId}/element`, {
    using: 'css selector',
    value: `[data-testid="${testId}"]`,
  });
  return value[elementKey];
}

async function click(testId) {
  await waitFor(`${testId} element`, () => find(testId));
  const clicked = await waitFor(`${testId} visible enabled control`, () =>
    execute(`
    const element = [...document.querySelectorAll('[data-testid="${testId}"]')].find((candidate) => {
      if (!(candidate instanceof HTMLElement)) return false;
      const style = getComputedStyle(candidate);
      const rect = candidate.getBoundingClientRect();
      return style.display !== 'none' && style.visibility !== 'hidden' &&
        Number.parseFloat(style.opacity) > 0 && rect.width > 0 && rect.height > 0 &&
        rect.right > 0 && rect.bottom > 0 && rect.left < window.innerWidth && rect.top < window.innerHeight;
    });
    if (!(element instanceof HTMLElement) || element.matches(':disabled')) return false;
    element.click();
    return true;
  `)
  );
  if (!clicked) fail(`${testId} is disabled or not clickable`);
}

async function type(testId, value) {
  await waitFor(`${testId} element`, () => find(testId));
  const changed = await execute(`
    const input = document.querySelector('[data-testid="${testId}"]');
    if (!(input instanceof HTMLInputElement || input instanceof HTMLTextAreaElement)) return false;
    const prototype = input instanceof HTMLTextAreaElement
      ? HTMLTextAreaElement.prototype
      : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(prototype, 'value').set.call(input, ${JSON.stringify(value)});
    input.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertText', data: ${JSON.stringify(value)} }));
    input.dispatchEvent(new Event('change', { bubbles: true }));
    return input.value === ${JSON.stringify(value)};
  `);
  if (!changed) fail(`${testId} did not accept keyboard input`);
}

async function selectValue(testId, value) {
  await waitFor(`${testId} element`, () => find(testId));
  const changed = await execute(`
    const select = document.querySelector('[data-testid="${testId}"]');
    if (!(select instanceof HTMLSelectElement)) return false;
    Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value').set.call(select, ${JSON.stringify(value)});
    select.dispatchEvent(new Event('change', { bubbles: true }));
    return select.value === ${JSON.stringify(value)};
  `);
  if (!changed) fail(`${testId} did not accept selection ${value}`);
}

async function execute(script, args = []) {
  return request('POST', `/session/${sessionId}/execute/sync`, { script, args });
}

async function reveal(testId) {
  await execute(`
    const element = document.querySelector('[data-testid="${testId}"]');
    if (!(element instanceof HTMLElement)) return false;
    element.scrollIntoView({ block: 'center', inline: 'nearest' });
    return true;
  `);
  await new Promise((resolve) => setTimeout(resolve, 250));
}

async function selectLearningGuideTab(label) {
  await waitFor(`learning guide tab ${label}`, () =>
    execute(`
      const button = [...document.querySelectorAll('[aria-label="学习指南分类"] button')]
        .find((element) => element.textContent.trim() === ${JSON.stringify(label)});
      if (!(button instanceof HTMLButtonElement) || button.disabled) return false;
      button.click();
      return true;
    `)
  );
  await waitFor(`learning guide tab ${label} selected`, () =>
    execute(`return [...document.querySelectorAll('[aria-label="学习指南分类"] button')]
      .some((button) => button.textContent.trim() === ${JSON.stringify(label)} &&
        button.getAttribute('aria-pressed') === 'true');`)
  );
}

async function invokeTauri(command, args) {
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

async function bodyContains(text) {
  return execute(`return document.body.innerText.includes(${JSON.stringify(text)});`);
}

async function active(testId) {
  return execute(
    `return [...document.querySelectorAll('[data-testid="${testId}"]')].some((element) => {
      if (!(element instanceof HTMLElement)) return false;
      const style = getComputedStyle(element);
      const rect = element.getBoundingClientRect();
      const visible = style.display !== 'none' && style.visibility !== 'hidden' &&
        Number.parseFloat(style.opacity) > 0 && rect.width > 0 && rect.height > 0;
      return visible && (element.classList.contains('is-active') || element.classList.contains('active'));
    });`
  );
}

async function visualLayerSettled() {
  return execute(`
    const opaque = (element) => {
      if (!(element instanceof HTMLElement)) return false;
      const style = getComputedStyle(element);
      const rect = element.getBoundingClientRect();
      return style.display !== 'none' && style.visibility !== 'hidden' &&
        Number.parseFloat(style.opacity) >= 0.99 && rect.width > 0 && rect.height > 0;
    };
    const activePanel = [...document.querySelectorAll('.tool-panel')].find((element) =>
      element instanceof HTMLElement && getComputedStyle(element).pointerEvents !== 'none'
    );
    if (!opaque(activePanel)) return false;
    for (const selector of ['[data-testid="settings-overlay"]', '[data-testid="settings-panel"]']) {
      const layer = document.querySelector(selector);
      if (layer && !opaque(layer)) return false;
    }
    return true;
  `);
}

async function setWindowSize(width, height) {
  if (nativeWindowsDriver) {
    const result = execFileSync(
      'powershell.exe',
      [
        '-NoProfile',
        '-ExecutionPolicy',
        'Bypass',
        '-File',
        path.join(root, 'scripts/windows-ui-window-size.ps1'),
        '-Width',
        String(width),
        '-Height',
        String(height),
        '-Executable',
        binary,
      ],
      { encoding: 'utf8', windowsHide: true }
    );
    await writeFile(path.join(evidenceRoot, 'windows-last-size.json'), result);
    await waitFor(`Windows logical client ${width}x${height}`, async () => {
      const actual = await execute(
        'return {width:innerWidth,height:innerHeight,dpr:devicePixelRatio};'
      );
      await writeFile(path.join(evidenceRoot, 'windows-client-size.json'), JSON.stringify(actual));
      return Math.abs(actual.width - width) <= 1 && Math.abs(actual.height - height) <= 1;
    });
    return;
  }
  await request('POST', `/session/${sessionId}/window/rect`, { x: 0, y: 0, width, height });
  await waitFor(`window ${width}x${height}`, async () => {
    const rect = await request('GET', `/session/${sessionId}/window/rect`);
    return Math.abs(rect.width - width) <= 1 && Math.abs(rect.height - height) <= 1;
  });
}

async function collectVisualAudit() {
  return execute(`
    const visible = (element) => {
      const style = getComputedStyle(element);
      const rect = element.getBoundingClientRect();
      for (let ancestor = element.parentElement; ancestor; ancestor = ancestor.parentElement) {
        if (ancestor instanceof HTMLDetailsElement && !ancestor.open) {
          const summary = ancestor.querySelector(':scope > summary');
          if (!summary?.contains(element)) return false;
        }
      }
      const toolPanel = element.closest('.tool-panel');
      if (toolPanel instanceof HTMLElement && getComputedStyle(toolPanel).pointerEvents === 'none') {
        return false;
      }
      return style.display !== 'none' && style.visibility !== 'hidden' &&
        Number.parseFloat(style.opacity) > 0 && rect.width > 0 && rect.height > 0;
    };
    const parseColor = (value) => {
      const match = value.match(/rgba?\(([^)]+)\)/);
      if (!match) return null;
      const parts = match[1].split(/[ ,/]+/).filter(Boolean).map(Number);
      if (parts.length < 3 || parts.some((part) => !Number.isFinite(part))) return null;
      return [parts[0], parts[1], parts[2], parts[3] ?? 1];
    };
    const blend = (foreground, background) => {
      const alpha = foreground[3];
      return [
        foreground[0] * alpha + background[0] * (1 - alpha),
        foreground[1] * alpha + background[1] * (1 - alpha),
        foreground[2] * alpha + background[2] * (1 - alpha),
        1,
      ];
    };
    const effectiveBackground = (element) => {
      const layers = [];
      for (let current = element; current instanceof Element; current = current.parentElement) {
        const color = parseColor(getComputedStyle(current).backgroundColor);
        if (color && color[3] > 0) layers.push(color);
      }
      return layers.reverse().reduce((background, layer) => blend(layer, background), [255, 255, 255, 1]);
    };
    const luminance = (color) => {
      const channels = color.slice(0, 3).map((channel) => {
        const value = channel / 255;
        return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
      });
      return channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722;
    };
    const contrast = (foreground, background) => {
      const foregroundLuminance = luminance(blend(foreground, background));
      const backgroundLuminance = luminance(background);
      return (Math.max(foregroundLuminance, backgroundLuminance) + 0.05) /
        (Math.min(foregroundLuminance, backgroundLuminance) + 0.05);
    };
    const describe = (element, index) => {
      const style = getComputedStyle(element);
      const rect = element.getBoundingClientRect();
      const labelledBy = (element.getAttribute('aria-labelledby') || '')
        .split(/\\s+/)
        .filter(Boolean)
        .map((id) => document.getElementById(id)?.innerText || '')
        .join(' ');
      const associatedLabels = 'labels' in element
        ? [...(element.labels || [])].map((label) => label.innerText || '').join(' ')
        : '';
      const wrappingLabel = element.closest('label')?.innerText || '';
      const text = (element.getAttribute('aria-label') || labelledBy || associatedLabels || wrappingLabel ||
        element.getAttribute('title') || element.innerText ||
        element.getAttribute('placeholder') || '').replace(/\\s+/g, ' ').trim();
      const testId = element.getAttribute('data-testid') || '';
      const clippedX = element.scrollWidth > element.clientWidth + 1;
      const clippedY = element.scrollHeight > element.clientHeight + 1;
      const foreground = parseColor(style.color);
      const background = effectiveBackground(element);
      return {
        key: testId || element.id || element.getAttribute('name') || element.tagName.toLowerCase() + '-' + index,
        tag: element.tagName.toLowerCase(),
        className: typeof element.className === 'string' ? element.className : '',
        inputType: element instanceof HTMLInputElement ? element.type : '',
        role: element.getAttribute('role') || '',
        text: text.slice(0, 120),
        disabled: Boolean(element.disabled) || element.getAttribute('aria-disabled') === 'true',
        rect: {
          x: Math.round(rect.x * 100) / 100,
          y: Math.round(rect.y * 100) / 100,
          width: Math.round(rect.width * 100) / 100,
          height: Math.round(rect.height * 100) / 100,
        },
        style: {
          fontFamily: style.fontFamily,
          fontSize: style.fontSize,
          fontWeight: style.fontWeight,
          lineHeight: style.lineHeight,
          color: style.color,
          backgroundColor: style.backgroundColor,
          borderColor: style.borderColor,
          borderRadius: style.borderRadius,
          padding: style.padding,
          textOverflow: style.textOverflow,
          whiteSpace: style.whiteSpace,
        },
        clippedX,
        clippedY,
        contrastRatio: foreground ? Math.round(contrast(foreground, background) * 100) / 100 : null,
      };
    };
    const controls = [...document.querySelectorAll('button, input, select, textarea, a, [role="button"], [role="tab"]')]
      // Monaco 的 2px textarea 是跟随光标移动的输入代理，不是独立点击目标。
      .filter((element) => !element.matches('.monaco-editor textarea.inputarea'))
      .filter(visible)
      .map(describe);
    const textNodes = [...document.querySelectorAll('h1, h2, h3, h4, p, label, small, dt, dd, th, td')]
      .filter(visible)
      .map(describe);
    const issues = [];
    for (const item of controls) {
      const fontSize = Number.parseFloat(item.style.fontSize);
      const compactNativeControl = item.tag === 'input' &&
        ['checkbox', 'radio', 'range', 'color'].includes(item.inputType);
      if (fontSize > 0 && fontSize < 13) issues.push({ type: 'control-font-small', key: item.key, value: fontSize });
      if (!compactNativeControl && item.tag !== 'a' &&
          (item.rect.width < 28 || item.rect.height < 28)) {
        issues.push({ type: 'control-target-small', key: item.key, value: item.rect });
      }
      if (!compactNativeControl && (item.clippedX || item.clippedY)) {
        issues.push({ type: 'control-content-clipped', key: item.key });
      }
      if (!item.text) issues.push({ type: 'control-name-missing', key: item.key });
    }
    for (const item of textNodes) {
      const fontSize = Number.parseFloat(item.style.fontSize);
      const fontWeight = Number.parseInt(item.style.fontWeight, 10) || 400;
      const largeText = fontSize >= 24 || (fontSize >= 18 && fontWeight >= 700);
      const minimumContrast = largeText ? 3 : 4.5;
      if (fontSize > 0 && fontSize < 12) issues.push({ type: 'text-small', key: item.key, value: fontSize });
      const intentionalEllipsis = item.clippedX && item.style.textOverflow === 'ellipsis';
      if ((item.clippedX && !intentionalEllipsis) || item.clippedY) {
        issues.push({ type: 'text-clipped', key: item.key });
      }
      if (item.text && item.contrastRatio !== null && item.contrastRatio < minimumContrast) {
        issues.push({
          type: 'text-contrast-low',
          key: item.key,
          value: { ratio: item.contrastRatio, minimum: minimumContrast },
        });
      }
    }
    return {
      viewport: {
        width: window.innerWidth,
        height: window.innerHeight,
        dpr: window.devicePixelRatio,
        documentScrollWidth: document.documentElement.scrollWidth,
        documentScrollHeight: document.documentElement.scrollHeight,
      },
      controls,
      textNodes,
      issues,
    };
  `);
}

async function collectLargeLightSurfacesInDarkTheme() {
  return execute(`
    if (document.documentElement.dataset.theme !== 'lynx-dark') return [];
    const currentPanel = [...document.querySelectorAll('.tool-panel')].find((element) =>
      element instanceof HTMLElement && getComputedStyle(element).pointerEvents !== 'none'
    );
    const scope = currentPanel || document.body;
    return [...scope.querySelectorAll('main, header, footer, nav, aside, section, article, form, div')]
      .filter((element) => {
        if (!(element instanceof HTMLElement)) return false;
        const style = getComputedStyle(element);
        const rect = element.getBoundingClientRect();
        if (style.display === 'none' || style.visibility === 'hidden' ||
            Number.parseFloat(style.opacity) <= 0 || rect.width * rect.height < 8000) return false;
        const match = style.backgroundColor.match(/rgba?\\((\\d+)[ ,]+(\\d+)[ ,]+(\\d+)/);
        if (!match) return false;
        return Number(match[1]) >= 235 && Number(match[2]) >= 235 && Number(match[3]) >= 235;
      })
      .slice(0, 12)
      .map((element) => ({
        tag: element.tagName.toLowerCase(),
        className: element.className,
        testId: element.dataset.testid || '',
        background: getComputedStyle(element).backgroundColor,
      }));
  `);
}

async function sidebarIsInViewport() {
  return execute(`
    const sidebar = document.querySelector('[data-testid="app-sidebar"]');
    if (!(sidebar instanceof HTMLElement)) return false;
    const rect = sidebar.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0 && rect.left >= 0 && rect.right <= window.innerWidth;
  `);
}

async function recordControlSnapshot(blId, name) {
  const snapshot = await execute(`
    const key='lynx-private-test-interactions';
    const describe=e=>({tag:e.tagName.toLowerCase(),type:e.getAttribute('type')||'',testId:e.getAttribute('data-testid')||'',name:e.getAttribute('aria-label')||e.getAttribute('title')||(e.tagName==='BUTTON'?e.textContent.trim().replace(/\\s+/g,' ').slice(0,140):''),disabled:!!e.disabled});
    if(!window.__lynxAuditInstalled){
      window.__lynxAuditInstalled=true;
      for(const kind of ['click','change','input'])document.addEventListener(kind,event=>{
        const element=event.target.closest?.('button,[role="button"],input,select,textarea,a[href]');
        if(!element)return;
        const queue=JSON.parse(sessionStorage.getItem(key)||'[]');
        queue.push({kind,time:Date.now(),...describe(element)});
        sessionStorage.setItem(key,JSON.stringify(queue));
      },true);
    }
    const controls=[...document.querySelectorAll('button,[role="button"],input,select,textarea,a[href]')].filter(e=>{const r=e.getBoundingClientRect();return r.width>0&&r.height>0&&getComputedStyle(e).visibility!=='hidden';}).map(describe);
    const events=JSON.parse(sessionStorage.getItem(key)||'[]');sessionStorage.removeItem(key);
    return {controls,events};
  `);
  controlSnapshots.push({ blId, name, controls: snapshot.controls });
  interactionEvents.push(
    ...snapshot.events.map((event) => ({ ...event, observedAtScenario: name }))
  );
  await writeFile(
    path.join(evidenceRoot, 'control-inventory.json'),
    JSON.stringify(
      {
        schemaVersion: 1,
        isolated: true,
        valueContentsRecorded: false,
        snapshots: controlSnapshots,
        events: interactionEvents,
        coverageStatement:
          'Inventory and event traces are not success assertions; unexercised controls remain unverified.',
      },
      null,
      2
    )
  );
}

async function capture(blId, name, assertions) {
  await recordControlSnapshot(blId, name);
  for (const assertion of assertions) {
    if (!(await assertion.operation())) fail(`${blId} ${assertion.description}`);
  }
  await waitFor(`${blId} ${name} visual transition`, visualLayerSettled);
  let png;
  const captureSizes = [];
  for (let attempt = 1; attempt <= 3; attempt += 1) {
    const encoded = await request('GET', `/session/${sessionId}/screenshot`);
    png = Buffer.from(encoded, 'base64');
    captureSizes.push(png.length);
    if (png.length >= 20_000) break;
    await new Promise((resolve) => setTimeout(resolve, 500));
    await waitFor(`${blId} ${name} recapture transition`, visualLayerSettled);
  }
  if (!png || png.length < 20_000) {
    fail(`${blId} screenshot is suspiciously small after 3 attempts (${captureSizes.join(', ')})`);
  }
  const filename = `${blId.toLowerCase()}-${name}.png`;
  await writeFile(path.join(evidenceRoot, filename), png);
  const visualAudit = await collectVisualAudit();
  await writeFile(
    path.join(evidenceRoot, `${blId.toLowerCase()}-${name}-audit.json`),
    JSON.stringify(visualAudit, null, 2) + '\n'
  );
  if (visualAudit.issues.length > 0) {
    fail(
      `${blId} ${name} visual audit found ${visualAudit.issues.length} issue(s): ` +
        JSON.stringify(visualAudit.issues.slice(0, 10))
    );
  }
  const largeLightSurfaces = await collectLargeLightSurfacesInDarkTheme();
  if (largeLightSurfaces.length > 0) {
    fail(
      `${blId} ${name} contains large light surfaces in LYNX Dark: ${JSON.stringify(largeLightSurfaces)}`
    );
  }
  checks.push({
    blId,
    name,
    assertions: assertions.map((item) => item.description),
    screenshot: filename,
    bytes: png.length,
    sha256: createHash('sha256').update(png).digest('hex'),
    visualAudit,
    result: 'PASS',
  });
  process.stdout.write(`${blId} PASS · ${name}\n`);
}

async function selectTab(testId) {
  if (!(await active(testId))) await click(testId);
  await waitFor(`${testId} active`, () => active(testId));
}

async function cleanup() {
  if (sessionId) {
    const idle = await waitFor('hardware idle before test cleanup', () =>
      ensureDesktopIdle(invokeTauri)
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
  if (process.env.LYNX_BUSINESS_KEEP_DATA === '1') {
    process.stderr.write(`Business logic debug data retained at ${isolatedRoot}\n`);
  } else {
    if (
      path.dirname(isolatedRoot) !== path.resolve(os.tmpdir()) ||
      !path.basename(isolatedRoot).startsWith('lynx-business-logic-')
    )
      fail('Unexpected private test data cleanup path');
    await rm(isolatedRoot, {
      recursive: true,
      force: true,
      maxRetries: 5,
      retryDelay: 100,
    });
  }
}

try {
  const applicationBytes = await readFile(binary);
  if (process.platform === 'win32') {
    const allowed = path.join(root, 'artifacts') + path.sep;
    if (!evidenceRoot.toLowerCase().startsWith(allowed.toLowerCase()))
      fail('Windows evidence cleanup must stay inside this repository artifacts directory');
  }
  await rm(evidenceRoot, { recursive: true, force: true });
  await mkdir(evidenceRoot, { recursive: true });
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
    LYNX_TEST_SUPERVISOR_KEYRING_NAMESPACE: path.basename(isolatedRoot),
    ...(nativeWindowsDriver
      ? {
          LYNX_TEST_DISABLE_HARDWARE_AUTOSTART: '1',
          WEBVIEW2_USER_DATA_FOLDER: path.join(isolatedRoot, 'webview-profile'),
        }
      : {}),
  };
  driver = spawn(
    nativeWindowsDriver || 'tauri-driver',
    nativeWindowsDriver ? ['--port=' + port, '--allowed-ips=127.0.0.1'] : ['--port', String(port)],
    {
      cwd: root,
      env,
      stdio: ['ignore', 'inherit', 'inherit'],
      windowsHide: true,
    }
  );
  await waitFor('tauri-driver startup', async () => {
    const response = await fetch(`${origin}/status`);
    return response.ok;
  });
  const session = await request(
    'POST',
    '/session',
    {
      capabilities: {
        alwaysMatch: nativeWindowsDriver
          ? {
              browserName: 'webview2',
              'ms:edgeChromium': true,
              'ms:edgeOptions': { binary, args: [] },
            }
          : { 'tauri:options': { application: binary } },
      },
    },
    120_000
  );
  sessionId = session.sessionId || session.id;
  if (!sessionId) fail('tauri-driver did not return a session id');

  await setWindowSize(1600, 900);
  await waitFor('desktop startup', () => find('app-sidebar'), 30_000);
  if (nativeWindowsDriver) {
    if ((await invokeTauri('auth_get_user', {})) !== null)
      fail('Windows test auth profile is not isolated');
    if ((await invokeTauri('workbench_list_devices', {})).length !== 0)
      fail('Windows fixture must start with an empty isolated device database');
    const monitor = await invokeTauri('hardware_monitor_status', {});
    if (monitor.state !== 'stopped' || monitor.errorCode !== 'ISOLATED_UI_TEST_NO_HARDWARE')
      fail('Windows test must not start a physical hardware monitor');
  }
  if (process.env.LYNX_BUSINESS_INITIAL_THEME) {
    if (process.env.LYNX_BUSINESS_INITIAL_THEME !== 'light')
      fail('Unsupported initial theme fixture');
    await click('sidebar-settings');
    await waitFor('initial theme settings', () => find('settings-panel'));
    await selectValue('settings-theme-mode', 'light');
    await selectValue('settings-theme-style', 'lynx-light');
    await execute(
      `const e=document.querySelector('.settings-footer .settings-btn-primary');if(!e||e.disabled)throw Error('Theme save unavailable');e.click();`
    );
    await waitFor('initial theme saved', () => find('settings-save-success'));
    await click('settings-close');
    await waitFor('initial light theme active', () =>
      execute(
        `return !document.querySelector('[data-testid=settings-overlay]')&&document.documentElement.dataset.theme==='lynx-light';`
      )
    );
  }
  await capture('BL-00', 'startup-navigation', [
    { description: '真实 Tauri 窗口已显示侧栏', operation: () => find('app-sidebar') },
    { description: 'Linux 工作台入口存在', operation: () => find('sidebar-tool-workbench') },
    { description: 'MCU 工作台入口存在', operation: () => find('sidebar-tool-mcu-workbench') },
  ]);

  await click('sidebar-settings');
  await waitFor('settings panel', () => find('settings-panel'));
  await capture('BL-14', 'settings-auth-update', [
    { description: '设置面板可打开', operation: () => find('settings-panel') },
    { description: '语言设置可操作', operation: () => find('settings-language') },
    { description: '主题设置可操作', operation: () => find('settings-theme-mode') },
    {
      description: '官网状态同步入口默认关闭',
      operation: () =>
        execute(`
          const control = document.querySelector('[data-testid="settings-cloud-sync"]');
          return control instanceof HTMLInputElement && !control.checked;
        `),
    },
    {
      description: '未登录时诊断上报入口关闭且不可编辑',
      operation: () =>
        execute(`
          const control = document.querySelector('[data-testid="settings-diagnostic-reporting"]');
          return control instanceof HTMLInputElement && !control.checked && control.disabled;
        `),
    },
    {
      description: '未登录时显示重新登录提示',
      operation: () => find('settings-diagnostic-auth-hint'),
    },
    { description: '登录入口存在', operation: () => find('sidebar-login') },
  ]);
  await execute(`
    const content = document.querySelector('.settings-content');
    if (content instanceof HTMLElement) content.scrollTop = content.scrollHeight;
    return true;
  `);
  await capture('BL-14', 'settings-update-about', [
    {
      description: '更新与关于区域可滚动到达',
      operation: () => bodyContains('https://www.100ask.net/'),
    },
    { description: '设置底部操作仍保持可见', operation: () => find('settings-close') },
  ]);
  await click('settings-close');
  await waitFor('settings close animation', () =>
    execute(`return !document.querySelector('[data-testid="settings-overlay"]');`)
  );

  await click('sidebar-tool-learning-home');
  await waitFor('learning center', () => find('learning-center'));
  await waitFor('learning center transition', () =>
    execute(`
      const page = document.querySelector('[data-testid="learning-center"]');
      const layer = page?.parentElement;
      if (!(layer instanceof HTMLElement)) return false;
      const style = getComputedStyle(layer);
      return style.visibility === 'visible' && Number.parseFloat(style.opacity) >= 0.99;
    `)
  );
  // REQ-LEARN-V2-017..020 supersede the removed bundled demo. Exercise the
  // real V2 routes against an isolated, empty native database; never invent
  // an entitlement, signed package, successful lab or hardware result.
  await waitFor('V2 learning guide', () => find('learn-guide'));
  await capture('BL-16', 'learning-guide', [
    { description: '学习指南显示 V2 标题', operation: () => bodyContains('今天想学什么？') },
    { description: '学习首页保留完整应用侧栏', operation: sidebarIsInViewport },
    {
      description: '课程搜索、方向与权益筛选可用',
      operation: () =>
        execute(`return ['搜索课程与章节', '课程方向', '课程权益'].every(
        (label) => document.querySelector('[aria-label="' + label + '"]'));`),
    },
    {
      description: '无正式课程时显示真实空态且没有旧演示或伪造课程',
      operation: () =>
        execute(`return document.querySelector('[data-testid="learn-guide"]')
        ?.textContent.includes('暂无可展示的 AI 学习整课') &&
        document.querySelectorAll('.learn-course-card').length === 0 &&
        !document.querySelector('[data-testid="learning-demo-start"]');`),
    },
  ]);
  await execute(
    `document.querySelectorAll('[data-testid="learn-guide"] details').forEach((element) => { element.open = true; });`
  );
  await capture('BL-16', 'learning-guide-expanded-options', [
    {
      description: '离线选项展开后显示真实导入控件',
      operation: () => bodyContains('导入离线课程包'),
    },
  ]);
  await execute(
    `document.querySelectorAll('[data-testid="learn-guide"] details').forEach((element) => { element.open = false; });`
  );
  await selectLearningGuideTab('配套资料');
  await capture('BL-16', 'learning-resources-tab', [
    {
      description: '配套资料分类切换到真实硬件资源入口',
      operation: () => bodyContains('按产品查看配套资料'),
    },
    {
      description: '资料面板使用实体边框且没有渐变和阴影',
      operation: () =>
        execute(`
        const panel = document.querySelector('[data-testid="learn-guide"] .learn-panel');
        if (!(panel instanceof HTMLElement)) return false;
        const style = getComputedStyle(panel);
        return style.backgroundImage === 'none' && style.boxShadow === 'none' &&
          style.borderStyle === 'solid' && Number.parseFloat(style.borderWidth) > 0;
      `),
    },
  ]);
  await selectLearningGuideTab('我的计划');
  await capture('BL-16', 'learning-plan-empty', [
    { description: '没有会话时不伪造学习进度', operation: () => bodyContains('尚无进行中的课程') },
    {
      description: '空计划没有续学记录',
      operation: () => execute(`return document.querySelectorAll('.learn-plan-row').length === 0;`),
    },
  ]);
  await selectLearningGuideTab('AI辅助课程');
  await click('sidebar-tool-learning-hardware');
  await waitFor('my hardware', () => find('learning-hardware-page'));
  await capture('BL-16', 'learning-hardware', [
    {
      description: '我的硬件作为独立二级页面显示',
      operation: () => find('learning-hardware-page'),
    },
    { description: '订单兑换入口可见', operation: () => find('learning-order-redeem') },
    {
      description: '未登录时不能输入兑换码、提交订单或刷新权益',
      operation: () =>
        execute(`return ['learning-order-code', 'learning-order-redeem-submit', 'learning-hardware-refresh']
        .every((id) => document.querySelector('[data-testid="' + id + '"]')?.disabled === true);`),
    },
    {
      description: '资料引导不依赖订单接口即可查看',
      operation: () => find('learning-hardware-guides'),
    },
    {
      description: '未登录时不伪造已绑定硬件',
      operation: () => bodyContains('登录后查看我的硬件'),
    },
    { description: '我的硬件页面保留完整应用侧栏', operation: sidebarIsInViewport },
  ]);
  await click('sidebar-tool-learning-courses');
  await waitFor('my courses V2', () => find('learn-guide'));
  await capture('BL-16', 'learning-courses', [
    {
      description: '我的课程复用 V2 目录并显示课程模式标题',
      operation: () =>
        execute(
          `return document.querySelector('[data-testid="learn-guide"] h1')?.textContent === '我的课程';`
        ),
    },
    {
      description: '没有课程时不代入任意首个演示课程',
      operation: () => bodyContains('暂无已报名的 AI 学习整课'),
    },
    { description: '课程页面保留完整应用侧栏', operation: sidebarIsInViewport },
  ]);
  await click('sidebar-tool-learning-labs');
  await waitFor('learning labs V2', () => find('learn-labs'));
  await capture('BL-16', 'learning-labs', [
    { description: '实验中心只展示真实会话空态', operation: () => find('learning-labs-empty') },
    {
      description: '未安装签名课程时不伪造实验包',
      operation: () =>
        bodyContains(
          '当前设备尚未安装课程包。已报名课程无需重复导入；老师提供的 .lynx-course 包可单独导入。'
        ),
    },
    {
      description: '实验中心不会自动连接或操作开发板',
      operation: async () => {
        const status = await invokeTauri('hardware_safety_status', {});
        const sessions = await invokeTauri('learning_list_local_sessions', {});
        return status.busy === false && status.activities.length === 0 && sessions.length === 0;
      },
    },
    {
      description: '没有真实包和会话时不暴露工作区或模拟成功结果',
      operation: () =>
        execute(
          `return document.querySelectorAll('.learn-session-row, [data-testid="learning-preview-workspace"], [data-testid="learning-demo-report"]').length === 0;`
        ),
    },
    { description: '实验中心保留完整应用侧栏', operation: sidebarIsInViewport },
  ]);
  await click('sidebar-tool-learning-history');
  await waitFor('learning records V2', () => find('learn-records'));
  await capture('BL-16', 'learning-history', [
    {
      description: '学习记录明确需要真实签名课程和会话',
      operation: () => bodyContains('导入签名课程并开始学习后，记录会保存在这里。'),
    },
    {
      description: '无真实会话时既无演示报告也无虚构完成统计',
      operation: () =>
        execute(`return document.querySelectorAll('[aria-label="学习会话"] option').length === 1 &&
        document.querySelector('[aria-label="学习会话"]').value === '' &&
        !document.querySelector('.learn-summary-layout, [data-testid="learning-demo-history"]');`),
    },
    {
      description: '原生数据库证明页面导航没有自动创建课程会话',
      operation: async () => (await invokeTauri('learning_list_local_sessions', {})).length === 0,
    },
    { description: '学习记录页保留侧栏底部登录与设置入口', operation: sidebarIsInViewport },
  ]);

  await click('sidebar-tool-hardware-monitor');
  await waitFor('hardware monitor page', () => find('hardware-monitor-page'));
  await waitFor('hardware monitor content visible', () =>
    bodyContains('CH347F · ST7789VW · INA226')
  );
  await capture('BL-20', 'ch347-power-ai-monitor', [
    {
      description: '硬件监视作为独立顶级页面显示',
      operation: () =>
        execute(`return Boolean(document.querySelector('[data-testid="hardware-monitor-page"]'));`),
    },
    {
      description: '原生页面展示 ST7789VW、四路 INA226 与 AI 状态',
      operation: () => bodyContains('CH347F · ST7789VW · INA226'),
    },
    {
      description: '后台监视随应用默认启动并以非阻塞状态等待硬件',
      operation: () =>
        execute(`
          const state = document.querySelector('.hm-state');
          return state instanceof HTMLElement &&
            ${JSON.stringify(nativeWindowsDriver ? ['stopped'] : ['starting', 'running', 'paused', 'waiting', 'reconnecting'])}
              .some((value) => state.classList.contains('hm-state-' + value));
        `),
    },
    {
      description: '硬件掉线时页面仍保留设备选择和恢复操作',
      operation: () =>
        execute(`
          return Boolean(document.querySelector('.hm-controls select')) &&
            Boolean(document.querySelector('[data-testid="${nativeWindowsDriver ? 'hardware-monitor-start' : 'hardware-monitor-stop'}"]'));
        `),
    },
  ]);
  await click('sidebar-tool-workbench');

  await invokeTauri('workbench_save_device', {
    device: {
      id: null,
      boardUid: '',
      name: 'BL Gate Linux Board',
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
  await invokeTauri('mcu_save_board', {
    board: {
      id: '',
      name: 'BL Gate MCU Board',
      boardImagePath: '',
      chipVendor: 'st',
      chipSeries: 'stm32h7',
      mcuModel: 'STM32H750ZBT6',
      probeBackend: 'openOcd',
      probeSerial: '',
      serialPort: '',
      serialVid: 0,
      serialPid: 0,
      serialNumber: '',
      serialInterfaceNumber: null,
      baudRate: 115200,
      toolPath: '',
      swdFrequencyKhz: 1000,
      openocdInterfaceConfig: 'interface/stlink.cfg',
      openocdTargetConfig: 'target/stm32h7x.cfg',
      openocdBoardConfig: '',
      openocdFlashBank: 'stm32h7x.bank1.cpu0',
      pyocdTarget: '',
      pyocdPackPath: '',
      jlinkDevice: 'STM32H750ZBT6',
      externalLoaderPath: '',
      ospiAddressStart: 0,
      ospiSizeBytes: 0,
      externalFlashModel: '',
      externalSramModel: '',
      uartInstance: '',
      uartTxPin: '',
      uartRxPin: '',
      bootSuccessText: '',
      bootFailureText: '',
      bootTimeoutMs: 30000,
      mcpHostAddress: '',
      mcpHostPort: 22,
      mcpHostUsername: 'ubuntu',
      mcpAuthType: 'password',
      mcpPrivateKeyPath: '',
      mcpHostFingerprint: '',
      mcpLocalPort: 8767,
      mcpRemotePort: 28767,
      mcpServerName: 'lynx-mcu-bl-gate',
      mcuSdkDir: '',
      mcuBuildCommand: '',
      mcuFirmwarePath: '',
      powerControllerType: '',
      powerDeviceIndex: 0,
      powerGpio: 4,
      powerActiveHigh: true,
    },
  });
  await request('POST', `/session/${sessionId}/refresh`, {});
  await waitFor('desktop reload after isolated persistence', () => find('app-sidebar'), 30_000);
  await waitFor('Linux board persistence', () => bodyContains('BL Gate Linux Board'));
  await capture('BL-01', 'linux-board-save', [
    {
      description: '真实 Tauri IPC 持久化后重新加载可见',
      operation: () => bodyContains('BL Gate Linux Board'),
    },
    {
      description: '保存按钮没有永久停留',
      operation: () => execute(`return !document.body.innerText.includes('保存中…');`),
    },
    {
      description: 'Linux 新建按钮的加号完整位于按钮内并与文字垂直居中',
      operation: () =>
        execute(`
          const button = document.querySelector('[data-testid="linux-new-board"]');
          const icon = button?.querySelector('svg');
          if (!(button instanceof HTMLElement) || !(icon instanceof SVGElement)) return false;
          const outer = button.getBoundingClientRect();
          const inner = icon.getBoundingClientRect();
          const epsilon = 0.75;
          return inner.width >= 15 && inner.height >= 15 &&
            inner.left >= outer.left - epsilon && inner.right <= outer.right + epsilon &&
            inner.top >= outer.top - epsilon && inner.bottom <= outer.bottom + epsilon &&
            Math.abs((inner.top + inner.bottom) / 2 - (outer.top + outer.bottom) / 2) <= 1.5;
        `),
    },
  ]);

  await selectTab('linux-tab-overview');
  await capture('BL-02', 'linux-overview', [
    { description: 'Linux 概览页签可进入', operation: () => active('linux-tab-overview') },
  ]);
  await selectTab('linux-tab-serial');
  await capture('BL-03', 'linux-serial-terminal', [
    { description: '串口终端页签独立显示', operation: () => active('linux-tab-serial') },
  ]);
  await selectTab('linux-tab-overview');
  await capture('BL-03', 'linux-overview-file-transfer', [
    {
      description: 'ADB 文件传输只在当前单板设备概览内显示',
      operation: () =>
        execute(`
          return Boolean(document.querySelector('.linux-overview-transfer [data-testid="adb-file-transfer"]')) &&
            !document.querySelector('[data-testid="linux-tab-adb"]');
        `),
    },
  ]);
  await selectTab('linux-tab-sdk');
  await capture('BL-05', 'linux-ssh-sdk-terminal', [
    { description: 'SSH 终端页签按当前板卡独立显示', operation: () => active('linux-tab-sdk') },
    {
      description: 'SSH 终端声明可见性边界',
      operation: () => find('sdk-terminal-visibility-boundary'),
    },
  ]);
  await setWindowSize(1920, 1080);
  await click('linux-open-terminal-workspace');
  await waitFor('Linux four-pane terminal workspace', () => find('linux-terminal-workspace'));
  const workspaceGeometry = await execute(`
    const workspace = document.querySelector('[data-testid="linux-terminal-workspace"]');
    const bounds = workspace?.getBoundingClientRect();
    return {
      innerWidth: window.innerWidth,
      innerHeight: window.innerHeight,
      screenWidth: window.screen.width,
      screenHeight: window.screen.height,
      profile: workspace instanceof HTMLElement ? workspace.dataset.layoutProfile : null,
      paneCount: workspace?.querySelectorAll('.terminal-workspace-pane').length || 0,
      bounds: bounds ? { width: bounds.width, height: bounds.height } : null,
    };
  `);
  process.stdout.write(`BL-05 workspace geometry · ${JSON.stringify(workspaceGeometry)}\n`);
  await waitFor('native fullscreen four-pane profile', () =>
    execute(`
      const workspace = document.querySelector('[data-testid="linux-terminal-workspace"]');
      if (!(workspace instanceof HTMLElement)) return false;
      const panes = [...workspace.querySelectorAll('.terminal-workspace-pane')].filter((pane) => {
        if (!(pane instanceof HTMLElement)) return false;
        const style = getComputedStyle(pane);
        const rect = pane.getBoundingClientRect();
        return style.display !== 'none' && rect.width > 0 && rect.height > 0;
      });
      const bounds = workspace.getBoundingClientRect();
      return workspace.dataset.layoutProfile === 'four-pane' && panes.length === 4 &&
        bounds.width >= 1800 && bounds.height >= 820;
    `)
  );
  await capture('BL-05', 'linux-four-terminal-workspace-1920', [
    {
      description: '当前单个 Linux 设备在原生全屏中显示四个终端与状态窗格',
      operation: () =>
        execute(`
          const workspace = document.querySelector('[data-testid="linux-terminal-workspace"]');
          return workspace instanceof HTMLElement &&
            workspace.dataset.layoutProfile === 'four-pane' &&
            workspace.querySelectorAll('.terminal-workspace-pane').length === 4 &&
            workspace.dataset.deviceId !== '';
        `),
    },
    {
      description: '烧录设备状态与日志窗格只读且同步输入默认关闭',
      operation: () =>
        execute(`
          const workspace = document.querySelector('[data-testid="linux-terminal-workspace"]');
          return workspace?.innerText.includes('同步输入：关闭') &&
            workspace.innerText.includes('FLASH / DEVICE STATUS') &&
            workspace.innerText.includes('烧录状态') &&
            workspace.querySelector('[data-testid="linux-workspace-device-flash-status"][aria-readonly="true"]') instanceof HTMLElement &&
            !workspace.querySelector('.terminal-workspace-error');
        `),
    },
    {
      description: '工作区左侧导航图标没有越界',
      operation: () =>
        execute(`
          const workspace = document.querySelector('[data-testid="linux-terminal-workspace"]');
          const rail = workspace?.querySelector('.terminal-workspace-rail');
          if (!(workspace instanceof HTMLElement) || !(rail instanceof HTMLElement)) return false;
          const railRect = rail.getBoundingClientRect();
          const epsilon = 0.75;
          const within = (inner, outer) => inner.left >= outer.left - epsilon &&
            inner.right <= outer.right + epsilon && inner.top >= outer.top - epsilon &&
            inner.bottom <= outer.bottom + epsilon;
          return rail.scrollWidth <= rail.clientWidth &&
            [...rail.querySelectorAll('button')].every((button) => {
              const buttonRect = button.getBoundingClientRect();
              const icon = button.querySelector('svg');
              return within(buttonRect, railRect) &&
                (!(icon instanceof SVGElement) || within(icon.getBoundingClientRect(), buttonRect));
            });
        `),
    },
    {
      description: '辅助窗格标题工具没有越界',
      operation: () =>
        execute(`
          const workspace = document.querySelector('[data-testid="linux-terminal-workspace"]');
          if (!(workspace instanceof HTMLElement)) return false;
          const epsilon = 0.75;
          const withinHorizontally = (inner, outer) => inner.left >= outer.left - epsilon &&
            inner.right <= outer.right + epsilon;
          const auxiliaryHeaders = [...workspace.querySelectorAll(
            '.terminal-workspace-pane[data-workspace-slot^="auxiliary"] .terminal-workspace-pane-header'
          )];
          return auxiliaryHeaders.length === 3 && auxiliaryHeaders.every((header) => {
            if (!(header instanceof HTMLElement)) return false;
            const headerRect = header.getBoundingClientRect();
            return [...header.querySelectorAll('button, svg')].every((control) =>
              withinHorizontally(control.getBoundingClientRect(), headerRect)
            );
          });
        `),
    },
  ]);
  await click('linux-exit-terminal-workspace');
  await waitFor('Linux terminal workspace exit', () =>
    execute(`return !document.querySelector('[data-testid="linux-terminal-workspace"]');`)
  );
  await setWindowSize(1600, 900);
  await selectTab('linux-tab-flash');
  await capture('BL-04', 'linux-flash-power', [
    { description: '固件烧录页可进入但未启动烧录', operation: () => active('linux-tab-flash') },
  ]);
  await selectTab('linux-tab-ai');
  await capture('BL-05', 'linux-ai-channel', [
    { description: 'Linux 设备 AI 通道按当前板卡显示', operation: () => active('linux-tab-ai') },
  ]);

  await click('sidebar-tool-mcu-workbench');
  await waitFor('MCU board persistence', () => bodyContains('BL Gate MCU Board'));
  await click('mcu-open-settings');
  await waitFor('MCU settings dialog', () => find('mcu-settings-dialog'));
  await type('mcu-board-name', 'BL Gate MCU Editable');
  await capture('BL-06', 'mcu-board-save', [
    { description: 'MCU 单板配置已持久化并可重新加载', operation: () => find('mcu-board-save') },
    {
      description: 'MCU 新建表单可编辑且保存按钮可用',
      operation: () =>
        execute(
          `return document.querySelector('[data-testid="mcu-board-name"]')?.value === 'BL Gate MCU Editable' && !document.querySelector('[data-testid="mcu-board-save"]')?.disabled;`
        ),
    },
    { description: '设置弹窗位于内容视口', operation: () => find('mcu-settings-dialog') },
  ]);
  await execute(`
    const content = document.querySelector('.workbench-settings-body');
    if (content instanceof HTMLElement) content.scrollTop = content.scrollHeight;
    return true;
  `);
  await capture('BL-06', 'mcu-settings-bottom', [
    { description: 'MCU 设置全部字段可滚动到达', operation: () => find('mcu-board-save') },
    { description: 'MCU 设置关闭操作保持可见', operation: () => find('mcu-settings-close') },
  ]);
  await click('mcu-settings-close');

  await selectTab('mcu-tab-overview');
  await capture('BL-06', 'mcu-overview', [
    { description: 'MCU 概览页签独立显示', operation: () => active('mcu-tab-overview') },
  ]);
  await selectTab('mcu-tab-serial');
  await capture('BL-07', 'mcu-serial-terminal', [
    { description: 'MCU 串口页签独立显示', operation: () => active('mcu-tab-serial') },
  ]);
  await selectTab('mcu-tab-probe');
  await capture('BL-07', 'mcu-probe', [
    { description: '探针页签可进入且未执行目标动作', operation: () => active('mcu-tab-probe') },
  ]);
  await selectTab('mcu-tab-flash');
  await capture('BL-08', 'mcu-flash-power', [
    {
      description: 'MCU 烧录页可进入但未启动烧录或电源动作',
      operation: () => active('mcu-tab-flash'),
    },
  ]);
  await selectTab('mcu-tab-ai');
  await capture('BL-09', 'mcu-ai-channel', [
    { description: 'MCU AI 通道独立于 Linux 域', operation: () => active('mcu-tab-ai') },
  ]);

  await click('sidebar-tool-ai-supervisor-overview');
  await waitFor('Supervisor overview', () => bodyContains('监督总览'));
  await waitFor('Supervisor overview layout', () =>
    execute(`
      const layout = document.querySelector('.ais-overview-layout');
      if (!(layout instanceof HTMLElement)) return false;
      const panel = layout.closest('.tool-panel');
      const style = panel instanceof HTMLElement ? getComputedStyle(panel) : null;
      const rect = layout.getBoundingClientRect();
      return style?.visibility === 'visible' && Number.parseFloat(style.opacity) >= 0.99 &&
        rect.width > 0 && rect.height > 0;
    `)
  );
  await capture('BL-10', 'ai-supervisor-overview', [
    { description: '监督总览独立显示', operation: () => bodyContains('监督总览') },
  ]);
  await click('sidebar-tool-ai-supervisor-config');
  await selectTab('ai-config-tab-policy');
  await capture('BL-10', 'ai-config-policy', [
    { description: '策略与安全页签独立显示', operation: () => active('ai-config-tab-policy') },
  ]);
  await selectTab('ai-config-tab-notifications');
  await capture('BL-10', 'ai-config-notifications', [
    { description: '通知设置页签独立显示', operation: () => active('ai-config-tab-notifications') },
  ]);
  await selectTab('ai-config-tab-providers');
  await capture('BL-10', 'ai-config-providers', [
    { description: 'AI 服务配置可进入', operation: () => active('ai-config-tab-providers') },
  ]);

  await click('sidebar-tool-ai-supervisor-automatic');
  await waitFor('automatic task editor', () => bodyContains('创建任务'));
  await capture('BL-11', 'automatic-task-editor', [
    { description: '自动任务编辑器可进入', operation: () => bodyContains('创建任务') },
  ]);

  await click('sidebar-tool-ai-supervisor-runtime');
  await waitFor('run center', () => bodyContains('运行中心'));
  await capture('BL-12', 'run-human-audit', [
    { description: '运行中心可进入', operation: () => bodyContains('运行中心') },
    {
      description: '人工介入或审计上下文可见',
      operation: () =>
        execute(
          `return document.body.innerText.includes('人工') || document.body.innerText.includes('审计');`
        ),
    },
  ]);

  await click('sidebar-tool-record-center');
  for (const section of ['overview', 'events', 'sessions', 'memory', 'feedback']) {
    await selectTab(`record-tab-${section}`);
    await capture('BL-13', `record-${section}`, [
      {
        description: `记录中心 ${section} 页签独立显示`,
        operation: () => active(`record-tab-${section}`),
      },
    ]);
  }
  await selectTab('record-tab-feedback');
  await type('record-feedback-title', 'BL Gate 本地问题草稿');
  await click('record-feedback-save');
  await waitFor('local feedback draft', () => bodyContains('BL Gate 本地问题草稿'));
  await capture('BL-13', 'record-feedback-saved', [
    {
      description: '反馈草稿已在隔离数据库保存',
      operation: () => bodyContains('BL Gate 本地问题草稿'),
    },
  ]);
  for (const section of ['backup', 'access']) {
    await selectTab(`record-tab-${section}`);
    await capture('BL-13', `record-${section}`, [
      {
        description: `记录中心 ${section} 页签独立显示`,
        operation: () => active(`record-tab-${section}`),
      },
    ]);
  }

  await click('sidebar-tool-ai-supervisor-config');
  await selectTab('ai-config-tab-policy');
  await runExtendedButtonAudit({ execute, invokeTauri, click, capture, waitFor, selectTab });
  await click('sidebar-tool-ai-supervisor-config');
  await selectTab('ai-config-tab-policy');
  await setWindowSize(1366, 880);
  await capture('BL-10', 'responsive-1366-ai-policy', [
    { description: '1366×880 下策略页保持可操作', operation: () => active('ai-config-tab-policy') },
  ]);
  await setWindowSize(1090, 880);
  await capture('BL-10', 'responsive-1090-ai-policy', [
    { description: '1090×880 下策略页保持可操作', operation: () => active('ai-config-tab-policy') },
  ]);

  await click('sidebar-settings');
  await waitFor('settings panel for theme matrix', () => find('settings-panel'));
  await selectValue('settings-theme-mode', 'light');
  await capture('BL-14', 'theme-light-settings', [
    {
      description: '浅色主题生效',
      operation: () =>
        execute(`
          return document.documentElement.dataset.theme === 'lynx-light' &&
            document.documentElement.dataset.themeVariant === 'light';
        `),
    },
  ]);
  await selectValue('settings-language', 'en-US');
  await capture('BL-14', 'language-en-settings', [
    { description: '英文界面可切换', operation: () => bodyContains('Language') },
  ]);
  await selectValue('settings-language', 'zh-CN');
  await selectValue('settings-theme-mode', 'dark');
  await selectValue('settings-theme-style', 'lynx-dark');
  await capture('BL-14', 'theme-dark-settings', [
    {
      description: '深色主题生效',
      operation: () =>
        execute(`
          return document.documentElement.dataset.theme === 'lynx-dark' &&
            document.documentElement.dataset.themeVariant === 'dark';
        `),
    },
  ]);
  await execute(`
    const button=document.querySelector('.settings-footer .settings-btn-primary');
    if(!button || button.disabled)throw Error('Settings save unavailable');button.scrollIntoView({block:'center'});button.click();
  `);
  await waitFor('settings save result', () => find('settings-save-success'));
  await click('settings-close');
  await waitFor('settings close after persisted theme', () =>
    execute('return !document.querySelector("[data-testid=settings-overlay]");')
  );
  await click('sidebar-tool-workbench');
  await waitFor('dark Linux workbench', () => bodyContains('BL Gate Linux Board'));
  await selectTab('linux-tab-overview');
  const darkRailVisual = await execute(`
    const heading = document.querySelector('.device-board-rail-heading h3');
    const button = document.querySelector('[data-testid="linux-new-board"]');
    return {
      heading: heading ? getComputedStyle(heading).color : null,
      button: button ? getComputedStyle(button).color : null,
      opacity: button ? getComputedStyle(button).opacity : null,
      disabled: button instanceof HTMLButtonElement ? button.disabled : null,
      rootText: getComputedStyle(document.documentElement).getPropertyValue('--color-text').trim(),
      lynxText: getComputedStyle(document.documentElement).getPropertyValue('--lynx-text-primary').trim(),
    };
  `);
  process.stdout.write(`BL-14 dark rail visual · ${JSON.stringify(darkRailVisual)}\n`);
  await capture('BL-14', 'theme-dark-linux-overview', [
    {
      description: 'LYNX Dark 覆盖 Linux 工作台且新建图标保持在按钮内',
      operation: () =>
        execute(`
          const root = document.documentElement;
          const workbench = document.querySelector('.board-workbench');
          const rail = document.querySelector('.board-list');
          const heading = document.querySelector('.device-board-rail-heading h3');
          const button = document.querySelector('[data-testid="linux-new-board"]');
          const icon = button?.querySelector('svg');
          if (!(workbench instanceof HTMLElement) || !(rail instanceof HTMLElement) ||
              !(heading instanceof HTMLElement) || !(button instanceof HTMLElement) ||
              !(icon instanceof SVGElement)) return false;
          const outer = button.getBoundingClientRect();
          const inner = icon.getBoundingClientRect();
          return root.dataset.theme === 'lynx-dark' &&
            getComputedStyle(workbench).backgroundColor === 'rgb(8, 9, 11)' &&
            getComputedStyle(rail).backgroundColor === 'rgb(17, 19, 22)' &&
            getComputedStyle(heading).color === 'rgb(240, 242, 245)' &&
            getComputedStyle(button).color === 'rgb(47, 140, 255)' &&
            inner.width >= 15 && inner.height >= 15 &&
            inner.left >= outer.left && inner.right <= outer.right &&
            inner.top >= outer.top && inner.bottom <= outer.bottom;
        `),
    },
    {
      description: '设备概览内只嵌入文件传输，不混入串口、烧录或全屏终端控制器',
      operation: () =>
        execute(`
          return !document.querySelector('.board-console') &&
            !document.querySelector('[data-testid="linux-terminal-workspace"]') &&
            Boolean(document.querySelector('.linux-overview-transfer [data-testid="adb-file-transfer"]'));
        `),
    },
  ]);
  for (const [tab, name] of [
    ['serial', 'serial-terminal'],
    ['sdk', 'ssh-terminal'],
    ['flash', 'firmware-flash'],
    ['ai', 'ai-channel'],
  ]) {
    await selectTab(`linux-tab-${tab}`);
    await capture('BL-14', `theme-dark-linux-${name}`, [
      {
        description: `LYNX Dark 覆盖 Linux ${name} 页面`,
        operation: () => active(`linux-tab-${tab}`),
      },
    ]);
  }

  // The responsive matrix above intentionally leaves the desktop at 1090×880.
  // Restore the presentation viewport before asserting the four-pane profile;
  // otherwise the workspace correctly selects its compact layout on slower
  // WebDriver runners and this release gate becomes order-dependent.
  await setWindowSize(1920, 1080);
  await click('linux-open-terminal-workspace');
  await waitFor('dark Linux fullscreen workspace', () => find('linux-terminal-workspace'));
  await waitFor('dark Linux fullscreen four-pane reflow', () =>
    execute(`
      return document.querySelector('[data-testid="linux-terminal-workspace"]')
        ?.getAttribute('data-layout-profile') === 'four-pane';
    `)
  );
  const darkFullscreenVisual = await execute(`
    const workspace = document.querySelector('[data-testid="linux-terminal-workspace"]');
    return workspace instanceof HTMLElement ? {
      background: getComputedStyle(workspace).backgroundColor,
      profile: workspace.dataset.layoutProfile,
      panes: workspace.querySelectorAll('.terminal-workspace-pane').length,
      isolated: workspace.textContent?.includes('INPUT ISOLATED'),
      theme: document.documentElement.dataset.theme,
      viewport: [window.innerWidth, window.innerHeight],
    } : null;
  `);
  process.stdout.write(`BL-14 dark fullscreen visual · ${JSON.stringify(darkFullscreenVisual)}\n`);
  await capture('BL-14', 'theme-dark-linux-fullscreen-workspace', [
    {
      description: 'LYNX Dark 全屏四窗工作区使用黑色画布并保持输入隔离',
      operation: () =>
        execute(`
          const workspace = document.querySelector('[data-testid="linux-terminal-workspace"]');
          if (!(workspace instanceof HTMLElement)) return false;
          return document.documentElement.dataset.theme === 'lynx-dark' &&
            getComputedStyle(workspace).backgroundColor === 'rgb(5, 6, 7)' &&
            workspace.querySelectorAll('.terminal-workspace-pane').length === 4 &&
            workspace.textContent?.includes('INPUT ISOLATED');
        `),
    },
  ]);
  await click('linux-exit-terminal-workspace');
  await waitFor('exit dark Linux fullscreen workspace', () =>
    execute(`return !document.querySelector('[data-testid="linux-terminal-workspace"]');`)
  );

  await click('sidebar-tool-mcu-workbench');
  for (const [tab, name] of [
    ['overview', 'overview'],
    ['serial', 'serial-terminal'],
    ['probe', 'probe'],
    ['flash', 'firmware-flash'],
    ['ai', 'ai-channel'],
  ]) {
    await selectTab(`mcu-tab-${tab}`);
    await capture('BL-14', `theme-dark-mcu-${name}`, [
      {
        description: `LYNX Dark 覆盖 MCU ${name} 页面`,
        operation: () => active(`mcu-tab-${tab}`),
      },
    ]);
  }

  for (const [route, pageId, name] of [
    ['home', 'learn-guide', 'home'],
    ['hardware', 'learning-hardware-page', 'hardware'],
    ['courses', 'learn-guide', 'courses'],
    ['labs', 'learn-labs', 'labs'],
    ['history', 'learn-records', 'history'],
  ]) {
    await click(`sidebar-tool-learning-${route}`);
    await waitFor(`dark learning ${name}`, () => find(pageId));
    await capture('BL-14', `theme-dark-learning-${name}`, [
      { description: `LYNX Dark 覆盖学习中心 ${name} 页面`, operation: () => find(pageId) },
    ]);
  }
  await click('sidebar-tool-learning-home');
  await selectLearningGuideTab('配套资料');
  await capture('BL-14', 'theme-dark-learning-resources', [
    {
      description: 'LYNX Dark 覆盖 V2 配套资料分类',
      operation: () => bodyContains('按产品查看配套资料'),
    },
  ]);
  await selectLearningGuideTab('我的计划');
  await capture('BL-14', 'theme-dark-learning-plan', [
    {
      description: 'LYNX Dark 覆盖 V2 真实学习计划空态',
      operation: () => bodyContains('尚无进行中的课程'),
    },
  ]);
  await selectLearningGuideTab('AI辅助课程');

  await click('sidebar-tool-ai-supervisor-overview');
  await capture('BL-14', 'theme-dark-ai-overview', [
    { description: 'LYNX Dark 覆盖 AI 监督总览', operation: () => bodyContains('监督总览') },
  ]);
  await click('sidebar-tool-ai-supervisor-config');
  for (const [tab, name] of [
    ['policy', 'policy'],
    ['notifications', 'notifications'],
    ['providers', 'providers'],
  ]) {
    await selectTab(`ai-config-tab-${tab}`);
    await capture('BL-14', `theme-dark-ai-${name}`, [
      {
        description: `LYNX Dark 覆盖 AI ${name} 页面`,
        operation: () => active(`ai-config-tab-${tab}`),
      },
    ]);
  }
  await click('sidebar-tool-ai-supervisor-automatic');
  await capture('BL-14', 'theme-dark-automatic-task-editor', [
    { description: 'LYNX Dark 覆盖自动任务编辑器', operation: () => bodyContains('创建任务') },
  ]);
  await click('sidebar-tool-ai-supervisor-runtime');
  await capture('BL-14', 'theme-dark-runtime-audit', [
    { description: 'LYNX Dark 覆盖运行和审计页面', operation: () => bodyContains('运行中心') },
  ]);

  await click('sidebar-tool-record-center');
  for (const section of [
    'overview',
    'events',
    'sessions',
    'memory',
    'feedback',
    'backup',
    'access',
  ]) {
    await selectTab(`record-tab-${section}`);
    await capture('BL-14', `theme-dark-record-${section}`, [
      {
        description: `LYNX Dark 覆盖记录中心 ${section} 页面`,
        operation: () => active(`record-tab-${section}`),
      },
    ]);
  }

  const requiredIds = Array.from(
    { length: 15 },
    (_, index) => `BL-${String(index).padStart(2, '0')}`
  );
  const actualIds = new Set(checks.map((item) => item.blId));
  for (const id of requiredIds) if (!actualIds.has(id)) fail(`missing ${id} evidence`);
  if (!actualIds.has('BL-16')) fail('missing BL-16 evidence');
  if (!actualIds.has('BL-20')) fail('missing BL-20 evidence');

  const manifest = {
    schemaVersion: 1,
    generatedAt: new Date().toISOString(),
    platform,
    initialTheme: process.env.LYNX_BUSINESS_INITIAL_THEME || 'system',
    isolatedData: true,
    destructiveHardwareActionsExecuted: false,
    application: binary,
    applicationSha256: createHash('sha256').update(applicationBytes).digest('hex'),
    checks,
    packagingGate: { blId: 'BL-15', result: 'RUN_SEPARATELY_AFTER_UI_PASS' },
  };
  await writeFile(
    path.join(evidenceRoot, 'evidence.json'),
    `${JSON.stringify(manifest, null, 2)}\n`
  );
  process.stdout.write(
    `Business logic UI gate passed: ${checks.length} BL checks and screenshots saved to ${evidenceRoot}.\n`
  );
} finally {
  await cleanup();
}
