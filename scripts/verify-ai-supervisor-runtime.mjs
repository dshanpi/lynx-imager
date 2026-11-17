import { ensureDesktopIdle } from './lib/desktop-test-safety.mjs';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

const root = path.resolve(import.meta.dirname, '..');
const binary = path.join(root, 'src-tauri', 'target', 'debug', 'lynx-app');
const isolatedRoot = await mkdtemp(path.join(os.tmpdir(), 'lynx-ai-runtime-'));
const evidenceRoot = path.join(root, 'artifacts', 'ai-supervisor-runtime-gate', 'linux');
const port = Number(process.env.LYNX_AI_RUNTIME_DRIVER_PORT || 4466);
const origin = `http://127.0.0.1:${port}`;
let driver;
let socat;
let serialReader;
let sessionId = '';
let serialBytes = Buffer.alloc(0);

const fail = (message) => {
  throw new Error(`AI supervisor runtime gate failed: ${message}`);
};

async function waitFor(description, operation, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs;
  let lastError;
  while (Date.now() < deadline) {
    try {
      const value = await operation();
      if (value) return value;
    } catch (error) {
      lastError = error;
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  fail(`${description} timed out${lastError ? `: ${lastError.message}` : ''}`);
}

async function request(method, endpoint, body, timeoutMs = 30_000) {
  const response = await fetch(`${origin}${endpoint}`, {
    method,
    headers: body === undefined ? undefined : { 'content-type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(timeoutMs),
  });
  const payload = await response.json().catch(() => ({}));
  if (!response.ok || (payload?.value?.error && payload?.value?.message))
    fail(`${method} ${endpoint}: ${payload?.value?.message || response.statusText}`);
  return payload.value;
}

async function execute(script) {
  return request('POST', `/session/${sessionId}/execute/sync`, { script, args: [] });
}

async function invoke(command, args) {
  const result = await request('POST', `/session/${sessionId}/execute/async`, {
    script: `const done = arguments[arguments.length - 1]; window.__TAURI_INTERNALS__.invoke(arguments[0], arguments[1]).then(value => done({ok:true,value})).catch(error => done({ok:false,error:String(error)}));`,
    args: [command, args],
  });
  if (!result?.ok) fail(`${command}: ${result?.error || 'unknown IPC error'}`);
  return result.value;
}

async function click(testId) {
  const clicked = await waitFor(`${testId} clickable`, () =>
    execute(
      `const e=document.querySelector('[data-testid="${testId}"]'); if(!(e instanceof HTMLElement)||e.matches(':disabled')) return false; const style=getComputedStyle(e); const rect=e.getBoundingClientRect(); if(style.display==='none'||style.visibility==='hidden'||Number.parseFloat(style.opacity)<=0||rect.width<=0||rect.height<=0) return false; e.scrollIntoView({block:'nearest'}); e.click(); return true;`
    )
  );
  if (!clicked) fail(`${testId} is not clickable`);
}

function startPseudoSerial() {
  return new Promise((resolve, reject) => {
    const ports = [];
    socat = spawn('socat', ['-d', '-d', 'pty,raw,echo=0', 'pty,raw,echo=0'], {
      stdio: ['ignore', 'ignore', 'pipe'],
    });
    socat.once('error', reject);
    socat.stderr.on('data', (chunk) => {
      for (const match of chunk.toString().matchAll(/PTY is (\/dev\/pts\/\d+)/g))
        ports.push(match[1]);
      if (ports.length >= 2) resolve(ports.slice(0, 2));
    });
  });
}

async function cleanup() {
  if (sessionId) {
    const idle = await waitFor('hardware idle before AI test cleanup', () =>
      ensureDesktopIdle(invoke)
    );
    await writeFile(
      path.join(evidenceRoot, 'safety-before-close.json'),
      JSON.stringify(idle, null, 2)
    );
    await request('DELETE', `/session/${sessionId}`).catch(() => undefined);
  }
  const exits = [];
  for (const process of [serialReader, driver, socat]) {
    if (process && process.exitCode == null) {
      exits.push(new Promise((resolve) => process.once('exit', resolve)));
      process.kill('SIGTERM');
    }
  }
  await Promise.race([Promise.all(exits), new Promise((resolve) => setTimeout(resolve, 3_000))]);
  if (process.env.LYNX_AI_RUNTIME_KEEP_DATA !== '1')
    await rm(isolatedRoot, { recursive: true, force: true }).catch(() => undefined);
}

try {
  await rm(evidenceRoot, { recursive: true, force: true });
  await mkdir(evidenceRoot, { recursive: true });
  const [appPort, peerPort] = await startPseudoSerial();
  serialReader = spawn('cat', [peerPort], { stdio: ['ignore', 'pipe', 'ignore'] });
  serialReader.stdout.on('data', (chunk) => {
    serialBytes = Buffer.concat([serialBytes, chunk]);
  });

  const env = {
    ...process.env,
    XDG_CONFIG_HOME: path.join(isolatedRoot, 'config'),
    XDG_CACHE_HOME: path.join(isolatedRoot, 'cache'),
    XDG_DATA_HOME: path.join(isolatedRoot, 'data'),
    XDG_STATE_HOME: path.join(isolatedRoot, 'state'),
    LYNX_TEST_ALLOW_PARALLEL_INSTANCE: '1',
    LYNX_TEST_AUTH_KEYRING_NAMESPACE: path.basename(isolatedRoot),
    LYNX_TEST_SUPERVISOR_KEYRING_NAMESPACE: path.basename(isolatedRoot),
  };
  driver = spawn('tauri-driver', ['--port', String(port)], { cwd: root, env, stdio: 'inherit' });
  await waitFor('tauri-driver', async () => (await fetch(`${origin}/status`)).ok);
  const session = await request(
    'POST',
    '/session',
    {
      capabilities: { alwaysMatch: { 'tauri:options': { application: binary } } },
    },
    120_000
  );
  sessionId = session.sessionId || session.id;
  await waitFor(
    'Tauri startup',
    () => execute(`return Boolean(document.querySelector('[data-testid="app-sidebar"]'));`),
    30_000
  );
  await click('sidebar-tool-ai-supervisor-automatic');
  await waitFor('AI task workspace', () =>
    execute(`return document.body.innerText.includes('创建任务');`)
  );

  const boardId = await invoke('workbench_save_device', {
    device: {
      id: null,
      boardUid: '',
      name: 'AI Runtime Pseudo Board',
      boardImagePath: '',
      chipVendor: 'allwinner',
      serialPort: appPort,
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
  const boardScope = `linux:${boardId}`;
  const policy = await invoke('supervisor_save_policy', {
    policy: {
      id: null,
      name: 'AI Runtime Gate Policy',
      enabled: true,
      providerId: null,
      maxAttempts: 2,
      maxDurationSeconds: 60,
      maxSameError: 1,
      maxPowerCycles: 0,
      maxFlashRetries: 0,
      waitDeviceSeconds: 5,
      costLimit: null,
      allowedActions: ['collect_logs', 'finish_failed', 'request_human'],
      approvalActions: [],
      boardScope: [boardScope],
      successSummary: false,
      notificationMode: 'desktop',
    },
  });
  const task = await invoke('supervisor_save_automatic_task', {
    task: {
      id: null,
      name: 'AI 串口统一运行链路门禁',
      enabled: true,
      triggerType: 'manual_api',
      trigger: {},
      boardId: boardScope,
      policyId: policy.id,
      nextRunAt: null,
      lastRunAt: null,
      successConditions: [{ kind: 'status', equals: 'success' }],
      taskTemplate: {
        kind: 'device_flow',
        version: 1,
        nodes: [
          { id: 'trigger', type: 'trigger.manual', label: '手动开始', x: 0, y: 0, config: {} },
          {
            id: 'open',
            type: 'serial.open',
            label: '打开测试串口',
            x: 0,
            y: 100,
            config: { boardId: boardScope },
          },
          {
            id: 'send',
            type: 'serial.send',
            label: '发送 PING',
            x: 0,
            y: 200,
            config: {
              boardId: boardScope,
              text: 'PING',
              appendEnter: true,
              approvalMode: 'required',
            },
          },
          {
            id: 'verify',
            type: 'verify.deterministic',
            label: '验证发送结果',
            x: 0,
            y: 300,
            config: { kind: 'output_match', pattern: 'SERIAL_WRITE_ACCEPTED' },
          },
          { id: 'success', type: 'result.success', label: '任务成功', x: 0, y: 400, config: {} },
          { id: 'failed', type: 'result.failed', label: '任务失败', x: 240, y: 400, config: {} },
        ],
        edges: [
          { id: 'e1', source: 'trigger', target: 'open' },
          { id: 'e2', source: 'open', target: 'send' },
          { id: 'e3', source: 'send', target: 'verify' },
          { id: 'e4', source: 'verify', target: 'success', branch: 'success' },
          { id: 'e5', source: 'verify', target: 'failed', branch: 'failure' },
        ],
      },
    },
    webhookSecret: null,
  });
  await waitFor('hardware gate idle after board reconciliation', async () => {
    const status = await invoke('hardware_safety_status');
    return !status.busy;
  });
  // Exercise the same path as an operator: reload the saved task into the
  // editor, approve the browser confirmation, then click "立即运行". This
  // intentionally catches frontend/backend high-risk classification drift.
  await execute('window.location.reload(); return true;');
  await waitFor('Tauri reload after fixture setup', () =>
    execute(
      `return performance.getEntriesByType('navigation')[0]?.type === 'reload' && Boolean(document.querySelector('[data-testid="app-sidebar"]'));`
    )
  );
  await click('sidebar-tool-ai-supervisor-automatic');
  await waitFor('automatic editor activation', () =>
    execute(`return document.body.innerText.includes('设备任务');`)
  );
  await waitFor('saved automatic task in editor', () =>
    execute(
      `return Boolean(document.querySelector('[data-testid="ai-automatic-task-${task.id}"]'));`
    )
  );
  await click(`ai-automatic-task-${task.id}`);
  await waitFor('saved task details in editor', () =>
    execute(`return document.body.innerText.includes('发送 PING');`)
  );
  await execute('window.confirm = () => false; return true;');
  await click('ai-automatic-run');
  const rejectedRuns = await invoke('supervisor_list_runs', { limit: 20 });
  const rejectedTasks = await invoke('workbench_list_tasks', { limit: 20 });
  if (rejectedRuns.length || rejectedTasks.length)
    fail('declining high-risk UI approval still started or registered a task');
  await execute('window.confirm = () => true; return true;');
  await click('ai-automatic-run');

  const run = await waitFor('unified supervisor run', async () => {
    const runs = await invoke('supervisor_list_runs', { limit: 20 });
    return runs.find((item) => item.boardId === boardScope);
  });
  const taskId = run.taskId;
  const workbenchTask = await waitFor('Workbench task terminal state', async () => {
    const tasks = await invoke('workbench_list_tasks', { limit: 20 });
    return tasks.find(
      (item) => item.taskId === taskId && ['success', 'failed', 'cancelled'].includes(item.status)
    );
  });
  if (workbenchTask.status !== 'success')
    fail(`Workbench task ended as ${workbenchTask.status}: ${workbenchTask.error}`);
  const finalRun = await waitFor('Supervisor terminal state', async () => {
    const runs = await invoke('supervisor_list_runs', { limit: 20 });
    return runs.find((item) => item.id === run.id && item.status === 'success');
  });
  const steps = await invoke('workbench_list_workflow_steps', { taskId });
  if (!steps.some((step) => step.stepName.endsWith(':发送 PING') && step.status === 'success'))
    fail('serial execution step is missing from Workbench history');
  await waitFor('pseudo serial bytes', () => serialBytes.includes(Buffer.from('PING\n')));
  const events = await invoke('record_center_list_events', {
    filter: { sessionId: taskId, limit: 100 },
  });
  if (!events.some((event) => event.action === 'serial.tx'))
    fail('serial.tx task audit is missing');

  await click('sidebar-tool-ai-supervisor-runtime');
  await waitFor('run center step visibility', () =>
    execute(
      `return document.body.innerText.includes('发送 PING') && document.body.innerText.includes('设备任务实时步骤');`
    )
  );
  const screenshot = Buffer.from(
    await request('GET', `/session/${sessionId}/screenshot`),
    'base64'
  );
  await writeFile(path.join(evidenceRoot, 'ai-supervisor-unified-runtime.png'), screenshot);
  const evidence = {
    generatedAt: new Date().toISOString(),
    isolatedData: true,
    pseudoSerial: true,
    triggeredViaUi: true,
    declinedApprovalCreatedNoTask: true,
    taskId,
    runId: finalRun.id,
    workbenchStatus: workbenchTask.status,
    supervisorStatus: finalRun.status,
    steps: steps.map(({ stepName, status, error }) => ({ stepName, status, error })),
    serialPayloadSha256: createHash('sha256').update(serialBytes).digest('hex'),
    serialTxAudit: true,
    screenshot: 'ai-supervisor-unified-runtime.png',
    result: 'PASS',
  };
  await writeFile(
    path.join(evidenceRoot, 'evidence.json'),
    `${JSON.stringify(evidence, null, 2)}\n`
  );
  process.stdout.write(`AI supervisor runtime gate passed: ${taskId} / ${finalRun.id}\n`);
} finally {
  await cleanup();
}
