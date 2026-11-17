import { createImmutableDesktopTestBinary } from './lib/desktop-test-binary.mjs';
import { ensureDesktopIdle } from './lib/desktop-test-safety.mjs';
// Real Tauri IPC + loopback SSH. Generated test course; no cloud grades or hardware.
import { spawn, spawnSync } from 'node:child_process';
import {
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  realpath,
  writeFile,
  unlink,
  symlink,
  stat,
  chmod,
  rename,
} from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { createServer } from 'node:net';
import path from 'node:path';
import os from 'node:os';
import assert from 'node:assert/strict';
import { prepareIdeFixture, verifyIdeFixture } from './learning-ide-fixture.mjs';
import { verifyHilTrustFixture } from './learning-hil-trust-fixture.mjs';
import { verifyEnvironmentFixture } from './learning-environment-fixture.mjs';
import { verifyEnvironmentExecution } from './learning-environment-execution-fixture.mjs';
import { verifyRevisit } from './learning-revisit-fixture.mjs';
import { verifyPrerequisites } from './learning-prerequisites-fixture.mjs';

const root = process.cwd();
const scenario = process.env.LYNX_LEARN_RUNTIME_SCENARIO || 'c';
assert.ok(
  ['c', 'multimedia', 'theory', 'mixed', 'bsp', 'optional'].includes(scenario),
  'Unknown course scenario'
);
const realProvider = process.env.LYNX_LEARN_RUNTIME_REAL_PROVIDER === '1';
const ideExtensions = process.env.LYNX_LEARN_RUNTIME_IDE_EXTENSIONS;
assert.ok(!ideExtensions || scenario === 'c', 'IDE acceptance uses the generated C course');
let ideFixture;
const hilTrust = process.env.LYNX_LEARN_RUNTIME_HIL_TRUST === '1';
const environmentFixture = process.env.LYNX_LEARN_RUNTIME_ENVIRONMENT === '1';
const environmentExecution = process.env.LYNX_LEARN_RUNTIME_ENVIRONMENT_EXECUTION === '1';
const prerequisitesFixture = process.env.LYNX_LEARN_RUNTIME_PREREQUISITES === '1';
assert.ok(
  !prerequisitesFixture || scenario === 'c',
  'Prerequisite acceptance extends the C course'
);
assert.ok(
  !environmentExecution || scenario === 'c',
  'Environment execution fixture follows the C course'
);
assert.ok(!environmentFixture || scenario === 'c', 'Environment fixture follows the C course');
assert.ok(!hilTrust || scenario === 'c', 'HIL trust fixture follows the generated C course');
assert.ok(
  !realProvider || scenario === 'multimedia',
  'Live AI test sends only the generated media course'
);
const output = path.resolve(
  process.env.LYNX_LEARN_RUNTIME_OUTPUT || '.work/learning-local-runtime'
);
const data = await mkdtemp(path.join(os.tmpdir(), 'lynx-learn-runtime-'));
const sourceBinary = await realpath(path.join(root, 'src-tauri/target/debug/lynx-app'));
const binarySnapshot = await createImmutableDesktopTestBinary(sourceBinary, path.basename(data));
const binary = binarySnapshot.binary;
const packages = path.join(data, 'packages');
const sdk = path.join(packages, 'prepared-sdk');
const env = {
  ...process.env,
  XDG_CONFIG_HOME: data + '/config',
  XDG_CACHE_HOME: data + '/cache',
  XDG_DATA_HOME: data + '/data',
  XDG_STATE_HOME: data + '/state',
  LYNX_TEST_ALLOW_PARALLEL_INSTANCE: '1',
  LYNX_TEST_AUTH_KEYRING_NAMESPACE: path.basename(data),
};
let driver,
  sshd,
  session = '',
  origin = '',
  driverLog = '';
