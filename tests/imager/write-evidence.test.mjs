import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import ts from 'typescript';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

const source = await readFile(
  new URL('../../src/Components/Imager/writeEvidence.ts', import.meta.url),
  'utf8'
);
const { outputText } = ts.transpileModule(source, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
});
const {
  failureEvidence,
  physicalPlan,
  physicalReadback,
  inspectSerialEvidence,
  sameBootScope,
  selectBootAcceptance,
  retainEvidence,
} = await import('data:text/javascript;base64,' + Buffer.from(outputText).toString('base64'));
const hash = '24b7280c797c7859bce5329a8e253210f95fdee7a2d60d4902c2ffc6fb31e256';
const fail = (logs, details = {}) =>
  failureEvidence({ logs, details, status: 'failed', error: '' });
const policy = [
  'RAW_SPINOR_CONTRACT:capacity=16777216,eraseAlignment=65536,writeBytes=655360',
  'RAW_SPINOR_EVIDENCE:test-evidence',
  'RAW_SPINOR_FES_STEP:erase_policy_download:start,bytes=16,attempt=1',
  'RAW_SPINOR_ERASE_POLICY_DOWNLOAD_FAILED:elapsedMs=0,source=UsbTransfer',
];

test('policy transport failure scopes 16 bytes to the request and preserves preceding backup evidence', () => {
  assert.deepEqual(physicalPlan(policy, 614400), {
    capacity: 16777216,
    alignment: 65536,
    span: 655360,
    blocks: 10,
    keep: 40960,
  });
  assert.equal(physicalPlan([], 614400), null);
  assert.equal(physicalPlan(policy, 655361), null);
  assert.equal(
    physicalPlan(['RAW_SPINOR_CONTRACT:capacity=2048,eraseAlignment=512,writeBytes=2000'], 2000),
    null
  );
  const retained = retainEvidence([], policy);
  const diagnostic = failureEvidence({
    status: 'failed',
    logs: Array(80).fill('cleanup'),
    evidenceLines: retainEvidence(retained, Array(100).fill('other')),
    details: {},
    error: '',
  });
  assert.match(diagnostic.title, /擦除策略下发失败/);
  assert.match(diagnostic.impact, /备份已完成.*尚未执行.*镜像写入/);
  assert.match(diagnostic.evidence, /16 B.*UsbTransfer/);
  assert.doesNotMatch(fail(policy.slice(2)).impact, /备份已完成/);
  assert.doesNotMatch(
    fail([...policy, 'RAW_SPINOR_FES_STEP:erase_policy_download:success']).impact,
    /尚未执行/
  );
  assert.doesNotMatch(fail([...policy, 'RAW_MBR_DOWNLOAD_START']).impact, /尚未执行/);
});

test('physical layout refusal, read error, mismatch and interruption keep different damage boundaries', () => {
  assert.match(fail(['RAW_SPINOR_PHYSICAL_LAYOUT_UNVERIFIED']).impact, /擦除和写入前/);
  const mismatch = fail([
    'RAW_WRITE_COMPLETE:medium=spinor',
    'RAW_SPINOR_PHYSICAL_VERIFY_FAILED: byte 524288 mismatch',
  ]);
  assert.match(mismatch.title, /不一致/);
  assert.match(mismatch.impact, /写入完成/);
  assert.match(fail(['RAW_SPINOR_PHYSICAL_VERIFY_FAILED: USB disconnected']).title, /回读失败/);
  assert.match(fail([], { progressPct: 100 }).impact, /不足以确定/);
  assert.match(fail([], { writeState: 'running' }).impact, /写入已开始/);
});

test('physical proof requires native success and matches the selected full SHA, regardless of job progress', () => {
  const lines = [
    'RAW_SPINOR_PHYSICAL_VERIFY_SUCCESS:imageSha256=' +
      hash +
      ',imageBytes=614400,fullFlashBytes=16777216,outsideImage=unchanged',
  ];
  assert.equal(physicalReadback({ progressPct: 100 }, lines, hash), null);
  assert.deepEqual(
    physicalReadback({ physicalVerifyState: 'success' }, lines, hash.toUpperCase()),
    { matches: true, imageHash: hash, fullBytes: 16777216, outsidePreserved: true }
  );
  assert.equal(
    physicalReadback({ physicalVerifyState: 'success' }, lines, '1'.repeat(64)).matches,
    false
  );
  assert.equal(physicalReadback({ physicalVerifyState: 'success' }, [], hash).matches, null);
});

test('a reboot command and another image’s console output cannot stand in for the expected application', () => {
  const marker = 'F101-FLASH-PROBE-20261029-A';
  assert.equal(
    inspectSerialEvidence('reboot command_completed\nNekoBox', 'F101-FLASH-PROBE'),
    null
  );
  assert.equal(inspectSerialEvidence('# echo ' + marker, marker), null);
  assert.equal(inspectSerialEvidence('hello', 'he'), null);
  const one = marker + ' HEARTBEAT 1 uptime_ms=5018';
  const two = marker + ' HEARTBEAT 4 uptime_ms=20033';
  assert.deepEqual(inspectSerialEvidence(one + '\r\n' + two, marker), {
    count: 2,
    first: one,
    last: two,
  });
});

