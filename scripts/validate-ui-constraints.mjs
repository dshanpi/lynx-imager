import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readdir, readFile } from 'node:fs/promises';
import path from 'node:path';

const root = path.resolve(import.meta.dirname, '..');
const read = (name) => readFile(path.join(root, name), 'utf8');
const skillNames = ['ui-ux-pro-max', 'frontend-design', 'web-design-guidelines', 'color-system'];

// All vendored upstream files are text. Hash LF-normalized content so the lock
// is stable under Git's CRLF checkout setting on Windows.
async function skillDigest(directory) {
  const files = [];
  async function collect(relative) {
    for (const entry of await readdir(path.join(directory, relative), { withFileTypes: true })) {
      if (entry.isDirectory() && entry.name === '__pycache__') continue;
      const name = path.posix.join(relative, entry.name);
      if (entry.isDirectory()) await collect(name);
      else {
        assert(entry.isFile(), `Unexpected non-file in skill: ${name}`);
        files.push(name);
      }
    }
  }
  await collect('');
  const digest = createHash('sha256');
  for (const name of files.sort()) {
    const content = await readFile(path.join(directory, name), 'utf8');
    const hash = createHash('sha256').update(content.replaceAll('\r\n', '\n')).digest('hex');
    digest.update(`${name}\0${hash}\n`);
  }
  return digest.digest('hex');
}

const registry = JSON.parse(await read('skills/sources.lock.json'));
assert.deepEqual(Object.keys(registry).sort(), [...skillNames].sort(), 'Incomplete skill registry');
for (const name of skillNames) {
  const record = registry[name];
  assert.match(record.revision, /^[a-f\d]{40}$/, `${name}: pin the upstream revision`);
  assert.equal(await skillDigest(path.join(root, 'skills', name)), record.sha256,
    `${name}: local skill differs from the reviewed source at ${record.revision}`);
}