const evidence = {
  startedAt: new Date().toISOString(),
  data,
  binary,
  sourceBinary,
  immutableSnapshotSha256: binarySnapshot.sha256,
  syntheticCourse: true,
  mockedIpc: false,
  hardwareActionsExecuted: false,
  cloudGradesWritten: false,
  checks: [],
};
await mkdir(output, { recursive: true });
await mkdir(path.join(data, 'config'), { recursive: true });
await mkdir(path.join(data, 'downloads'));
await writeFile(path.join(data, 'config/user-dirs.dirs'), `XDG_DOWNLOAD_DIR="${data}/downloads"\n`);
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
function run(command, args) {
  const result = spawnSync(command, args, { encoding: 'utf8', cwd: root });
  assert.equal(result.status, 0, `${command}: ${result.stderr}`);
  return result.stdout;
}
// Hash a consistent read-only snapshot of the isolated learning database.
// learning_get_journey itself refreshes archive generation time, so comparing
// two projections cannot establish that another command is read-only.
function learningStoreDigest() {
  return run('python3', [
    '-c',
    `import hashlib,sqlite3,sys
from pathlib import Path
p=Path(sys.argv[1]); assert p.is_file()
db=sqlite3.connect(p.as_uri()+'?mode=ro',uri=True)
db.execute('BEGIN')
print(hashlib.sha256(''.join(db.iterdump()).encode()).hexdigest())
db.close()`,
    path.join(data, 'data/com.dshanpi.lynx/learning.sqlite'),
  ]).trim();
}
function freePort() {
  return new Promise((resolve, reject) => {
    const server = createServer();
    server.on('error', reject);
    server.listen(0, '127.0.0.1', () => {
      const port = server.address().port;
      server.close(() => resolve(port));
    });
  });
}
async function wait(fn, description, timeoutMs = 30000) {
  const end = Date.now() + timeoutMs;
  let last;
  while (Date.now() < end) {
    try {
      const value = await fn();
      if (value) return value;
    } catch (error) {
      last = error;
    }
    await sleep(150);
  }
  throw new Error(`${description}: ${last || 'timed out'}`);
}
async function req(method, route, body) {
  const response = await fetch(origin + route, {
    method,
    headers: { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(90000),
  });
  const value = await response.json();
  if (!response.ok || value.value?.error) throw new Error(JSON.stringify(value));
  return value.value;
}
const execute = (script, args = []) =>
  req('POST', `/session/${session}/execute/sync`, { script, args });
async function ipc(command, args = {}) {
  const result = await req('POST', `/session/${session}/execute/async`, {
    script: `const done=arguments[arguments.length-1];
      window.__TAURI_INTERNALS__.invoke(arguments[0],arguments[1])
        .then(value=>done({ok:true,value})).catch(error=>done({ok:false,error:String(error)}));`,
    args: [command, args],
  });
  if (!result.ok) throw new Error(result.error);
  return result.value;
}
async function reject(command, args, expected) {
  await assert.rejects(() => ipc(command, args), expected);
  evidence.checks.push({ name: `${command} rejects ${expected.source}`, status: 'PASS' });
}
async function shot(name) {
  const png = await req('GET', `/session/${session}/screenshot`);
  await writeFile(path.join(output, `${name}.png`), Buffer.from(png, 'base64'));
}
async function clickText(text) {
  await wait(
    () =>
      execute(
        `const button=[...document.querySelectorAll('button')].find(e=>
    e.textContent.trim()===arguments[0] && e.getClientRects().length && !e.disabled);
    if(!button)return false;button.click();return true;`,
        [text]
      ),
    `button ${text}`
  );
}
async function startApp() {
  const port = await freePort();
  origin = `http://127.0.0.1:${port}`;
  const nativePort = await freePort();
  driver = spawn('tauri-driver', ['--port', String(port), '--native-port', String(nativePort)], {
    cwd: root,
    env,
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  driver.stdout.on('data', (bytes) => {
    driverLog += bytes;
  });
  driver.stderr.on('data', (bytes) => {
    driverLog += bytes;
  });
  await wait(() => req('GET', '/status'), 'driver startup');
  const created = await req('POST', '/session', {
    capabilities: { alwaysMatch: { 'tauri:options': { application: binary } } },
  });
  session = created.sessionId || created.id;
  assert.ok(session);
  await writeFile(
    path.join(output, 'control.json'),
    JSON.stringify({ origin, session, data, driverPid: driver.pid, sshdPid: sshd?.pid })
  );
  await wait(
    () => execute('return !!document.querySelector("[data-testid=app-sidebar]");'),
    'app startup'
  );
  assert.equal(await execute('return location.origin;'), 'tauri://localhost');
  await req('POST', `/session/${session}/window/rect`, { width: 1440, height: 1000 });
  await wait(async () => {
    evidence.safetyAtStartup = await ipc('hardware_safety_status');
    return evidence.safetyAtStartup.busy === false;
  }, 'startup hardware discovery becomes idle');
}
async function closeApp() {
  if (!session) return;
  await wait(async () => {
    evidence.desktopIdleBeforeClose = await ensureDesktopIdle(ipc);
    evidence.safetyBeforeClose = evidence.desktopIdleBeforeClose.hardware;
    return evidence.safetyBeforeClose.busy === false;
  }, 'hardware becomes idle before closing owned test app');
  assert.equal(
    evidence.safetyBeforeClose.busy,
    false,
    'Never stop an app while hardware work is active'
  );
  // Stop only this exact binary and this newly created profile. The safety command
  // includes persisted task_history, legacy flash, production, MCU and process gates.
  const owned = [];
  for (const entry of await readdir('/proc')) {
    if (!/^\d+$/.test(entry)) continue;
    try {
      if ((await realpath(`/proc/${entry}/exe`)) !== binary) continue;
      const processEnv = (await readFile(`/proc/${entry}/environ`, 'utf8')).split('\0');
      if (processEnv.includes(`XDG_DATA_HOME=${env.XDG_DATA_HOME}`)) owned.push(Number(entry));
    } catch {
      /* A process can exit while /proc is inspected. */
    }
  }
  assert.equal(owned.length, 1, 'Expected exactly one app owned by this isolated test');
  process.kill(owned[0], 'SIGTERM');
  await wait(async () => !(await readdir('/proc')).includes(String(owned[0])), 'test app exit');
  await req('DELETE', `/session/${session}`).catch(() => {});
  session = '';
  driver.kill('SIGTERM');
  await wait(() => driver.exitCode !== null || driver.signalCode !== null, 'driver exit');
}
async function reloadAppPage() {
  const previous = await execute('return performance.timeOrigin;');
  await execute('location.reload();');
  await wait(
    () =>
      execute(
        `return performance.timeOrigin !== arguments[0] && document.readyState === 'complete' && !!document.querySelector('[data-testid=app-sidebar]');`,
        [previous]
      ),
    'new application document after reload'
  );
}
async function openLocalSetup() {
  await wait(
    () =>
      execute(`const button=document.querySelector('[data-testid=sidebar-tool-learning-labs]');
    if(!button || !button.getClientRects().length)return false;button.click();return true;`),
    'learning labs navigation ready'
  );
  await wait(
    () => execute('return !!document.querySelector("[data-testid=learn-labs]");'),
    'learning labs'
  );
  await execute('document.querySelector("details.learn-course-link").open=true;');
  await clickText('导入离线课程包');
  await wait(
    () => execute('return !!document.querySelector("[data-testid=learn-session-setup]");'),
    'local setup'
  );
}
async function showSession(targetSessionId) {
  await openLocalSetup();
  const sessions = await ipc('learning_list_local_sessions');
  const index = targetSessionId
    ? sessions.findIndex((item) => item.sessionId === targetSessionId)
    : 0;
  assert.ok(index >= 0, 'requested local session must exist');
  await wait(
    () =>
      execute(
        `const buttons=document.querySelectorAll('.local-course-session-list button');
    if(buttons.length!==arguments[1])return false;
    const button=buttons[arguments[0]];
    if(!button)return false;button.click();return true;`,
        [index, sessions.length]
      ),
    'resume local session'
  );
  await wait(
    () => execute('return !!document.querySelector(".local-course-session-shell");'),
    'local session'
  );
}

async function readArtifact(scope, meta) {
  const parts = [];
  let offset = 0;
  while (offset < meta.bytes) {
    const part = await ipc('learning_read_local_artifact_chunk', {
      request: { scope, artifactId: meta.spec.artifactId, sha256: meta.sha256, offset },
    });
    assert.deepEqual(part.scope, scope);
    assert.equal(part.artifactId, meta.spec.artifactId);
    assert.equal(part.sha256, meta.sha256);
    assert.equal(part.offset, offset);
    assert.equal(part.bytes, meta.bytes);
    const bytes = Buffer.from(part.dataBase64, 'base64');
    assert.equal(bytes.length, Math.min(65536, meta.bytes - offset));
    assert.equal(part.nextOffset, offset + bytes.length);
    parts.push(bytes);
    offset = part.nextOffset;
  }
  const bytes = Buffer.concat(parts);
  assert.equal(createHash('sha256').update(bytes).digest('hex'), meta.sha256);
  return bytes;
}

async function showArtifact(scope, artifactId) {
  await clickText('运行与证据');
  await wait(
    () =>
      execute(
        `const panel=[...document.querySelectorAll('.local-artifacts')].find(p=>p.dataset.artifactDigest===arguments[0]);
    const button=panel?.querySelector('button');if(!button||button.disabled)return false;button.click();return true;`,
        [scope.digest]
      ),
    'artifact list button'
  );
  await wait(
    () =>
      execute(
        `const panel=[...document.querySelectorAll('.local-artifacts')].find(p=>p.dataset.artifactDigest===arguments[0]);
    const item=[...(panel?.querySelectorAll('.local-artifact-item')||[])].find(p=>p.dataset.artifactId===arguments[1]);
    const button=item?.querySelector('button');if(!button||button.disabled)return false;button.click();return true;`,
        [scope.digest, artifactId]
      ),
    'artifact read button'
  );
  await wait(
    () =>
      execute(
        `const panel=[...document.querySelectorAll('.local-artifacts')].find(p=>p.dataset.artifactDigest===arguments[0]);
    return !!panel?.querySelector('.local-artifact-preview a[download]');`,
        [scope.digest]
      ),
    'verified artifact preview'
  );
  await execute(
    `const panel=[...document.querySelectorAll('.local-artifacts')].find(p=>p.dataset.artifactDigest===arguments[0]);
    panel.querySelector('.local-artifact-preview').scrollIntoView({block:'center'});`,
    [scope.digest]
  );
}

async function fillLearningText(value) {
  await execute(
    `const field=document.querySelector('.learn-action-area:not([hidden]) textarea');
    if(!field)throw new Error('Learning input is missing');
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set.call(field,arguments[0]);
    field.dispatchEvent(new Event('input',{bubbles:true}));field.dispatchEvent(new Event('change',{bubbles:true}));`,
    [value]
  );
}
async function verifyTheory() {
  assert.equal((await ipc('workbench_list_hosts')).length, 0);
  assert.equal((await ipc('workbench_list_projects')).length, 0);
  for (const [name, error] of [
    ['bad-signature', /LOCAL_COURSE_SIGNATURE/],
    ['bad-path', /LOCAL_COURSE_(UNSAFE_PATH|PATH_INVALID)/],
    ['bad-provenance', /LOCAL_COURSE_.*(HASH|SOURCE|PROVENANCE)/],
    ['bad-command', /LOCAL_COURSE_REFLECTION_INVALID/],
  ])
    await reject(
      'learning_import_local_course',
      { path: packages + '/' + name + '.lynx-course' },
      error
    );
  assert.equal((await ipc('learning_list_local_courses')).length, 0);
  evidence.installed = await ipc('learning_import_local_course', {
    path: packages + '/course.lynx-course',
  });
  assert.equal(evidence.installed.entryRequiresProject, false);
  await openLocalSetup();
  await wait(
    () =>
      execute(
        `const button=[...document.querySelectorAll('button')].find(e=>e.textContent.trim()==='创建学习会话');return button && !button.disabled;`
      ),
    'theory start without project'
  );
  assert.equal(
    await execute(
      `return !!document.querySelector('.local-course-picker section:nth-child(2) select');`
    ),
    false
  );
  await shot('theory-setup');
  await clickText('创建学习会话');
  await wait(
    () => execute('return !!document.querySelector(".local-course-session-shell");'),
    'theory session'
  );
  const sessions = await ipc('learning_list_local_sessions');
  assert.equal(sessions.length, 1);
  const sessionId = sessions[0].sessionId;
  assert.equal(sessions[0].projectId, 0);
  assert.equal(sessions[0].deviceId, null);
  evidence.checks.push({
    name: 'native UI starts signed theory course without host, SDK or board',
    status: 'PASS',
  });
  await reject(
    'learning_run_step_action',
    { sessionId, credentials: {} },
    /LEARNING_STEP_NOT_PRACTICING/
  );
  await reject(
    'learning_bind_local_project',
    { sessionId, projectId: 1 },
    /LEARNING_PROJECT_BINDING_NOT_ALLOWED/
  );
  await ipc('learning_save_draft', {
    sessionId,
    stepId: 'source',
    attempt: 1,
    answer: '理论课重启草稿',
    reflection: '待完成复盘',
  });
  await closeApp();
  await startApp();
  assert.equal((await ipc('learning_get_journey', { sessionId })).answer, '理论课重启草稿');
  await showSession();
  await execute(
    `const button=[...document.querySelectorAll('.learn-workspace-tabs button')].find(e=>e.textContent.trim()==='学习记录');if(!button)throw new Error('Theory record tab missing');button.click();`
  );
  await wait(
    () =>
      execute(
        `return document.querySelector('[data-testid=learn-evidence] h1')?.textContent==='本活动的学习记录';`
      ),
    'theory evidence view'
  );
  assert.equal(
    await execute(
      `return !!document.querySelector('[data-testid=learn-evidence] .learn-camera-column');`
    ),
    false
  );
  assert.equal(
    await execute(
      `return document.querySelector('[data-testid=learn-evidence]').textContent.includes('工程 #0');`
    ),
    false
  );
  await clickText('教材');
  evidence.checks.push({
    name: 'theory record view exposes no camera or execution target',
    status: 'PASS',
  });
  evidence.checks.push({
    name: 'theory draft and unbound session survive native restart',
    status: 'PASS',
  });
  for (const stepId of ['source', 'build', 'run']) {
    await wait(
      async () =>
        (await ipc('learning_get_local_step', { sessionId })).session.currentStep === stepId,
      'current reflection step'
    );
    await clickText('已阅读，进入理解检查');
    await wait(
      () =>
        execute(
          `return [...document.querySelectorAll('button')].some(e=>e.textContent.trim()==='保存理解回答并复盘');`
        ),
      'understanding input'
    );
    await fillLearningText('');
    assert.equal(
      await execute(
        `return [...document.querySelectorAll('button')].find(e=>e.textContent.trim()==='保存理解回答并复盘').disabled;`
      ),
      true
    );
    await fillLearningText('编译退出码只能证明本次编译；设备运行需要独立观察。');
    await clickText('保存理解回答并复盘');
    await wait(
      async () =>
        (await ipc('learning_get_local_step', { sessionId })).session.state === 'reflecting',
      'reflection state'
    );
    await wait(
      () =>
        execute(
          `return [...document.querySelectorAll('.learn-action-area:not([hidden]) button')].some(e=>e.textContent.trim()==='完成本课并进入下一课');`
        ),
      'reflection form'
    );
    await fillLearningText('还需要区分真实观测与理解自述，不能据此授予设备通过。');
    if (stepId === 'source') {
      await wait(
        () =>
          execute(
            `return document.querySelector('[data-testid=learn-agent-plan] strong')?.textContent.includes('完成章节复盘');`
          ),
        'current reflection guidance'
      );
      await shot('theory-reflection');
    }
    await clickText('完成本课并进入下一课');
    await wait(async () => {
      const v = await ipc('learning_get_local_step', { sessionId });
      return v.session.currentStep !== stepId || v.session.state === 'completed';
    }, 'reflection completion');
    evidence.checks.push({
      name: `${stepId}: actual UI reading, answer and reflection; no execution`,
      status: 'PASS',
    });
  }
  evidence.report = await ipc('learning_generate_local_report', { sessionId });
  evidence.journey = await ipc('learning_get_journey', { sessionId });
  assert.equal(evidence.report.completedSteps, 3);
  assert.equal(evidence.report.softwareResult, 'NOT_REQUIRED');
  assert.equal(evidence.report.hilResult, 'NOT_REQUIRED');
  assert.equal(evidence.report.session.classification, 'REFLECTION_COMPLETED');
  assert.equal(evidence.journey.evidence.length, 3);
  for (const e of evidence.journey.evidence) {
    assert.equal(e.source, 'user-asserted');
    assert.equal(e.status, 'RECORDED');
  }
  assert.equal((await ipc('workbench_list_hosts')).length, 0);
  assert.equal((await ipc('workbench_list_projects')).length, 0);
  await wait(
    () =>
      execute(
        `return [...document.querySelectorAll('button')].some(e=>e.textContent.trim()==='生成最终学习报告');`
      ),
    'completed theory UI'
  );
  await clickText('生成最终学习报告');
  await wait(
    () => execute('return !!document.querySelector(".local-course-report");'),
    'theory report'
  );
  await shot('completed-report');
  evidence.checks.push({
    name: 'ungraded reflection report contains no software/HIL PASS and no host or project created',
    status: 'PASS',
  });
}

try {
  run('python3', ['scripts/generate-learning-smoke-course.py', packages, '--scenario', scenario]);
  evidence.fixture = JSON.parse(await readFile(path.join(packages, 'fixture.json'), 'utf8'));
  evidence.applicationSha256 = createHash('sha256')
    .update(await readFile(binary))
    .digest('hex');
  if (scenario === 'theory') {
    await startApp();
    await verifyTheory();
  } else {
    for (const key of ['host', 'client'])
      run('ssh-keygen', ['-q', '-t', 'ed25519', '-N', '', '-f', data + '/' + key]);
    await writeFile(data + '/authorized_keys', await readFile(data + '/client.pub'));
    const sshPort = await freePort();
    if (ideExtensions)
      ideFixture = await prepareIdeFixture({
        data,
        sdk,
        port: sshPort,
        env,
        extensions: ideExtensions,
      });
    await writeFile(
      data + '/sshd_config',
      [
        `Port ${sshPort}`,
        'ListenAddress 127.0.0.1',
        `HostKey ${data}/host`,
        `AuthorizedKeysFile ${data}/authorized_keys`,
        `PidFile ${data}/sshd.pid`,
        'PubkeyAuthentication yes',
        'PasswordAuthentication no',
        'KbdInteractiveAuthentication no',
        'UsePAM no',
        'Subsystem sftp internal-sftp',
        'PermitRootLogin no',
        'StrictModes no',
        'LogLevel ERROR',
        `AllowUsers ${os.userInfo().username}`,
      ].join('\n') + '\n'
    );
    sshd = spawn(run('which', ['sshd']).trim(), ['-D', '-e', '-f', data + '/sshd_config'], {
      stdio: ['ignore', 'ignore', 'pipe'],
    });
    sshd.stderr.on('data', (bytes) => {
      driverLog += bytes;
    });
    await startApp();
    if (realProvider) {
      assert.ok(
        process.env.LYNX_LEARN_RUNTIME_PROVIDER_DB,
        'Explicit existing provider database is required'
      );
      evidence.realProvider = JSON.parse(
        run('python3', [
          'scripts/prepare-learning-provider-test.py',
          process.env.LYNX_LEARN_RUNTIME_PROVIDER_DB,
          data,
        ])
      );
    }
    const boardRequest = {
      enrollmentId: 'fixture',
      releaseId: 'fixture',
      manifestSha256: 'a'.repeat(64),
      profileId: 'board',
      unitId: 'unit',
      activityId: 'activity',
      actionId: 'observe',
    };
    await reject('learn_collect_action_evidence', { runId: 'fixture_run' }, /LEARN_AUTH_REQUIRED/);
    const syncScope = {
      enrollmentId: 'fixture',
      releaseId: 'fixture',
      manifestSha256: 'a'.repeat(64),
      profileId: 'ubuntu',
      unitId: 'unit',
    };
    await reject(
      'learn_get_practice_review_gate',
      {
        request: {
          enrollmentId: syncScope.enrollmentId,
          releaseId: syncScope.releaseId,
          manifestSha256: syncScope.manifestSha256,
          unitId: syncScope.unitId,
        },
      },
      /LEARN_AUTH_REQUIRED/
    );
    await reject(
      'learn_inspect_practice_freshness',
      { batchId: 'fixture_batch' },
      /LEARN_AUTH_REQUIRED/
    );
    await reject('learn_list_practice_sync', { request: syncScope }, /LEARN_AUTH_REQUIRED/);
    await reject(
      'learn_advance_practice_sync',
      { batchId: 'fixture_batch' },
      /LEARN_AUTH_REQUIRED/
    );
    await reject(
      'learn_prepare_practice_sync',
      {
        request: {
          ...syncScope,
          batchId: 'fixture_batch',
          runs: [{ runId: 'fixture_run', snapshotSha256: 'b'.repeat(64) }],
        },
      },
      /LEARN_AUTH_REQUIRED/
    );
    await reject('learn_observe_board', { request: boardRequest }, /LEARN_AUTH_REQUIRED/);
    await reject('learn_list_board_observations', { request: boardRequest }, /LEARN_AUTH_REQUIRED/);
    await reject(
      'learn_observe_board',
      { request: { ...boardRequest, deviceId: 8 } },
      /unknown field.*deviceId/
    );
    for (const [name, error] of [
      ['bad-signature', /LOCAL_COURSE_SIGNATURE/],
      ['bad-path', /LOCAL_COURSE_(UNSAFE_PATH|PATH_INVALID)/],
      ['bad-provenance', /LOCAL_COURSE_.*(HASH|SOURCE|PROVENANCE)/],
      [
        'bad-command',
        scenario === 'mixed' ? /LOCAL_COURSE_REFLECTION_INVALID/ : /LOCAL_COURSE_COMMAND_FORBIDDEN/,
      ],
    ])
      await reject(
        'learning_import_local_course',
        { path: packages + '/' + name + '.lynx-course' },
        error
      );
    for (const name of ['bad-substitution', 'bad-quoted-traversal', 'bad-glob']) {
      await reject(
        'learning_import_local_course',
        { path: packages + '/' + name + '.lynx-course' },
        /LOCAL_COURSE_COMMAND_FORBIDDEN/
      );
    }
    assert.equal((await ipc('learning_list_local_courses')).length, 0);
    if (['bsp', 'multimedia'].includes(scenario)) {
      for (const name of [
        'bad-artifact-path',
        'bad-artifact-type',
        'bad-artifact-budget',
        'bad-artifact-old-schema',
      ]) {
        await reject(
          'learning_import_local_course',
          { path: packages + '/' + name + '.lynx-course' },
          /LOCAL_COURSE_ARTIFACT_INVALID/
        );
      }
    }
    evidence.installed = await ipc('learning_import_local_course', {
      path: packages + '/course.lynx-course',
    });
    assert.equal(evidence.installed.labCount, scenario === 'optional' ? 6 : 3);
    if (scenario === 'optional')
      await reject(
        'learning_import_local_course',
        { path: path.join(packages, 'bad-optional-old-schema.lynx-course') },
        /LOCAL_COURSE_OPTIONAL_SCHEMA_REQUIRED/
      );
    await reject(
      'learning_import_local_course',
      { path: packages + '/bad-signer.lynx-course' },
      /LOCAL_COURSE_SIGNER_CHANGED/
    );
    await reject(
      'learning_import_local_course',
      { path: packages + '/course.lynx-course' },
      /LOCAL_COURSE_VERSION_EXISTS/
    );
    const publicKey = (await readFile(data + '/host.pub', 'utf8')).split(/\s+/)[1];
    const hostFingerprint =
      'SHA256:' +
      createHash('sha256')
        .update(Buffer.from(publicKey, 'base64'))
        .digest('base64')
        .replace(/=+$/, '');
    const host = {
      id: null,
      name: 'Local learning acceptance SSH',
      address: '127.0.0.1',
      port: sshPort,
      username: os.userInfo().username,
      authType: 'private_key',
      privateKeyPath: data + '/client',
      hostFingerprint,
      defaultWorkDir: sdk,
      aiProvider: 'codex',
      aiExecutable: 'codex',
      aiArgsJson: '[]',
    };
    const trust = await ipc('workbench_test_ssh', { host, credentials: {}, allowUntrusted: false });
    assert.equal(trust.trusted, true);
    assert.equal(trust.fingerprint, hostFingerprint);
    const hostId = await ipc('workbench_save_host', {
      host,
      connectionTestSessionId: trust.auditSessionId,
    });
    const projectId = await ipc('workbench_save_project', {
      project: {
        id: null,
        name: '本地验收工程（无硬件）',
        hostId,
        deviceId: null,
        sdkDir: sdk,
        buildCommand: 'true',
        firmwarePath: '',
        localDownloadDir: data,
        verifySha256: true,
        localGatewayPort: await freePort(),
        remoteForwardPort: 18765,
        mcpServerName: 'learning-acceptance',
        aiProvider: 'codex',
      },
    });
    const created = await ipc('learning_start_session', {
      courseId: evidence.fixture.courseId,
      version: evidence.fixture.version,
      projectId: ['mixed', 'optional'].includes(scenario) ? 0 : projectId,
    });
    const sessionId = created.sessionId;
    const transition = (nextState, extra = {}) =>
      ipc('learning_transition_step', { sessionId, nextState, ...extra });
    await reject(
      'learning_transition_step',
      { sessionId, nextState: 'completed', reflection: 'skip' },
      /LEARNING_STATE_TRANSITION_INVALID/
    );
    if (scenario === 'optional') {
      assert.equal(created.projectId, 0);
      assert.equal(created.currentStep, 'optional-first');
      await ipc('learning_save_draft', {
        sessionId,
        stepId: 'optional-first',
        attempt: 1,
        answer: '选修阅读草稿',
        reflection: '先继续必修，再查选修',
      });
      await showSession();
      await clickText('跳过本选修活动');
      await wait(
        async () =>
          (await ipc('learning_get_local_step', { sessionId })).session.currentStep === 'source',
        'skip optional without project'
      );
      await reject(
        'learning_skip_optional_activity',
        {
          request: {
            sessionId,
            expectedStepId: 'source',
            expectedAttempt: 1,
            expectedState: 'reading',
          },
        },
        /LEARNING_REQUIRED_ACTIVITY_CANNOT_SKIP/
      );
      await ipc('learning_bind_local_project', { sessionId, projectId });
      await reloadAppPage();
      evidence.checks.push({
        name: 'optional entry skipped through UI without binding; required cannot skip; later practice binds',
        status: 'PASS',
      });
    }
    const document = await ipc('learning_read_document', { sessionId, stepId: 'source' });
    evidence.document = document;
    await ipc('learning_save_draft', {
      sessionId,
      stepId: 'source',
      attempt: 1,
      answer: '理解退出码与证据',
      reflection: '真实客户端重启后保留的草稿',
    });
    if (ideFixture)
      await verifyIdeFixture(ideFixture, {
        ipc,
        reject,
        wait,
        sessionId,
        evidence,
        storeDigest: learningStoreDigest,
        output,
      });
    await showSession();
    await shot('course-reading');
    await closeApp();
    await startApp();
    const recovered = await ipc('learning_get_journey', { sessionId });
    assert.equal(recovered.reflection, '真实客户端重启后保留的草稿');
    evidence.checks.push({ name: 'native restart restores session and draft', status: 'PASS' });
    await showSession();
    await shot('restored-draft');
    const sourceFile = path.join(
      sdk,
      scenario === 'multimedia' ? 'lesson/settings.json' : 'lesson/src/main.c'
    );
    const source = await readFile(sourceFile);
    for (const stepId of ['source', 'build', 'run']) {
      if (scenario === 'mixed' && stepId === 'source') {
        if ((await ipc('learning_get_local_step', { sessionId })).session.state === 'reading')
          await transition('checking_understanding');
        await transition('reflecting', { answer: '区分理解自述和实践证据' });
        await transition('completed', { reflection: '理论完成后仍需真实实践' });
        assert.equal(
          (await ipc('learning_get_local_step', { sessionId })).session.currentStep,
          'build'
        );
        if ((await ipc('learning_get_local_step', { sessionId })).session.state === 'reading')
          await transition('checking_understanding');
        await reject(
          'learning_transition_step',
          { sessionId, nextState: 'practicing', answer: 'ready' },
          /LEARNING_PROJECT_REQUIRED/
        );
        await reject(
          'learning_bind_local_project',
          { sessionId, projectId: 999999 },
          /LEARNING_PROJECT_NOT_FOUND/
        );
        await reloadAppPage();
        await showSession();
        await clickText('工程与版本');
        await wait(
          () => execute('return !!document.querySelector("[data-testid=local-deferred-project]");'),
          'deferred project UI'
        );
        await execute(
          `const select=document.querySelector('[data-testid=local-deferred-project] select');select.value=String(arguments[0]);select.dispatchEvent(new Event('change',{bubbles:true}));`,
          [projectId]
        );
        await clickText('绑定此工程');
        await wait(
          async () =>
            (await ipc('learning_get_local_step', { sessionId })).session.projectId === projectId,
          'bound project'
        );
        await reject(
          'learning_bind_local_project',
          { sessionId, projectId },
          /LEARNING_PROJECT_BINDING_NOT_ALLOWED/
        );
        evidence.checks.push({
          name: 'mixed course preserves reflection and binds saved project explicitly before practice',
          status: 'PASS',
        });
        continue;
      }
      assert.equal(
        (await ipc('learning_get_local_step', { sessionId })).session.currentStep,
        stepId
      );
      if ((await ipc('learning_get_local_step', { sessionId })).session.state === 'reading')
        await transition('checking_understanding');
      await reject(
        'learning_transition_step',
        { sessionId, nextState: 'practicing' },
        /LEARNING_ANSWER_REQUIRED/
      );
      await transition('practicing', { answer: '校验使用实际退出码，AI 建议不等于通过。' });
      await reject(
        'learning_transition_step',
        { sessionId, nextState: 'collecting_evidence' },
        /LEARNING_ACTION_REQUIRED/
      );
      if (scenario === 'bsp' && stepId === 'source') {
        const script = path.join(sdk, 'lesson/scripts/bsp_lesson.py');
        const original = await readFile(script, 'utf8');
        const changed = original.replace(
          "temporary.write_text(text + '\\n')",
          "temporary.write_text('invalid json')"
        );
        assert.notEqual(changed, original);
        try {
          await writeFile(script, changed);
          await reject(
            'learning_run_step_action',
            { sessionId, credentials: {} },
            /LEARNING_ARTIFACT_INVALID/
          );
          const failed = await ipc('learning_get_journey', { sessionId });
          const record = failed.evidence.find(
            (item) => item.kind === 'command' && item.status === 'ERROR'
          );
          assert.ok(record);
          const list = await ipc('learning_list_local_artifacts', {
            request: { sessionId, stepId, attempt: 1, digest: record.digest },
          });
          assert.deepEqual(list.items, []);
          assert.equal(
            (await ipc('learning_get_local_step', { sessionId })).session.state,
            'practicing'
          );
        } finally {
          await writeFile(script, original);
        }
        evidence.checks.push({
          name: 'zero-exit command with invalid declared artifact cannot create success or snapshot',
          status: 'PASS',
        });
      }
      if (stepId === 'build') {
        await writeFile(
          sourceFile,
          scenario === 'multimedia'
            ? JSON.stringify({ codec: 'lynx_missing_codec', sampleRate: 16000, channels: 1 })
            : 'deliberately invalid C source\n'
        );
        const failure = await ipc('learning_run_step_action', { sessionId, credentials: {} });
        assert.equal(failure.status, 'FAIL');
        assert.notEqual(failure.exitCode, 0);
        assert.equal(
          (await ipc('learning_generate_local_report', { sessionId })).softwareResult,
          'INCOMPLETE'
        );
        if (scenario === 'multimedia') evidence.encoderFailure = failure;
        else evidence.compilerFailure = failure;
        const diagnosticScope = { sessionId, stepId, attempt: 1, digest: failure.outputDigest };
        const diagnostic = await ipc('learning_get_local_command_diagnostic', {
          request: diagnosticScope,
        });
        assert.equal(diagnostic.status, 'FAIL');
        assert.equal(diagnostic.exitCode, failure.exitCode);
        assert.equal(diagnostic.availability, 'available');
        assert.match(diagnostic.text, scenario === 'multimedia' ? /Unknown encoder/ : /error:/);
        evidence.failureDiagnostic = diagnostic;
        await reject(
          'learning_get_local_command_diagnostic',
          {
            request: { ...diagnosticScope, digest: '0'.repeat(64) },
          },
          /LEARNING_DIAGNOSTIC_NOT_FOUND/
        );
        await closeApp();
        await startApp();
        await showSession();
        assert.deepEqual(
          await ipc('learning_get_local_command_diagnostic', { request: diagnosticScope }),
          diagnostic
        );
        await clickText('查看命令诊断');
        await wait(
          () => execute('return !!document.querySelector(".local-command-diagnostics pre");'),
          'stored failure diagnostics'
        );
        assert.match(
          await execute(
            'return document.querySelector(".local-command-diagnostics pre").textContent;'
          ),
          scenario === 'multimedia' ? /Unknown encoder/ : /error:/
        );
        await execute(
          'document.querySelector(".local-command-diagnostics").scrollIntoView({block:"center"});'
        );
        const diagnosticBox = await execute(
          'const r=document.querySelector(".local-command-diagnostics pre").getBoundingClientRect();return {left:r.left,right:r.right,width:innerWidth,top:r.top,bottom:r.bottom,height:innerHeight};'
        );
        assert.ok(
          diagnosticBox.left >= 0 && diagnosticBox.right <= diagnosticBox.width,
          'diagnostic output stays within viewport'
        );
        assert.ok(
          diagnosticBox.top >= 0 && diagnosticBox.bottom <= diagnosticBox.height,
          'diagnostic output is visible'
        );
        await shot('failure-diagnostic');
        await clickText('附到本次 AI 提问');
        await wait(
          () =>
            execute(
              'return !!document.querySelector("[data-testid=local-tutor-diagnostic-selection]");'
            ),
          'explicit tutor diagnostic selection'
        );
        await clickText('不附带诊断');
        assert.equal(
          await execute(
            'return !!document.querySelector("[data-testid=local-tutor-diagnostic-selection]");'
          ),
          false
        );
        evidence.checks.push({
          name: 'viewed diagnostic can be selected and removed without an AI request',
          status: 'PASS',
        });
        if (realProvider) {
          await clickText('附到本次 AI 提问');
          await clickText('向 AI 助教提问');
          // Matches the configured application's maximum 120-second HTTP budget.
          await wait(
            () => execute('return !!document.querySelector(".local-course-tutor-answer");'),
            'real provider tutor response',
            130000
          );
          const answer = await execute(
            'return document.querySelector(".local-course-tutor-answer .learn-document-markdown").innerText;'
          );
          const context = await execute(
            'return document.querySelector(".local-course-tutor-answer small").textContent;'
          );
          assert.match(answer, /编码|encoder|codec/i);
          assert.ok(
            answer.length <= 600,
            'L1 guidance stays concise instead of dumping internal metadata'
          );
          assert.match(context, /selected_local_command_diagnostic/);
          assert.equal(
            await execute(
              'return !!document.querySelector("[data-testid=local-tutor-diagnostic-selection]");'
            ),
            false
          );
          const afterTutor = await ipc('learning_get_journey', { sessionId });
          const note = afterTutor.notes.find(
            (item) => item.stepId === stepId && item.attempt === 1 && item.source === 'ai-provider'
          );
          assert.ok(note && note.answer.length > 20);
          assert.equal(
            (await ipc('learning_get_local_step', { sessionId })).session.state,
            'practicing'
          );
          assert.equal(
            (await ipc('learning_generate_local_report', { sessionId })).softwareResult,
            'INCOMPLETE'
          );
          evidence.realTutor = {
            answer,
            context,
            note,
            executedAction: false,
            source: 'ai-provider',
          };
          await execute(
            'document.querySelector(".local-course-tutor-answer").scrollIntoView({block:"center"});'
          );
          await shot('real-provider-tutor');
          evidence.checks.push({
            name: 'real provider explains selected synthetic diagnostic without executing or passing the step',
            status: 'PASS',
          });
        }
        evidence.checks.push({
          name: 'actual failed command diagnostics survive native restart and render',
          status: 'PASS',
        });
        await writeFile(sourceFile, source);
      }
      const result = await ipc('learning_run_step_action', { sessionId, credentials: {} });
      assert.equal(result.status, 'PASS');
      assert.equal(result.exitCode, 0);
      const diagnostic = await ipc('learning_get_local_command_diagnostic', {
        request: { sessionId, stepId, attempt: 1, digest: result.outputDigest },
      });
      assert.equal(diagnostic.status, 'PASS');
      assert.equal(diagnostic.exitCode, 0);
      assert.equal(diagnostic.availability, 'available');
      if (['bsp', 'multimedia'].includes(scenario)) {
        const scope = { sessionId, stepId, attempt: 1, digest: result.outputDigest };
        const list = await ipc('learning_list_local_artifacts', { request: scope });
        assert.deepEqual(list.scope, scope);
        const declared = (await ipc('learning_get_local_step', { sessionId })).lab.artifactOutputs;
        assert.equal(list.items.length, declared.length);
        for (const meta of list.items) {
          assert.deepEqual(
            meta.spec,
            declared.find((item) => item.artifactId === meta.spec.artifactId)
          );
          assert.deepEqual(
            await readArtifact(scope, meta),
            await readFile(path.join(sdk, 'lesson', meta.spec.path))
          );
        }
        evidence.artifactSnapshots ||= [];
        evidence.artifactSnapshots.push(list);
        if (stepId === 'source') {
          const meta = list.items[0],
            file = path.join(sdk, 'lesson', meta.spec.path);
          const original = await readFile(file);
          const originalMode = (await stat(file)).mode & 0o777;
          try {
            await unlink(file);
            await closeApp();
            await startApp();
            await showSession();
            assert.deepEqual(await ipc('learning_list_local_artifacts', { request: scope }), list);
            assert.deepEqual(await readArtifact(scope, meta), original);
            await reject(
              'learning_transition_step',
              { sessionId, nextState: 'verifying' },
              /LEARNING_INPUTS_CHANGED/
            );
            await showArtifact(scope, meta.spec.artifactId);
            assert.match(
              await execute(
                'return document.querySelector(".local-artifact-preview pre").textContent;'
              ),
              /synthetic/
            );
            await shot('artifact-text-after-restart');
            await execute('document.querySelector(".local-artifact-preview a[download]").click();');
            const download = path.join(data, 'downloads', path.basename(meta.spec.path));
            await wait(async () => {
              try {
                return (await readFile(download)).equals(original);
              } catch {
                return false;
              }
            }, 'native verified artifact download');
            evidence.checks.push({
              name: 'native download writes exact verified snapshot to isolated downloads directory',
              status: 'PASS',
            });
          } finally {
            await writeFile(file, original);
            await chmod(file, originalMode);
          }
          await reject(
            'learning_list_local_artifacts',
            { request: { ...scope, attempt: 99 } },
            /LEARNING_ARTIFACT_NOT_FOUND/
          );
          await reject(
            'learning_read_local_artifact_chunk',
            {
              request: { scope, artifactId: meta.spec.artifactId, sha256: meta.sha256, offset: 1 },
            },
            /LEARNING_ARTIFACT_INVALID/
          );
          evidence.checks.push({
            name: 'persisted artifact survives restart and remote deletion while stale workspace still blocks advancement',
            status: 'PASS',
          });
        }
        if (scenario === 'multimedia' && stepId === 'run') {
          await reloadAppPage();
          await showSession();
          await showArtifact(scope, 'audio');
          await wait(
            () =>
              execute(
                'const audio=document.querySelector(".local-artifact-preview audio");return audio?.duration===1;'
              ),
            'native WAV metadata'
          );
          assert.equal(
            await execute(
              'const audio=document.querySelector(".local-artifact-preview audio");return audio.paused&&!audio.autoplay;'
            ),
            true
          );
          await shot('artifact-native-audio');
          evidence.checks.push({
            name: 'native WebKit previews saved one-second WAV without autoplay',
            status: 'PASS',
          });
        }
      }
      if (scenario === 'multimedia') {
        const facts = JSON.parse(diagnostic.text);
        assert.equal(facts.sampleRate, 16000);
        if (stepId === 'run') assert.equal(facts.lossless, true);
      }
      if (scenario === 'bsp') {
        // The visible diagnostic is bounded; verify the complete metrics against
        // the recorded stdout digest instead of parsing a truncated tail.
        const metrics = await readFile(path.join(sdk, `lesson/build/${stepId}-metrics.json`));
        assert.equal(createHash('sha256').update(metrics).digest('hex'), diagnostic.outputSha256);
        assert.equal(diagnostic.truncated, metrics.toString('utf8').length > 2048);
        const facts = JSON.parse(metrics.toString('utf8'));
        assert.equal(facts.synthetic, true);
        assert.equal(facts.hardwareActions, false);
        assert.equal(facts.toolchain.target, 'arm-linux-gnueabihf');
        if (stepId !== 'source') {
          assert.equal(facts.architecture, 'ARM32');
          assert.equal(facts.memoryBytes, 64 * 1024 * 1024);
          assert.equal(facts.configMemoryBytes, facts.memoryBytes);
        }
        if (stepId === 'run') assert.equal(facts.deviceTreeRoundtrip, true);
      }
      if (stepId === (scenario === 'mixed' ? 'build' : 'source')) {
        const changedPath = path.join(
          sdk,
          'lesson',
          scenario === 'multimedia' ? 'settings.json' : 'src/main.c'
        );
        const original = await readFile(changedPath);
        const historyBefore = (await ipc('learning_get_journey', { sessionId })).evidence;
        try {
          await writeFile(changedPath, Buffer.concat([original, Buffer.from('\n')]));
          await reject(
            'learning_transition_step',
            { sessionId, nextState: 'verifying' },
            /LEARNING_INPUTS_CHANGED/
          );
          assert.equal(
            (await ipc('learning_get_local_step', { sessionId })).session.state,
            'collecting_evidence'
          );
          if (scenario === 'c') {
            await closeApp();
            await startApp();
            await showSession();
            await clickText('证据已记录，核对规则');
            await wait(
              () => execute("return document.body.textContent.includes('课程工程已变化');"),
              'stale workspace explanation'
            );
            assert.equal(
              await execute(
                "return [...document.querySelectorAll('button')].some(b=>b.textContent.trim()==='重新实践'&&!b.disabled);"
              ),
              true
            );
            assert.equal(
              await execute(
                "return document.body.textContent.includes('UnknownError:') || document.body.textContent.includes('LEARNING_INPUTS_CHANGED:');"
              ),
              false
            );
            await shot('stale-workspace-after-restart');
            evidence.checks.push({
              name: 'persisted snapshot rejects changed source after native restart and UI offers explicit retry',
              status: 'PASS',
            });
          }
        } finally {
          await writeFile(changedPath, original);
        }
        const marker = path.join(sdk, 'lesson', 'freshness-probe.txt');
        try {
          await writeFile(marker, 'new unverified input');
          await reject(
            'learning_transition_step',
            { sessionId, nextState: 'verifying' },
            /LEARNING_INPUTS_CHANGED/
          );
        } finally {
          await unlink(marker);
        }
        const script = path.join(sdk, 'lesson/scripts/verify.sh');
        const scriptBefore = await readFile(script);
        try {
          await writeFile(
            script,
            Buffer.concat([scriptBefore, Buffer.from('\n# changed script\n')])
          );
          await reject(
            'learning_transition_step',
            { sessionId, nextState: 'verifying' },
            /LEARNING_INPUTS_CHANGED/
          );
        } finally {
          await writeFile(script, scriptBefore);
        }
        try {
          await unlink(changedPath);
          await reject(
            'learning_transition_step',
            { sessionId, nextState: 'verifying' },
            /LEARNING_INPUTS_CHANGED/
          );
        } finally {
          await writeFile(changedPath, original);
        }
        try {
          await symlink(changedPath, marker);
          await reject(
            'learning_transition_step',
            { sessionId, nextState: 'verifying' },
            /LEARNING_INPUTS_UNAVAILABLE/
          );
        } finally {
          await unlink(marker);
        }
        const originalProject = (await ipc('workbench_list_projects')).find(
          (p) => p.id === projectId
        );
        assert.ok(originalProject);
        try {
          await ipc('workbench_save_project', {
            project: { ...originalProject, sdkDir: sdk + '-changed' },
          });
          await reject(
            'learning_transition_step',
            { sessionId, nextState: 'verifying' },
            /LEARNING_INPUTS_CHANGED/
          );
        } finally {
          await ipc('workbench_save_project', { project: originalProject });
        }
        assert.deepEqual(
          (await ipc('learning_get_journey', { sessionId })).evidence,
          historyBefore
        );
      }
      await transition('verifying');
      if (stepId === 'source') {
        await transition('practicing');
        await reject(
          'learning_transition_step',
          { sessionId, nextState: 'collecting_evidence' },
          /LEARNING_ACTION_REQUIRED/
        );
        assert.equal((await ipc('learning_get_journey', { sessionId })).attempt, 2);
        assert.equal(
          (await ipc('learning_run_step_action', { sessionId, credentials: {} })).status,
          'PASS'
        );
        await transition('verifying');
      }
      await transition('reflecting');
      if (stepId === (scenario === 'mixed' ? 'build' : 'source')) {
        const changedPath = path.join(
          sdk,
          'lesson',
          scenario === 'multimedia' ? 'settings.json' : 'src/main.c'
        );
        const original = await readFile(changedPath);
        try {
          await writeFile(changedPath, Buffer.concat([original, Buffer.from('\n')]));
          await reject(
            'learning_transition_step',
            { sessionId, nextState: 'completed', reflection: '不能把旧结果当成当前有效' },
            /LEARNING_INPUTS_CHANGED/
          );
          assert.equal(
            (await ipc('learning_get_local_step', { sessionId })).session.state,
            'reflecting'
          );
        } finally {
          await writeFile(changedPath, original);
        }
      }
      await reject(
        'learning_transition_step',
        { sessionId, nextState: 'completed' },
        /LEARNING_REFLECTION_REQUIRED/
      );
      await transition('completed', {
        reflection: `${stepId}：已核对命令结果与实际证据，未进行实机验证。`,
      });
      evidence.checks.push({
        name: `${stepId} real SSH execution and completion`,
        status: 'PASS',
        result,
      });
    }
    if (scenario === 'multimedia') {
      evidence.mediaMetrics = {};
      for (const step of ['source', 'build', 'run']) {
        evidence.mediaMetrics[step] = JSON.parse(
          await readFile(path.join(sdk, `lesson/build/${step}-metrics.json`), 'utf8')
        );
      }
      const { source, build, run: decoded } = evidence.mediaMetrics;
      assert.equal(source.frames, 16000);
      assert.equal(build.codec, 'flac');
      assert.equal(build.sampleRate, 16000);
      assert.equal(build.channels, 1);
      assert.equal(build.durationSeconds, 1);
      assert.equal(decoded.sourcePcmSha256, source.pcmSha256);
      assert.equal(decoded.decodedPcmSha256, source.pcmSha256);
      assert.equal(decoded.lossless, true);
      evidence.checks.push({
        name: 'actual FFprobe metrics and lossless PCM roundtrip',
        status: 'PASS',
      });
    }
    if (scenario === 'bsp') {
      evidence.bspMetrics = {};
      for (const step of ['source', 'build', 'run']) {
        evidence.bspMetrics[step] = JSON.parse(
          await readFile(path.join(sdk, `lesson/build/${step}-metrics.json`), 'utf8')
        );
      }
      const { build, run: checked } = evidence.bspMetrics;
      assert.equal(build.memoryBytes, 64 * 1024 * 1024);
      assert.equal(build.elfMachine, 40);
      assert.equal(checked.deviceTreeRoundtrip, true);
      assert.deepEqual(checked.artifacts, build.artifacts);
      for (const [name, digest] of Object.entries(build.artifacts)) {
        const bytes = await readFile(path.join(sdk, 'lesson/build', name));
        assert.equal(createHash('sha256').update(bytes).digest('hex'), digest);
      }
      const elf = await readFile(path.join(sdk, 'lesson/build/config.o'));
      assert.equal(elf.subarray(0, 6).toString('hex'), '7f454c460101');
      assert.equal(elf.readUInt16LE(18), 40);
      const dtb = await readFile(path.join(sdk, 'lesson/build/board.dtb'));
      assert.equal(dtb.readUInt32BE(0), 0xd00dfeed);
      evidence.checks.push({
        name: 'actual ARM32 object, device tree properties and semantic roundtrip',
        status: 'PASS',
      });
    }
    if (scenario === 'optional') {
      const before = await ipc('learning_generate_local_report', { sessionId });
      assert.equal(before.softwareResult, 'PASS');
      assert.equal(before.requiredCompletedSteps, 3);
      assert.equal(before.requiredSteps, 3);
      assert.equal(before.optionalSteps, 3);
      assert.equal(before.optionalSkippedSteps, 1);
      await transition('checking_understanding');
      await transition('practicing', { answer: '观察选修失败，不伪造通过' });
      const failed = await ipc('learning_run_step_action', { sessionId, credentials: {} });
      assert.equal(failed.status, 'FAIL');
      assert.equal(failed.exitCode, 64);
      const afterFailure = await ipc('learning_generate_local_report', { sessionId });
      assert.equal(afterFailure.softwareResult, 'PASS');
      assert.equal(afterFailure.currentWorkspace.reference.stepId, 'run');
      for (const [from, to] of [
        ['optional-failure', 'optional-reflection'],
        ['optional-reflection', null],
      ]) {
        await reloadAppPage();
        await showSession();
        await clickText('跳过本选修活动');
        await wait(async () => {
          const s = (await ipc('learning_get_local_step', { sessionId })).session;
          return to ? s.currentStep === to : s.state === 'completed';
        }, 'explicit optional skip ' + from);
        await reject(
          'learning_skip_optional_activity',
          {
            request: {
              sessionId,
              expectedStepId: from,
              expectedAttempt: 1,
              expectedState: from === 'optional-failure' ? 'practicing' : 'reading',
            },
          },
          /LEARNING_ACTION_CONTEXT_CHANGED/
        );
      }
      const saved = await ipc('learning_get_journey', { sessionId });
      assert.equal(saved.attempts.filter((a) => a.state === 'skipped').length, 3);
      const skippedDraft = saved.attempts.find((a) => a.stepId === 'optional-first');
      assert.equal(skippedDraft.answer, '选修阅读草稿');
      assert.equal(skippedDraft.reflection, '先继续必修，再查选修');
      assert.equal(
        saved.evidence.filter((e) => e.kind === 'optional-skip' && e.status === 'RECORDED').length,
        3
      );
      assert.ok(saved.evidence.some((e) => e.stepId === 'optional-failure' && e.status === 'FAIL'));
      assert.equal(
        saved.evidence.filter((e) => e.stepId.startsWith('optional-') && e.status === 'PASS')
          .length,
        0
      );
      await closeApp();
      await startApp();
      await showSession();
      const recovered = await ipc('learning_get_journey', { sessionId });
      assert.deepEqual(recovered.attempts, saved.attempts);
      assert.deepEqual(recovered.evidence, saved.evidence);
      await wait(
        () =>
          execute(
            'return document.querySelector(".learn-records")?.textContent.includes("选修跳过 3");'
          ),
        'required and optional report after restart'
      );
      evidence.optionalFailure = failed;
      evidence.checks.push({
        name: 'actual optional exit 64 preserves required current PASS; UI skips practice/reflection without PASS; restart preserves failures and three skip events',
        status: 'PASS',
      });
    }
    evidence.report = await ipc('learning_generate_local_report', { sessionId });
    assert.equal(evidence.report.completedSteps, 3);
    assert.equal(evidence.report.softwareResult, 'PASS');
    assert.equal(evidence.report.hilResult, 'NOT_REQUIRED');
    assert.equal(evidence.report.session.classification, 'SOFTWARE_PASS');
    assert.equal(evidence.report.recordedSoftwareResult, 'PASS');
    assert.equal(evidence.report.currentWorkspace.status, 'MATCH');
    assert.equal(evidence.report.currentWorkspace.reference.stepId, 'run');
    if (['c', 'optional'].includes(scenario)) {
      await reloadAppPage();
      const history = await ipc('learning_get_journey', { sessionId });
      const beforeChangedReport = learningStoreDigest();
      const sourcePath = path.join(sdk, 'lesson/src/main.c');
      const original = await readFile(sourcePath);
      try {
        await writeFile(
          sourcePath,
          Buffer.concat([original, Buffer.from('\n/* changed after completion */\n')])
        );
        const changed = await ipc('learning_generate_local_report', { sessionId });
        assert.equal(changed.softwareResult, 'REVALIDATION_REQUIRED');
        assert.equal(changed.recordedSoftwareResult, 'PASS');
        assert.equal(changed.currentWorkspace.status, 'CHANGED');
        assert.equal(changed.session.state, 'completed');
        assert.equal(learningStoreDigest(), beforeChangedReport);
        evidence.changedReport = changed;
        await closeApp();
        await startApp();
        await showSession();
        await wait(
          () =>
            execute(
              'return document.querySelector(".learn-records [data-testid=local-report-currency]")?.textContent.includes("需要重新验证");'
            ),
          'restarted record shows changed workspace'
        );
        await clickText('生成最终学习报告');
        await wait(
          () =>
            execute(
              'return document.querySelector(".local-course-report")?.textContent.includes("REVALIDATION_REQUIRED");'
            ),
          'workspace report shows changed current result'
        );
        await shot('changed-report-after-restart');
      } finally {
        await writeFile(sourcePath, original);
      }
      await wait(
        () => execute('return document.body.textContent.includes("草稿已保存到本地");'),
        'normal restart draft save finishes before readonly report sequence'
      );
      // Finish the UI's explicit save and unmount its draft timer before testing
      // the report API's read-only contract. Compare the entire stored database.
      await execute(
        'document.querySelector(".local-course-session-shell .learn-session-meta").open=true;'
      );
      await clickText('退出到会话列表');
      await wait(
        () => execute('return !!document.querySelector("[data-testid=learn-session-setup]");'),
        'workspace exited after explicit draft save'
      );
      const historyBeforeReadonly = await ipc('learning_get_journey', { sessionId });
      assert.deepEqual(historyBeforeReadonly.attempts, history.attempts);
      assert.deepEqual(historyBeforeReadonly.evidence, history.evidence);
      assert.deepEqual(historyBeforeReadonly.notes, history.notes);
      assert.equal(historyBeforeReadonly.answer, history.answer);
      assert.equal(historyBeforeReadonly.reflection, history.reflection);
      assert.ok(historyBeforeReadonly.savedAt >= history.savedAt);
      const beforeReadonly = learningStoreDigest();
      const project = (await ipc('workbench_list_projects')).find((p) => p.id === projectId);
      try {
        await ipc('workbench_save_project', { project: { ...project, sdkDir: sdk + '-changed' } });
        assert.equal(
          (await ipc('learning_generate_local_report', { sessionId })).softwareResult,
          'REVALIDATION_REQUIRED'
        );
      } finally {
        await ipc('workbench_save_project', { project });
      }
      await rename(data + '/client', data + '/client-unavailable');
      try {
        const unavailable = await ipc('learning_generate_local_report', { sessionId });
        assert.equal(unavailable.softwareResult, 'CURRENT_UNVERIFIED');
        assert.equal(unavailable.recordedSoftwareResult, 'PASS');
        assert.equal(unavailable.currentWorkspace.status, 'UNAVAILABLE');
        evidence.unavailableReport = unavailable;
      } finally {
        await rename(data + '/client-unavailable', data + '/client');
      }
      evidence.report = await ipc('learning_generate_local_report', { sessionId });
      assert.equal(evidence.report.softwareResult, 'PASS');
      assert.equal(evidence.report.currentWorkspace.status, 'MATCH');
      const afterReadonly = learningStoreDigest();
      assert.equal(afterReadonly, beforeReadonly);
      evidence.readonlyStore = {
        beforeChangedReport,
        beforeReadonly,
        afterReadonly,
        scope: 'entire isolated learning.sqlite logical snapshot',
        includesArchives: true,
      };
      evidence.checks.push({
        name: 'completed report rechecks changed/restored source and binding, survives restart, preserves history and refuses current PASS without SSH authentication',
        status: 'PASS',
      });
    }
    evidence.journey = await ipc('learning_get_journey', { sessionId });
    assert.ok(evidence.journey.evidence.some((e) => e.status === 'FAIL' && e.stepId === 'build'));
    await reloadAppPage();
    await wait(
      () => execute('return !!document.querySelector("[data-testid=app-sidebar]");'),
      'reload'
    );
    await showSession();
    // Completed state exposes its report action in the learning workspace.
    await wait(
      () =>
        execute(
          'return !!document.querySelector(".learn-records [data-testid=local-report-currency]");'
        ),
      'record report finished before independent workspace report'
    );
    await clickText('生成最终学习报告');
    await wait(() => execute('return !!document.querySelector(".local-course-report");'), 'report');
    await shot('completed-report');
    if (environmentFixture)
      await verifyEnvironmentFixture({
        data,
        hostId,
        projectId,
        ipc,
        run,
        wait,
        execute,
        clickText,
        shot,
        showSession,
        closeApp,
        startApp,
        learningStoreDigest,
        evidence,
      });
    if (environmentExecution)
      await verifyEnvironmentExecution({
        data,
        hostId,
        projectId,
        ipc,
        run,
        wait,
        execute,
        clickText,
        shot,
        showSession,
        closeApp,
        startApp,
        learningStoreDigest,
        evidence,
      });
    if (prerequisitesFixture)
      await verifyPrerequisites({
        data,
        hostId,
        projectId,
        ipc,
        run,
        wait,
        execute,
        clickText,
        shot,
        showSession,
        closeApp,
        startApp,
        evidence,
      });
    if (hilTrust)
      await verifyHilTrustFixture({
        data,
        hostId,
        ipc,
        reject,
        run,
        wait,
        execute,
        clickText,
        shot,
        reloadAppPage,
        showSession,
        startApp,
        closeApp,
        evidence,
      });
  }
  if (process.env.LYNX_LEARN_RUNTIME_REVISIT === '1')
    await verifyRevisit({
      data,
      ipc,
      run,
      wait,
      execute,
      clickText,
      shot,
      showSession,
      closeApp,
      startApp,
      evidence,
      scenario,
    });
  evidence.status = 'PASS';
} catch (error) {
  evidence.status = 'FAIL';
  evidence.error = String(error);
  evidence.errorStack = error.stack;
  process.exitCode = 1;
  if (session) await shot('failure').catch(() => {});
} finally {
  try {
    await closeApp();
  } catch (error) {
    evidence.cleanupError = String(error);
    evidence.status = 'FAIL';
    process.exitCode = 1;
  }
  if (!session && sshd) sshd.kill('SIGTERM');
  if (!session && driver) driver.kill('SIGTERM');
  if (session && driver) {
    driver.stdout.unref();
    driver.stderr.unref();
    driver.unref();
  }
  if (sshd) {
    sshd.stderr.unref();
    sshd.unref();
  }
  if (!session) await unlink(binary);
  evidence.finishedAt = new Date().toISOString();
  await writeFile(path.join(output, 'result.json'), JSON.stringify(evidence, null, 2));
  await writeFile(path.join(output, 'driver.log'), driverLog);
  console.log(
    JSON.stringify({
      status: evidence.status,
      checks: evidence.checks.length,
      output,
      error: evidence.error,
      cleanupError: evidence.cleanupError,
    })
  );
}
