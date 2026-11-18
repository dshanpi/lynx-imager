import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import ts from 'typescript';
const source = await readFile(
  new URL('../../src/Components/Imager/vendorPlan.ts', import.meta.url),
  'utf8'
);
const compiled = ts.transpileModule(source, {
  compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.ESNext },
}).outputText;
const isolated = compiled.replace(
  /import\s*\{\s*invokeCommand\s*\}\s*from\s*['"][^'"]+['"]\s*;/,
  "const invokeCommand=()=>{throw new Error('unexpected IPC in pure reference test')};"
);
const { defaultVendorSelection, vendorRoutes, vendorGate, vendorFlashOptions, vendorNeedsLoader } =
  await import('data:text/javascript;base64,' + Buffer.from(isolated).toString('base64'));
const inspect = (route) => ({
  route,
  path: '/tmp/system.img',
  format: 'fixture',
  bytes: 4096,
  items: [],
  alternates: [],
  dependencies: [],
  blockers: [],
});
const dev = (flasherKind, mode = 'loader') => ({
  key: '2:3.1',
  flasherKind,
  mode,
  dfuAlternates: [],
});

test('every vendor route has its own inputs and a matching successful file inspection', () => {
  assert.equal(Object.values(vendorRoutes).flat().length, 8);
  for (const [vendor, routes] of Object.entries(vendorRoutes)) {
    for (const { route } of routes) {
      const s = { ...defaultVendorSelection(vendor), route };
      const i = inspect(route);
      if (route === 'st-dfuse') i.alternates = [1];
      assert.equal(vendorGate(s, i), '');
      assert.ok(vendorGate(s, { ...i, blockers: ['missing'] }));
      assert.ok(vendorGate(s, { ...i, route: 'stale' }));
      assert.equal(vendorNeedsLoader(route), route === 'rk-emmc');
      assert.ok(vendorGate(s, i, null, 'target'));
    }
  }
});
test('RK raw requires target-specific erasure confirmation; update uses its own package', () => {
  const s = {
    ...defaultVendorSelection('rockchip'),
    route: 'rk-emmc',
    eraseConfirmed: true,
    targetKey: '2:3.1',
  };
  const u = dev('openrockcli', 'maskrom');
  assert.equal(vendorGate(s, inspect(s.route), u, 'target'), '');
  assert.ok(vendorGate({ ...s, eraseConfirmed: false }, inspect(s.route), u, 'target'));
  assert.ok(vendorGate(s, inspect(s.route), { ...u, key: '2:3.2' }, 'target'));
  assert.ok(vendorGate(s, inspect(s.route), { ...u, mode: 'adb' }, 'target'));
  const opts = vendorFlashOptions(s, '/rk-loader.bin');
  assert.equal(opts.mainlineRoute, 'rockchip_raw_emmc');
  assert.equal(opts.verify, true);
  assert.deepEqual(opts.vendorSetup, {
    route: 'rk-emmc',
    targetKey: '2:3.1',
    eraseConfirmed: true,
  });
  assert.equal(
    vendorFlashOptions({ ...s, route: 'rk-update' }, '/rk-loader.bin').loaderPath,
    undefined
  );
});
test('NXP only allows enumerated built-in recipe for a configured single image and carries UUU script reference evidence', () => {
  const s = defaultVendorSelection('nxp');
  const i = inspect(s.route);
  i.dependencies = [{ path: 'os.wic', bytes: 512, sha256: '0'.repeat(64) }];
  assert.deepEqual(vendorFlashOptions(s, '', i).vendorSetup.dependencies, i.dependencies);
  s.route = 'nxp-configured';
  const usb = dev('uuu', 'sdps');
  const gate = (flasherArgs) => vendorGate(s, inspect(s.route), usb, 'target', { flasherArgs });
  assert.equal(gate('["-b","emmc"]'), '');
  for (const args of ['[]', 'not-json', '["-b","not-a-recipe"]', '["-b","emmc","unreviewed.bin"]'])
    assert.ok(gate(args));
});
test('DFU matches device and package Alternate and rejects ambiguous target or invalid RAW address', () => {
  const s = { ...defaultVendorSelection('st'), alt: '0', targetKey: '2:3.1' };
  const u = { ...dev('dfu-util', 'dfu'), dfuAlternates: [{ number: 0, name: 'filesystem' }] };
  const i = { ...inspect(s.route), alternates: [0] };
  assert.equal(vendorGate(s, i, u, 'target'), '');
  for (const bad of [
    { ...u, key: '3:4' },
    { ...u, dfuAlternates: [] },
    { ...u, dfuAlternates: [...u.dfuAlternates, ...u.dfuAlternates] },
  ])
    assert.ok(vendorGate(s, i, bad, 'target'));
  assert.ok(vendorGate(s, { ...i, alternates: [1] }, u, 'target'));
  assert.ok(vendorGate(s, { ...i, alternates: [0, 2] }, u, 'target'));
  s.route = 'st-raw';
  for (const address of ['', '1234', '0x100000000', '0x15:mass-erase', '0xZZ'])
    assert.ok(vendorGate({ ...s, address }, inspect(s.route), u, 'target'));
  s.address = '0x08001000';
  assert.equal(vendorGate(s, inspect(s.route), u, 'target'), '');
  assert.equal(vendorFlashOptions(s, '').vendorSetup.address, s.address);
});
test('K230 default and USB port discovery do not select a medium', () => {
  const s = defaultVendorSelection('kendryte');
  const i = inspect(s.route);
  const u = dev('k230', 'bootrom / loader');
  assert.ok(vendorGate(s, i, u, 'target'));
  for (const medium of ['emmc', 'sdcard', 'spi_nand', 'spi_nor']) {
    const selected = { ...s, medium, targetKey: u.key };
    assert.equal(vendorGate(selected, i, u, 'target'), '');
    assert.equal(vendorFlashOptions(selected, '').vendorSetup.medium, medium);
    assert.ok(vendorGate(selected, i, { ...u, key: '1:4' }, 'target'));
  }
  assert.ok(vendorGate({ ...s, medium: 'otp', targetKey: u.key }, i, u, 'target'));
});