test('startup acceptance stays with the exact task, device and image', () => {
  const scope = { taskId: 'flash-1', deviceId: 42, imageSha256: hash };
  assert.ok(sameBootScope(scope, { ...scope, imageSha256: hash.toUpperCase() }));
  for (const change of [{ taskId: 'flash-2' }, { deviceId: 43 }, { imageSha256: '1'.repeat(64) }])
    assert.equal(sameBootScope(scope, { ...scope, ...change }), false);
  const base = {
    taskId: scope.taskId,
    imageSha256: hash,
    candidateId: 'candidate-1',
    recordedAt: 42,
    operator: 'QA',
  };
  const passed = {
    ...base,
    items: [
      { item: 'boot', result: 'pass', evidence: 'application build k1' },
      { item: 'power_cycle', result: 'pass', evidence: 'cold boot log' },
    ],
  };
  const failed = {
    ...base,
    items: [{ item: 'boot', result: 'fail', evidence: 'old firmware started' }],
  };
  assert.equal(selectBootAcceptance([failed, passed], scope.taskId, 42, hash)?.result, 'fail');
  assert.equal(selectBootAcceptance([passed], scope.taskId, 42, hash, 'candidate-2'), null);
  assert.equal(selectBootAcceptance([passed], scope.taskId, 42, hash)?.coldBoot, true);
});

test('actual React pages render confirm, execution, independent result stages and actionable failures', async () => {
  const server = await createServer({
    server: { middlewareMode: true, hmr: false },
    appType: 'custom',
    logLevel: 'silent',
  });
  try {
    const { V2Write, WritePreflightFailure } = await server.ssrLoadModule(
      '/src/Components/Imager/V2Write.tsx'
    );
    const snapshot = {
      board: 'F101',
      chip: 'F101',
      vendor: '全志',
      storage: 'SPI NOR',
      route: '物理写入',
      imagePath: '/probe.img',
      imageSize: 614400,
      imageSha256: hash,
      device: 'F101 B',
      deviceId: 43,
      usb: 'USB 2-2.5',
      mode: 'FEL',
      verify: '独立回读',
      postAction: 'reboot',
    };
    const noop = () => {};
    const view = (task, changes = {}) =>
      renderToStaticMarkup(
        createElement(V2Write, {
          snapshot: { ...snapshot, ...changes },
          task,
          confirmed: false,
          busy: false,
          onConfirmChange: noop,
          onStart: noop,
          onBack: noop,
          onRetry: noop,
        })
      );
    const task = {
      id: 'flash-sim',
      status: 'running',
      phase: 'writing',
      details: { progressPct: 12 },
      logs: [],
      evidenceLines: [],
      logVersion: 0,
      startedAt: null,
      finishedAt: null,
      observationError: '',
      error: '',
    };
    const confirm = view(undefined);
    assert.match(confirm, /确认烧录/);
    assert.match(confirm, /未?提交后，烧录器将检查实际介质/);
    const rk = view(undefined, {
      eraseEmmcUserArea: true,
      storage: 'eMMC',
      targetDetails: [['物理端口', '2-2.5']],
    });
    assert.match(rk, /将完整擦除目标 eMMC 用户区/);
    assert.match(rk, /物理端口/);
    assert.doesNotMatch(confirm, /将完整擦除目标 eMMC 用户区/);
    assert.match(view(task), /镜像有效载荷/);
    assert.match(view(task), /600 KiB/);
    const success = {
      ...task,
      status: 'success',
      details: {
        writeState: 'success',
        verifyState: 'success',
        physicalVerifyState: 'success',
        physicalImageSha256: hash,
        storageType: 'spinor',
        postFlashAction: 'reboot',
        postFlashState: 'success',
        bootVerifyState: 'not_verified',
      },
    };
    assert.match(view(success), /烧写完成，启动待验证/);
    assert.doesNotMatch(view(success), /写入完成，启动已验证/);
    assert.match(
      view({ ...success, details: { ...success.details, bootVerifyState: 'verified' } }),
      /写入完成，启动已验证/
    );
    assert.match(
      view({ ...success, details: { ...success.details, physicalImageSha256: '1'.repeat(64) } }),
      /指纹与所选镜像不一致/
    );
    const preflight = renderToStaticMarkup(
      createElement(WritePreflightFailure, {
        context: { ...snapshot, usbId: '1f3a:efe8' },
        message: '目标 USB 已断开',
        onReconnect: noop,
      })
    );
    assert.match(preflight, /预检时失去目标设备/);
    assert.match(preflight, /未提交烧录任务/);
    assert.match(preflight, /1f3a:efe8/);
    assert.match(preflight, /上次选定的目标/);
    assert.equal((preflight.match(/<button/g) || []).length, 1);
    assert.match(view({ ...task, status: 'failed', logs: policy }), /擦除策略下发失败/);
    assert.match(
      view({
        ...task,
        status: 'failed',
        logs: ['RAW_SPINOR_PHYSICAL_VERIFY_FAILED: byte 0 mismatch'],
      }),
      /物理回读与预期不一致/
    );
    assert.match(view({ ...task, status: 'cancelled' }), /任务已取消/);
  } finally {
    await server.close();
  }
});
