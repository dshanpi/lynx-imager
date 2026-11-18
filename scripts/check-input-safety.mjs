import { readFile, readdir } from 'node:fs/promises';
import path from 'node:path';
import process from 'node:process';

const repositoryRoot = path.resolve(import.meta.dirname, '..');
const scanRoots = ['src', 'src-tauri/src', 'src-tauri/capabilities', 'scripts'];
const rootFiles = ['package.json', 'src-tauri/Cargo.toml', 'scripts/windows-installer.nsi'];
const sourceExtensions = new Set(['.ts', '.tsx', '.js', '.mjs', '.rs', '.json', '.toml', '.nsi']);

const prohibitedRuntimePatterns = [
  ['xdotool', /\bxdotool\b/i],
  ['ydotool', /\bydotool\b/i],
  ['wtype', /\bwtype\b/i],
  ['Linux uinput', /(?:\/dev\/uinput|\buinput\b)/i],
  ['Windows SendInput', /\bSendInput\b/i],
  ['Windows keybd_event', /\bkeybd_event\b/i],
  ['Windows mouse_event', /\bmouse_event\b/i],
  ['PowerShell SendKeys', /\bSendKeys\b/i],
  ['AppleScript input automation', /(?:\bosascript\b|System Events)/i],
  ['input automation dependency', /\b(?:enigo|autopy|pyautogui|robotjs|nut-js)\b/i],
  ['global shortcut registration', /\bglobal[_-]?shortcut\b|\bGlobalShortcut\b/i],
  [
    'global Ctrl/Cmd+F suppression',
    /(?:ctrlKey|metaKey)[\s\S]{0,160}(?:key|code)\s*===?\s*['"]f['"][\s\S]{0,160}preventDefault\s*\(/i,
  ],
  [
    'synthetic keyboard event',
    /(?:dispatchEvent\s*\(\s*new\s+KeyboardEvent|new\s+KeyboardEvent\s*\()/i,
  ],
  ['forced user attention', /\brequestUserAttention\b/i],
];

// These focus operations are allowed only because each is reached from an explicit
// user action (clicking a terminal, opening a prompt, creating/renaming a folder,
// or opening API documentation). Any new focus site must be reviewed and added here.
const reviewedFocusCounts = new Map([
  ['src/Hooks/useModalDialog.tsx', 1],
  ['src/Components/ADBExplorer/Hooks/useNewFolder.ts', 2],
  ['src/Components/SerialTool/Components/SerialMonitor.tsx', 1],
  ['src/Components/SPITool/Components/SPIMasterTab.tsx', 1],
  // Settings opens on click. Its close button receives initial focus, and the
  // palette and settings return focus to their respective openers on dismissal.
  ['src/Settings/Settings.tsx', 2],
  ['src/Themes/Components/ThemeViewer.tsx', 1],
  ['src/main.tsx', 1],
  // Explicit Tab at a modal boundary and an explicit clipboard copy fallback.
  ['src/CoreUI/dialogKeys.ts', 1],
  ['src/Components/Imager/imagerAIControls.tsx', 1],
  // Closing the confirmation drawer with Back or Escape restores keyboard focus.
  ['src/Components/Imager/V2Write.tsx', 1],
  // Explicit Ctrl+K on the visible board picker focuses its search box.
  ['src/Components/Imager/TargetPicker.tsx', 1],
  // Opening an inspector, clicking Back or pressing Escape inside it transfers focus once.
  ['src/Components/Imager/SystemImageReview.tsx', 1],
]);

const violations = [];

function lineNumber(source, index) {
  return source.slice(0, index).split('\n').length;
}

function report(relativePath, source, index, message) {
  violations.push(`${relativePath}:${lineNumber(source, index)} ${message}`);
}

async function collectFiles(relativeDirectory) {
  const absoluteDirectory = path.join(repositoryRoot, relativeDirectory);
  const entries = await readdir(absoluteDirectory, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const relativePath = path.join(relativeDirectory, entry.name);
    if (entry.isDirectory()) {
      if (entry.name === 'node_modules' || entry.name === 'target' || entry.name === 'dist')
        continue;
      files.push(...(await collectFiles(relativePath)));
    } else if (sourceExtensions.has(path.extname(entry.name))) {
      files.push(relativePath);
    }
  }
  return files;
}

const discoveredFiles = (await Promise.all(scanRoots.map(collectFiles))).flat();
const files = [...new Set([...discoveredFiles, ...rootFiles])]
  .map((file) => file.split(path.sep).join('/'))
  .filter((file) => file !== 'scripts/check-input-safety.mjs');

for (const relativePath of files) {
  const source = await readFile(path.join(repositoryRoot, relativePath), 'utf8');

  for (const [label, pattern] of prohibitedRuntimePatterns) {
    const match = pattern.exec(source);
    if (match) report(relativePath, source, match.index, `prohibited ${label}`);
  }

  if (/\.(?:ts|tsx)$/.test(relativePath)) {
    const focusMatches = [...source.matchAll(/\.focus\s*\(|\.setFocus\s*\(|\bautoFocus\b/g)];
    const expectedCount = reviewedFocusCounts.get(relativePath) ?? 0;
    if (focusMatches.length !== expectedCount) {
      const index = focusMatches[expectedCount]?.index ?? 0;
      report(
        relativePath,
        source,
        index,
        `unreviewed focus operation count: expected ${expectedCount}, found ${focusMatches.length}`
      );
    }

    // Lifecycle-driven focus was the historical terminal keyboard-capture defect.
    // Ban it even in files that contain separately reviewed, click-driven focus.
    const effectPattern = /\b(?:React\.)?useEffect\s*\([\s\S]*?\n\s*\},\s*\[[^\]]*\]\s*\);?/g;
    for (const effect of source.matchAll(effectPattern)) {
      const focus = /\.focus\s*\(|\.setFocus\s*\(|\bautoFocus\b/.exec(effect[0]);
      if (focus) {
        report(
          relativePath,
          source,
          (effect.index ?? 0) + focus.index,
          'focus operation inside a React effect'
        );
      }
    }
  }
}

if (violations.length) {
  console.error('Input-safety gate failed:');
  for (const violation of violations) console.error(`- ${violation}`);
  process.exit(1);
}

console.log(`Input-safety gate passed (${files.length} shipped/runtime files scanned).`);
