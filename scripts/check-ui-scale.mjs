import { readFile, readdir } from 'node:fs/promises';
import path from 'node:path';
import process from 'node:process';

const repositoryRoot = path.resolve(import.meta.dirname, '..');
const sourceRoot = path.join(repositoryRoot, 'src');
const allowedPx = new Set([10, 11, 12, 13, 14, 15, 16, 18, 20, 22, 24, 28, 32, 36, 48, 64]);
const allowedRem = new Set([0.6875, 0.75, 0.8125, 0.875, 1, 1.125, 1.25, 1.5, 1.75]);

async function collectCss(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const absolutePath = path.join(directory, entry.name);
    if (entry.isDirectory()) files.push(...(await collectCss(absolutePath)));
    else if (entry.name.endsWith('.css')) files.push(absolutePath);
  }
  return files;
}

function lineNumber(source, index) {
  return source.slice(0, index).split('\n').length;
}

const violations = [];
for (const file of await collectCss(sourceRoot)) {
  const source = await readFile(file, 'utf8');
  const relativePath = path.relative(repositoryRoot, file).split(path.sep).join('/');
  for (const match of source.matchAll(/font-size:\s*([0-9]*\.?[0-9]+)(px|rem|em)\s*;/g)) {
    const value = Number(match[1]);
    const unit = match[2];
    const allowed = unit === 'px' ? allowedPx.has(value) : unit === 'rem' && allowedRem.has(value);
    if (!allowed) {
      violations.push(
        `${relativePath}:${lineNumber(source, match.index ?? 0)} unsupported font size ${value}${unit}`
      );
    }
  }
  for (const match of source.matchAll(/font-weight:\s*([^;]+)\s*;/g)) {
    const value = match[1].trim();
    if (!value.startsWith('var(') && !['400', '500', '600', '650', '700'].includes(value)) {
      violations.push(
        `${relativePath}:${lineNumber(source, match.index ?? 0)} unsupported font weight ${value}`
      );
    }
  }
}

const globalUi = await readFile(path.join(sourceRoot, 'Themes/GlobalUI.css'), 'utf8');
for (const contract of [
  '--ui-text-meta: 11px',
  '--ui-text-caption: 12px',
  '--ui-text-control: 13px',
  '--ui-text-body: 14px',
  '--ui-text-page: 28px',
  '--ui-weight-regular: 400',
  '--ui-weight-medium: 500',
  '--ui-weight-semibold: 600',
  '--ui-weight-bold: 700',
  '--ui-control-height-compact: var(--lynx-control-height-compact)',
  '--ui-control-height: var(--lynx-control-height)',
  '--ui-control-height-comfortable: var(--lynx-control-height-primary)',
  '--ui-icon-box: 32px',
  '--ui-radius-control: var(--lynx-radius-control)',
  '--ui-radius-card: var(--lynx-radius-card)',
]) {
  if (!globalUi.includes(contract)) violations.push(`src/Themes/GlobalUI.css missing ${contract}`);
}
for (const visualContract of [
  ':focus-visible',
  '.device-tab-icon > i',
  'width: 11px',
  'height: 11px',
]) {
  if (!globalUi.includes(visualContract)) {
    violations.push(
      `src/Themes/GlobalUI.css missing visual accessibility contract: ${visualContract}`
    );
  }
}

const lynxTokens = await readFile(path.join(sourceRoot, 'Themes/tokens.css'), 'utf8');
for (const contract of [
  '--lynx-control-height-compact: 32px',
  '--lynx-control-height: 36px',
  '--lynx-control-height-primary: 40px',
  '--lynx-radius-control: 6px',
  '--lynx-radius-card: 8px',
  '--lynx-radius-dialog: 10px',
  '--lynx-sidebar-width: 224px',
  '--lynx-sidebar-collapsed-width: 56px',
  '--lynx-device-rail-width: 264px',
  '--lynx-page-padding: 24px',
]) {
  if (!lynxTokens.includes(contract)) violations.push(`src/Themes/tokens.css missing ${contract}`);
}

const main = await readFile(path.join(sourceRoot, 'main.tsx'), 'utf8');
if (!main.includes("import './Themes/GlobalUI.css';")) {
  violations.push('src/main.tsx must import the global UI scale');
}
if (!main.includes("import './Themes/tokens.css';")) {
  violations.push('src/main.tsx must import the lynx-light design tokens');
}
if (!main.includes("import './Themes/LynxLightPages.css';")) {
  violations.push('src/main.tsx must import the lynx-light mounted-page theme layer');
}
if (!main.includes("import './Themes/FlatUI.css';")) {
  violations.push('src/main.tsx must retain the accepted flat visual system');
}
if (main.indexOf("import './Themes/FlatUI.css';") < main.indexOf("import './Themes/GlobalUI.css';")) {
  violations.push('src/main.tsx must load FlatUI.css after GlobalUI.css');
}
const flatUi = await readFile(path.join(sourceRoot, 'Themes/FlatUI.css'), 'utf8');
for (const contract of [
  '--flat-radius-control: 4px',
  '--flat-radius-card: 6px',
  'background-image: none !important',
  'box-shadow: none !important',
  'prefers-reduced-motion: reduce',
]) {
  if (!flatUi.includes(contract)) violations.push(`src/Themes/FlatUI.css missing ${contract}`);
}

const aiWorkspace = await readFile(path.join(sourceRoot, 'Themes/AIWorkspace.css'), 'utf8');
for (const contract of [
  '--ai-ui-radius: 8px',
  '--ai-ui-rail: 240px',
  '--ai-ui-inspector: 320px',
  '#root .ais-header',
  '#root .device-ai-channel',
]) {
  if (!aiWorkspace.includes(contract)) {
    violations.push(`src/Themes/AIWorkspace.css missing ${contract}`);
  }
}
for (const contract of [
  '#root .ais-policy-unified > *',
  'grid-template-columns: minmax(72px, 0.75fr) minmax(88px, 1fr) minmax(0, 96px)',
  '@media (max-width: 1480px)',
  'grid-template-columns: 210px minmax(0, 1fr)',
  '#root .ais-flow-page > *',
  "#root .ais-flow-task-settings .ais-check input[type='checkbox']",
  'grid-template-columns: 220px minmax(0, 1fr)',
]) {
  if (!aiWorkspace.includes(contract)) {
    violations.push(`src/Themes/AIWorkspace.css missing horizontal-overflow guard: ${contract}`);
  }
}
if (!main.includes("import './Themes/AIWorkspace.css';")) {
  violations.push('src/main.tsx must import the shared AI workspace scale');
}
if (!main.includes("import './Themes/EmbeddedWorkbenchTypography.css';")) {
  violations.push('src/main.tsx must import the embedded workbench typography contract');
}

if (violations.length) {
  console.error('UI scale gate failed:');
  for (const violation of violations) console.error(`- ${violation}`);
  process.exit(1);
}

console.log(
  'UI scale gate passed: typography, control boxes, radii, AI workspace overflow and editability guards use the shared contract.'
);
