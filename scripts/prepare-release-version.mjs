#!/usr/bin/env node

import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

import {
  compareStableVersions,
  deltaVersionsCompatible,
  normalizeStableVersion,
} from './build-windows-delta.mjs';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(scriptDir, '..');
const requested = process.argv[2];
if (!requested) {
  throw new Error('Usage: npm run release:prepare -- MAJOR.MINOR.PATCH');
}

const version = normalizeStableVersion(requested);
const packagePath = join(repoRoot, 'package.json');
const packageJson = JSON.parse(readFileSync(packagePath, 'utf8'));
const current = normalizeStableVersion(packageJson.version);
if (compareStableVersions(version, current) < 0) {
  throw new Error(`Refusing to move the release version backwards: ${current} -> ${version}`);
}

if (packageJson.version !== version) {
  packageJson.version = version;
  writeFileSync(packagePath, `${JSON.stringify(packageJson, null, 2)}\n`);
}

const sync = spawnSync(process.execPath, [join(scriptDir, 'sync-version.js')], {
  cwd: repoRoot,
  stdio: 'inherit',
});
if (sync.status !== 0) {
  throw new Error(`Version synchronization failed with exit code ${sync.status ?? 'spawn'}`);
}

const tag = `v${version}`;
const notesDirectory = join(repoRoot, 'docs', 'releases');
const notesPath = join(notesDirectory, `${tag}.md`);
if (!existsSync(notesPath)) {
  mkdirSync(notesDirectory, { recursive: true });
  const strategy = deltaVersionsCompatible(current, version)
    ? `从 ${current} 自动生成精确差分包；同时保留完整安装包回退。`
    : '版本边界不兼容差分，只发布完整安装包。';
  writeFileSync(
    notesPath,
    `# LYNX ${tag}\n\n` +
      `> TODO：发布前填写并删除本行。\n\n` +
      `## 更新内容\n\n- TODO\n\n` +
      `## 升级方式\n\n- ${strategy}\n`
  );
}

const strategy =
  current === version
    ? '当前版本基线：完整安装包'
    : deltaVersionsCompatible(current, version)
      ? `兼容小版本：CI 尝试生成 ${current} -> ${version} 差分包`
      : '大版本或 0.x minor 边界：完整安装包';
console.log(`Release metadata prepared for ${tag}.`);
console.log(`Strategy: ${strategy}.`);
console.log(`Edit ${notesPath}, commit the changes, then run npm run release:stage -- ${tag}.`);
