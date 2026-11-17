import { nextFixturePort } from './lib/fixture-ports.mjs';
// Signed synthetic course, real desktop/SSH, external SDK file. No hardware or cloud writes.
import assert from 'node:assert/strict';
import { readFile, writeFile, rename, unlink, stat } from 'node:fs/promises';
import path from 'node:path';

export async function verifyEnvironmentExecution(ctx) {
  const {
    data,
    hostId,
    projectId: originalProjectId,
    ipc,
    run,
    wait,
    execute,
    shot,
    showSession,
    closeApp,
    startApp,
    learningStoreDigest,
    evidence,
  } = ctx;
  await closeApp();
  await startApp();
  const packages = path.join(data, 'packages-environment-execution');
  run('python3', [
    'scripts/generate-learning-smoke-course.py',
    packages,
    '--scenario',
    'environment-execution',
  ]);
  const fixture = JSON.parse(await readFile(path.join(packages, 'fixture.json'), 'utf8'));
  const original = (await ipc('workbench_list_projects')).find((p) => p.id === originalProjectId);
  const sdk = path.join(packages, 'prepared-sdk');
  const projectId = await ipc('workbench_save_project', {
    project: {
      ...original,
      id: null,
      name: '执行环境与当前性测试工程（无硬件）',
      mcpServerName: 'learning-environment-execution',
      hostId,
      deviceId: null,
      sdkDir: sdk,
      localGatewayPort: await nextFixturePort(),
      remoteForwardPort: await nextFixturePort(),
    },
  });
  await ipc('learning_import_local_course', { path: path.join(packages, 'course.lynx-course') });
  const created = await ipc('learning_start_session', {
    courseId: fixture.courseId,
    version: fixture.version,
    projectId,
  });
  const sessionId = created.sessionId;
  const file = path.join(sdk, 'shared-sdk/VERSION'),
    originalVersion = await readFile(file);
  const transition = (nextState, extra = {}) =>
    ipc('learning_transition_step', { sessionId, nextState, ...extra });
  const action = () => ipc('learning_run_step_action', { sessionId, credentials: {} });
  const report = () => ipc('learning_generate_local_report', { sessionId });
  const db = path.join(data, 'data/com.dshanpi.lynx/learning.sqlite');
  function rows() {
    return JSON.parse(
      run('python3', [
        '-c',
        `import sqlite3,json,sys,hashlib
from pathlib import Path
c=sqlite3.connect(Path(sys.argv[1]).as_uri()+'?mode=ro',uri=True)
result=[]
for row in c.execute("SELECT id,step_id,attempt,status,digest,detail_json FROM learning_evidence WHERE session_id=? AND kind='command' AND source='software' ORDER BY id",(sys.argv[2],)):
 assert hashlib.sha256(row[5].encode()).hexdigest()==row[4]
 result.append(dict(id=row[0],stepId=row[1],attempt=row[2],status=row[3],digest=row[4],detail=json.loads(row[5])))
print(json.dumps(result));c.close()`,
        db,
        sessionId,
      ])
    );
  }
  async function rejected(fn) {
    let error = '';
    try {
      await fn();
    } catch (e) {
      error = String(e);
    }
    assert.match(error, /LEARNING_INPUTS_CHANGED/);
    return error;
  }
  const initial = await ipc('learning_inspect_project', { sessionId });
  assert.ok(initial.checks.every((c) => c.status === 'MATCH'));
  await transition('checking_understanding');
  await transition('practicing', { answer: '执行时重新核验环境；先前检查不能替代当前输入。' });
  const rejectedBefore = [];
  await rename(file, file + '.absent');
  try {
    rejectedBefore.push(await rejected(action));
  } finally {
    await rename(file + '.absent', file);
  }
  await writeFile(file, 'different SDK before execution\n');
  rejectedBefore.push(await rejected(action));
  assert.equal(
    await stat(path.join(sdk, 'lesson/build/source-metrics.json')).then(
      () => true,
      () => false
    ),
    false
  );
  assert.equal(rows().filter((r) => r.status === 'PASS').length, 0);
  await writeFile(file, originalVersion);
  evidence.checks.push({
    name: 'environment requirements are rechecked before execution: missing/mismatched external SDK blocks commands, preserves errors and creates no PASS',
    status: 'PASS',
  });

  await writeFile(
    path.join(sdk, 'shared-sdk/change-during-run'),
    'controlled synthetic mutation\n'
  );
  const changedDuring = await rejected(action);
  assert.equal(
    await stat(path.join(sdk, 'lesson/build/source-metrics.json')).then(
      () => true,
      () => false
    ),
    true
  );
  assert.equal(rows().filter((r) => r.status === 'PASS').length, 0);
  assert.equal((await ipc('learning_get_local_step', { sessionId })).session.state, 'practicing');
  await unlink(path.join(sdk, 'shared-sdk/change-during-run'));
  await writeFile(file, originalVersion);
  const source = await action();
  assert.equal(source.status, 'PASS');
  const sourceRow = rows().find((r) => r.status === 'PASS');
  assert.equal(sourceRow.detail.workspaceSnapshot.version, 3);
  assert.equal(sourceRow.detail.workspaceSnapshot.environment.observations.length, 5);
  await writeFile(file, 'changed after source PASS\n');
  const transitionRejected = await rejected(() => transition('verifying'));
  await writeFile(file, originalVersion);
  await transition('verifying');
  await transition('reflecting');
  await transition('completed', {
    reflection: '环境变化会使当前推进失效；恢复后重新核验，保留失败记录。',
  });
  evidence.checks.push({
    name: 'external SDK mutation during real command never creates success; repaired execution saves environment identity and subsequent change blocks advancement',
    status: 'PASS',
  });

  const actions = [source];
  for (const stepId of ['build', 'run']) {
    assert.equal((await ipc('learning_get_local_step', { sessionId })).session.currentStep, stepId);
    await transition('checking_understanding');
    await transition('practicing', { answer: '真实 ARM 配置与设备树验证，不部署到开发板。' });
    const result = await action();
    assert.equal(result.status, 'PASS');
    actions.push(result);
    await transition('verifying');
    await transition('reflecting');
    await transition('completed', {
      reflection: '软件产物与环境各有证据，不能替代 T113-Pro 验证。',
    });
  }
  const passed = await report();
  assert.equal(passed.softwareResult, 'PASS');
  assert.equal(passed.hilResult, 'NOT_REQUIRED');
  assert.deepEqual(
    passed.currentWorkspace.environmentChecks.map((c) => [c.stepId, c.status]),
    [
      ['source', 'MATCH'],
      ['build', 'MATCH'],
    ]
  );
  const last = rows()
    .filter((r) => r.status === 'PASS')
    .at(-1);
  assert.equal(last.stepId, 'run');
  assert.equal(last.detail.workspaceSnapshot.version, 3);
  await closeApp();
  await startApp();
  const reopened = await report();
  assert.equal(reopened.softwareResult, 'PASS');
  const before = learningStoreDigest();
  await writeFile(file, 'changed external SDK after final step without environment declarations\n');
  const changed = await report();
  assert.equal(changed.recordedSoftwareResult, 'PASS');
  assert.equal(changed.softwareResult, 'REVALIDATION_REQUIRED');
  assert.deepEqual(
    changed.currentWorkspace.environmentChecks.map((c) => [c.stepId, c.status]),
    [
      ['source', 'CHANGED'],
      ['build', 'MATCH'],
    ]
  );
  const after = learningStoreDigest();
  assert.equal(after, before);
  await showSession();
  await wait(
    () =>
      execute(
        'return document.querySelector("[data-testid=local-report-environments]")?.innerText.includes("已变化，需重新验证")'
      ),
    'earlier environment difference visible'
  );
  await execute(
    'document.querySelector("[data-testid=local-report-environments]").scrollIntoView({block:"center"}); return new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve(true))));'
  );
  await shot('environment-execution-changed');
  assert.equal(
    await execute(
      'return document.querySelector("[data-testid=local-report-environments]")?.innerText.includes("已变化，需重新验证")'
    ),
    true
  );
  await writeFile(file, originalVersion);
  const restored = await report();
  assert.equal(restored.softwareResult, 'PASS');
  evidence.checks.push({
    name: 'reopened report detects an earlier required activity dependency even though final step declares none; current result changes, historical PASS and entire learning database remain unchanged',
    status: 'PASS',
  });

  // Seed only this isolated course's old-format execution after a complete safety-gated close.
  // This compatibility fixture is explicitly synthetic; no real learner history is rewritten.
  await closeApp();
  const legacySeed = JSON.parse(
    run('python3', [
      '-c',
      `import sqlite3,json,sys,hashlib
c=sqlite3.connect(sys.argv[1]);row=c.execute("SELECT id,detail_json FROM learning_evidence WHERE session_id=? AND step_id='source' AND status='PASS' ORDER BY id DESC LIMIT 1",(sys.argv[2],)).fetchone()
value=json.loads(row[1]);value['workspaceSnapshot'].pop('environment');value['workspaceSnapshot'].pop('inputsSha256');value['workspaceSnapshot']['version']=1
raw=json.dumps(value,separators=(',',':'),ensure_ascii=False);digest=hashlib.sha256(raw.encode()).hexdigest()
c.execute('UPDATE learning_evidence SET detail_json=?,digest=? WHERE id=?',(raw,digest,row[0]));c.commit();c.close()
print(json.dumps(dict(id=row[0],digest=digest,detail=value,syntheticLegacySeed=True)))`,
      db,
      sessionId,
    ])
  );
  await startApp();
  const legacy = await report();
  assert.equal(legacy.recordedSoftwareResult, 'PASS');
  assert.equal(legacy.softwareResult, 'CURRENT_UNVERIFIED');
  assert.deepEqual(
    legacy.currentWorkspace.environmentChecks.map((c) => [c.stepId, c.status]),
    [
      ['source', 'UNRECORDED'],
      ['build', 'MATCH'],
    ]
  );
  const preserved = rows().find((r) => r.id === legacySeed.id);
  assert.equal(preserved.digest, legacySeed.digest);
  assert.deepEqual(preserved.detail, legacySeed.detail);
  await showSession();
  await wait(
    () =>
      execute(
        'return document.querySelector("[data-testid=local-report-environments]")?.innerText.includes("旧执行未记录环境")'
      ),
    'legacy execution environment gap visible'
  );
  await execute(
    'document.querySelector("[data-testid=local-report-environments]").scrollIntoView({block:"center"}); return new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve(true))));'
  );
  await shot('environment-execution-legacy');
  evidence.checks.push({
    name: 'explicitly seeded old schema-7 execution remains historically PASS but current environment is UNRECORDED after restart, without backfilling or rewriting the seeded record',
    status: 'PASS',
  });
  evidence.environmentExecution = {
    fixture,
    sessionId,
    initial,
    rejectedBefore,
    changedDuring,
    transitionRejected,
    sourceRow,
    actions,
    passed,
    reopened,
    changed,
    restored,
    readonlyStore: { before, after },
    legacySeed,
    legacy,
    legacyRawPreserved: true,
    hardwareActionsExecuted: false,
  };
}
