import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';

const root = process.cwd();
const read = (relative) => fs.readFile(path.join(root, relative), 'utf8');

test('LYNX Imager exposes a real four-step flow and three workspaces', async () => {
  const source = await read('src/Components/Imager/ImagerPage.tsx');
  assert.match(source, /基础烧录/);
  assert.match(source, /高级功能/);
  assert.match(source, /连接 AI/);
  assert.match(source, /选择板子/);
  assert.match(source, /选择系统/);
  assert.match(source, /配置接口/);
  assert.match(source, /烧写系统/);
});

test('basic flow calls existing typed IPC boundaries', async () => {
  const source = await read('src/Components/Imager/ImagerPage.tsx');
  assert.match(source, /workbench_list_devices/);
  assert.match(source, /workbench_start_flash/);
  assert.match(source, /workbench-task-event/);
  assert.match(source, /postFlashAction: 'reboot'/);
  assert.doesNotMatch(source, /setInterval\(/);
});

test('storage routes remain explicit for Allwinner and Rockchip boards', async () => {
  const source = await read('src/Components/Imager/ImagerPage.tsx');
  assert.match(source, /mainline_spinor/);
  assert.match(source, /mainline_emmc/);
  assert.match(source, /route: 'vendor'/);
  assert.match(source, /needsLoader: true/);
});

test('desktop metadata uses the LYNX Imager product identity', async () => {
  const [packageJson, tauriJson, html] = await Promise.all([
    read('package.json').then(JSON.parse),
    read('src-tauri/tauri.conf.json').then(JSON.parse),
    read('index.html'),
  ]);
  assert.equal(packageJson.name, 'lynx-imager');
  assert.equal(tauriJson.productName, 'LYNX Imager');
  assert.match(html, /LYNX Imager/);
});