const tokenDir = 'design-system/lynx-imager';
const tokens = JSON.parse(await read(`${tokenDir}/tokens.json`));
const stylesheet = (await read(`${tokenDir}/tokens.css`)).replace(/\/\*[\s\S]*?\*\//g, '');
const lynx = JSON.parse(await read('src/Themes/configs/lynx.json'));
assert.equal(lynx.id, 'lynx', 'Expected the original LYNX theme');
const sourceCss = (await read('src/Themes/tokens.css')).replace(/\/\*[\s\S]*?\*\//g, '');
const sourceRules = new Map();
for (const [, selector, body] of sourceCss.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
  const declarations = new Map();
  for (const [, key, value] of body.matchAll(/(--lynx-[\w-]+)\s*:\s*([^;]+);/g)) {
    declarations.set(key, value.trim());
  }
  sourceRules.set(selector.trim(), declarations);
}
const names = ['light', 'dark'];
const css = { light: new Map(), dark: new Map() };
// This declaration-only token file uses three simple scopes; evaluate their
// cascade on a themed root so a variable in the wrong mode cannot pass.
for (const [, selectors, body] of stylesheet.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
  const scopes = new Set();
  for (const rawSelector of selectors.split(',')) {
    const selector = rawSelector.trim();
    if (selector === ':root') names.forEach(mode => scopes.add(mode));
    else {
      const theme = /^\[data-theme=["'](light|dark)["']\]$/.exec(selector);
      assert(theme, `Unsupported token scope: ${selector}`);
      scopes.add(theme[1]);
    }
  }
  for (const [, key, value] of body.matchAll(/([\w-]+)\s*:\s*([^;]+);/g)) {
    for (const mode of scopes) css[mode].set(key, value.trim());
  }
}

function luminance(hex) {
  assert.match(hex, /^#[\da-f]{6}$/i, `Expected an opaque sRGB color: ${hex}`);
  const [r, g, b] = hex.slice(1).match(/../g).map(channel => {
    const s = parseInt(channel, 16) / 255;
    return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

let checkedPairs = 0;
let sourceMatches = 0;
assert.deepEqual(Object.keys(tokens.color).sort(), Object.keys(tokens.dark.color).sort(),
  'Light and dark themes must expose the same roles');
for (const mode of names) {
  const color = mode === 'light' ? tokens.color : tokens.dark.color;
  const expected = new Map([['color-scheme', mode]]);
  for (const [group, roles] of Object.entries({ color, size: tokens.size, space: tokens.space, font: tokens.font })) {
    for (const [role, token] of Object.entries(roles)) expected.set(`--${group}-${role}`, token.$value);
  }
  assert.deepEqual(css[mode], expected, `${mode}: CSS declarations must match tokens.json`);
  const light = mode === 'light';
  const original = lynx.variants[light ? 'Light' : 'Dark'].config;
  const sourceRule = sourceRules.get(`[data-theme='lynx-${mode}']`);
  assert(sourceRule, `${mode}: missing original LYNX CSS theme`);
  function matchesSource(role, value) {
    assert.match(value, /^#[\da-f]{6}$/i, `${mode}: missing original color for ${role}`);
    assert.equal(color[role]?.$value.toLowerCase(), value.toLowerCase(),
      `${mode} ${role} must match the original LYNX theme: ${value}`);
    sourceMatches++;
  }
  const originalRoles = {
    background: 'base', sidebar: 'mantle', surface: 'mantle', subtle: 'crust',
    text: 'text', selected: 'interactiveActive', border: 'border', divider: 'surface1',
    brand: 'accent', 'selection-line': 'accent', focus: 'accent', 'chart-cpu': 'accent',
    'chart-memory': 'subtext0', 'success-mark': 'success', 'warning-mark': 'warning',
    'error-mark': 'error', primary: light ? 'accentHover' : 'accent', 'on-primary': 'accentText',
  };
  if (!light) Object.assign(originalRoles, {
    muted: 'subtext0', connected: 'success', warning: 'warning', error: 'error', 'control-border': 'overlay1',
  });
  for (const [role, field] of Object.entries(originalRoles)) matchesSource(role, original[field]);
  matchesSource('console', sourceRule.get('--lynx-terminal-bg'));
  matchesSource('console-text', sourceRule.get('--lynx-terminal-text'));

  // The translucent LYNX focus halo is specified relative to the surface.
  // The opaque outline has its own 3:1 check below on every interactive surface.
  const ring = sourceRule.get('--lynx-focus-ring') ?? sourceRules.get(':root')?.get('--lynx-focus-ring');
  const rgba = /rgb\((\d+) (\d+) (\d+) \/ (\d+)%\)/.exec(ring ?? '');
  assert(rgba, `${mode}: original LYNX focus ring is missing its RGBA color`);
  const opacity = Number(rgba[4]) / 100;
  const channels = original.mantle.slice(1).match(/../g);
  const blended = channels.map((hex, index) => {
    const value = Number(rgba[index + 1]) * opacity + parseInt(hex, 16) * (1 - opacity);
    return Math.round(value).toString(16).padStart(2, '0');
  });
  matchesSource('halo', '#' + blended.join(''));
  function checkContrast(foreground, background, minimum) {
    assert(color[foreground] && color[background], `${mode}: missing role in ${foreground}/${background}`);
    const a = luminance(color[foreground].$value);
    const b = luminance(color[background].$value);
    const ratio = (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
    assert(ratio >= minimum,
      `${mode} ${foreground}/${background}: ${ratio.toFixed(2)}:1, requires ${minimum}:1`);
    checkedPairs++;
  }
  for (const bg of ['background', 'sidebar', 'surface', 'subtle', 'selected']) {
    for (const fg of ['text', 'muted', 'connected', 'warning', 'error']) checkContrast(fg, bg, 4.5);
    for (const fg of ['focus', 'control-border']) checkContrast(fg, bg, 3);
  }
  checkContrast('on-primary', 'primary', 4.5);
  checkContrast('console-text', 'console', 4.5);
  checkContrast('selection-line', 'selected', 3);
  checkContrast('chart-cpu', 'surface', 3);
  checkContrast('chart-memory', 'surface', 3);
  for (const mark of ['success-mark', 'warning-mark', 'error-mark']) checkContrast(mark, 'surface', 3);
}

// Preserve the shell, supported routes and basic-flow regressions from the previous gate.
const page = await read('src/Components/Imager/ImagerPage.tsx');
const shell = await read('src/main.tsx');
const shellStyles = await read('src/Components/Imager/ImagerShell.css');
for (const name of ['烧录系统', '高级功能', '连接 AI']) assert(shell.includes(name), `Missing workspace ${name}`);
for (const forbidden of ['LearningCenterPage', '学习中心', 'import Layout from']) {
  assert(!shell.includes(forbidden), `Legacy workspace entry ${forbidden}`);
}
for (const name of ['选择烧录目标', '选择系统', '设备连接', '烧写系统', 'V2BoardPicker', 'V2SystemPicker', 'V2Interface', 'V2Write']) {
  assert(page.includes(name), `Missing basic flow control ${name}`);
}
for (const vendor of ['allwinner', 'rockchip', 'nxp', 'st', 'kendryte']) {
  assert(page.includes(`chipVendor: '${vendor}'`), `Missing vendor route ${vendor}`);
}
for (const name of ['imager-app-header', 'imager-app-tabs', 'imager-app-main', 'imager-app-status']) {
  assert(shellStyles.includes(name), `Missing desktop shell region ${name}`);
}
console.log(`Verified ${skillNames.length} vendored skills, ${sourceMatches} original LYNX color matches, theme token parity and ${checkedPairs} contrast pairs plus desktop shell contracts.`);
console.log('Screenshot alignment, keyboard operation and hardware behavior require separate review.');
