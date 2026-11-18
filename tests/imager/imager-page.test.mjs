import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import ts from 'typescript';

const root = process.cwd();
const read = (relative) => fs.readFile(path.join(root, relative), 'utf8');

test('LYNX Imager exposes five visible steps and all three workspaces', async () => {
  const source = await read('src/Components/Imager/ImagerPage.tsx');
  const shell = await read('src/main.tsx');
  assert.match(source, /基础烧录/);
  assert.match(shell, /高级烧录/);
  assert.match(shell, /连接 AI/);
  assert.match(source, /\['选择开发板', '选择镜像', '配置接口', '确认烧录', '写入与校验'\]/);
  assert.match(source, /aria-current=.*'step'/);
  assert.match(source, /aria-label="烧录步骤"/);
  assert.match(source, /搜索板型、芯片或厂商/);
  assert.match(source, /imager-board-picker/);
  assert.match(source, /已选板型/);
});

test('basic flow calls existing typed IPC boundaries', async () => {
  const source = await read('src/Components/Imager/ImagerPage.tsx');
  const observation = await read('src/Components/Imager/BasicTask.tsx');
  const catalog = await read('src/Components/Imager/BasicCatalog.tsx');
  assert.match(source, /workbench_list_devices/);
  assert.match(source, /workbench_start_flash/);
  assert.match(source, /imager_file_identity/);
  assert.match(source, /workbench_scan_flasher_devices/);
  assert.match(source, /workbench_firmware_prepare/);
  assert.match(source, /workbench_firmware_start/);
  assert.match(source, /!confirmed \|\| !files/);
  assert.match(source, /fresh\.usb\.key !== target\.usb\.key/);
  assert.match(source, /current\.image\.sha256 !== files\.image\.sha256/);
  assert.match(catalog, /workbench_firmware_catalog/);
  assert.match(catalog, /workbench_firmware_download/);
  assert.match(observation, /workbench-task-event/);
  assert.match(observation, /workbench-flash-telemetry/);
  assert.match(observation, /workbench_list_tasks/);
  assert.match(observation, /taskId !== id/);
  assert.match(observation, /physicalVerifyState/);
  assert.match(observation, /bootVerifyState/);
  assert.doesNotMatch(source, /setInterval\(/);
});

async function basicPlan() {
  const source = await read('src/Components/Imager/BasicPlan.ts');
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  });
  return import('data:text/javascript;base64,' + Buffer.from(outputText).toString('base64'));
}

test('the basic routes submit the native options for the selected vendor and storage', async () => {
  const { flashOptions } = await basicPlan();
  for (const vendor of ['nxp', 'st', 'kendryte'])
    assert.deepEqual(flashOptions('vendor', vendor, ''), { systemType: 'vendor' });
  assert.deepEqual(flashOptions('vendor', 'allwinner', ''), {
    systemType: 'vendor',
    mode: 'full_erase',
    verify: true,
    postFlashAction: 'reboot',
  });
  for (const route of ['mainline_emmc', 'mainline_nand'])
    assert.deepEqual(flashOptions(route, 'allwinner', '  /loader.fex  '), {
      systemType: 'mainline',
      mainlineRoute: route,
      loaderPath: '/loader.fex',
    });
  assert.deepEqual(flashOptions('mainline_spinor', 'allwinner', '/loader.fex'), {
    systemType: 'mainline',
    mainlineRoute: 'mainline_spinor',
    loaderPath: '/loader.fex',
    postFlashAction: 'reboot',
  });
  assert.deepEqual(flashOptions('rockchip_raw_emmc', 'rockchip', '/loader.bin'), {
    systemType: 'mainline',
    mainlineRoute: 'rockchip_raw_emmc',
    loaderPath: '/loader.bin',
    mode: 'full_erase',
    verify: true,
    postFlashAction: 'reboot',
  });
});

test('preflight resolves a single native USB binding and rejects ambiguous hub ports', async () => {
  const { uniqueUsbTarget } = await basicPlan();
  const dev = (key, flasherKind = 'openixcli') => ({ key, flasherKind });
  const full = dev('location:libusb:1:2.3');
  const more = dev('location:libusb:1:4.3');
  assert.equal(uniqueUsbTarget(full.key, 'openixcli', [full, more]), full);
  assert.equal(uniqueUsbTarget('location:libusb:1:3', 'openixcli', [full]), full);
  assert.throws(
    () => uniqueUsbTarget('location:libusb:1:3', 'openixcli', [full, more]),
    /多个物理端口/
  );
  assert.throws(() => uniqueUsbTarget(full.key, 'uuu', [full]), /未检测到/);
  assert.throws(
    () => uniqueUsbTarget('1:3', 'openrockcli', [dev('2:3', 'openrockcli')]),
    /未检测到/
  );
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
