import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';

const root = process.cwd();
const read = (relative) => fs.readFile(path.join(root, relative), 'utf8');

test('LYNX Imager exposes a real four-step flow and three workspaces', async () => {
  const source = await read('src/Components/Imager/ImagerPage.tsx');
  const shell = await read('src/main.tsx');
  assert.match(source, /基础烧录/);
  assert.match(shell, /高级烧录/);
  assert.match(shell, /连接 AI/);
  assert.match(source, /选择板子/);
  assert.match(source, /选择系统/);
  assert.match(source, /配置接口/);
  assert.match(source, /烧写系统/);
  assert.match(source, /搜索板型、芯片或厂商/);
  assert.match(source, /imager-board-picker/);
  assert.match(source, /已选板型/);
});

test('basic flow calls existing typed IPC boundaries', async () => {
  const source = await read('src/Components/Imager/ImagerPage.tsx');
  assert.match(source, /workbench_list_devices/);
  assert.match(source, /workbench_start_flash/);
  assert.match(source, /workbench-task-event/);
  assert.match(source, /postFlashAction: 'reboot'/);
  assert.doesNotMatch(source, /setInterval\(/);
});

test('storage routes and vendor backends remain explicit for all supported families', async () => {
  const source = await read('src/Components/Imager/ImagerPage.tsx');
  assert.match(source, /mainline_spinor/);
  assert.match(source, /mainline_emmc/);
  assert.match(source, /rockchip_raw_emmc/);
  assert.match(source, /route: 'vendor'/);
  assert.match(source, /needsLoader: true/);
  assert.match(source, /chipVendor: 'nxp'/);
  assert.match(source, /chipVendor: 'st'/);
  assert.match(source, /chipVendor: 'kendryte'/);
  assert.match(source, /usesSavedVendorRecipe/);
});

test('the product shell has no learning-center navigation', async () => {
  const source = await read('src/main.tsx');
  assert.doesNotMatch(source, /LearningCenterPage/);
  assert.doesNotMatch(source, /学习中心/);
  assert.match(source, /基础烧录/);
  assert.match(source, /连接 AI/);
});

test('desktop metadata uses the LYNX Imager product identity', async () => {
  const [packageJson, tauriJson, html, cargoToml] = await Promise.all([
    read('package.json').then(JSON.parse),
    read('src-tauri/tauri.conf.json').then(JSON.parse),
    read('index.html'),
    read('src-tauri/Cargo.toml'),
  ]);
  assert.equal(packageJson.name, 'lynx-imager');
  assert.equal(tauriJson.productName, 'LYNX Imager');
  assert.match(html, /LYNX Imager/);
  assert.match(cargoToml, /name = "lynx-imager"/);
});
