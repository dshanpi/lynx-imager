import { nextFixturePort } from './lib/fixture-ports.mjs';
// Real Tauri, signed synthetic graph, loopback SSH. No cloud grades or hardware.
import assert from 'node:assert/strict';
import { readFile, writeFile, appendFile, stat, rename } from 'node:fs/promises';
import path from 'node:path';

export async function verifyPrerequisites(ctx) {
  const {
    data,
    hostId,
    projectId: originalProjectId,
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
  } = ctx;
  await closeApp();
  await startApp();
  const packages = path.join(data, 'packages-prerequisites');
  run('python3', [
    'scripts/generate-learning-smoke-course.py',
    packages,
    '--scenario',
    'prerequisites',
  ]);
  const fixture = JSON.parse(await readFile(path.join(packages, 'fixture.json'), 'utf8'));
  const sdk = path.join(packages, 'prepared-sdk');
  const original = (await ipc('workbench_list_projects')).find((p) => p.id === originalProjectId);
  const projectId = await ipc('workbench_save_project', {
    project: {
      ...original,
      id: null,
      name: '前置关系合成验收工程',
      mcpServerName: 'learning-prerequisites',
      hostId,
      deviceId: null,
      sdkDir: sdk,
      localGatewayPort: await nextFixturePort(),
      remoteForwardPort: await nextFixturePort(),
    },
  });
  const check = (name) => evidence.checks.push({ name, status: 'PASS' });
  async function reject(fn, pattern) {
    let error = '';
    try {
      await fn();
    } catch (e) {
      error = String(e);
    }
    assert.match(error, pattern);
    return error;
  }
  const rejectedPackages = [];
  for (const invalid of ['cycle', 'missing', 'duplicate', 'self', 'legacy']) {
    rejectedPackages.push(
      await reject(
        () =>
          ipc('learning_import_local_course', {
            path: path.join(packages, `bad-prerequisites-${invalid}.lynx-course`),
          }),
        /LOCAL_COURSE_PREREQUISITES/
      )
    );
  }
  const installed = await ipc('learning_import_local_course', {
    path: path.join(packages, 'course.lynx-course'),
  });
  assert.equal(installed.entryRequiresProject, false);
  const created = await ipc('learning_start_session', {
    courseId: fixture.courseId,
    version: fixture.version,
    projectId: 0,
  });
  const sessionId = created.sessionId;
  assert.equal(created.currentStep, 'intro');
  assert.equal(created.projectId, 0);
  check(
    'signed graph rejects cycles, missing/duplicate/self references and legacy ignored gates; first available theory root starts without a project'
  );
  const step = () => ipc('learning_get_local_step', { sessionId });
  const journey = () => ipc('learning_get_journey', { sessionId });
  const report = () => ipc('learning_generate_local_report', { sessionId });
  const transition = (nextState, extra = {}) =>
    ipc('learning_transition_step', { sessionId, nextState, ...extra });
  const action = () => ipc('learning_run_step_action', { sessionId, credentials: {} });
  const latest = (j, id) =>
    j.attempts.filter((a) => a.stepId === id).sort((a, b) => b.attempt - a.attempt)[0];
  async function choose(targetStepId) {
    const s = (await step()).session,
      j = await journey();
    return ipc('learning_revisit_activity', {
      request: {
        sessionId,
        expectedStepId: s.currentStep,
        expectedState: s.state,
        expectedAttempt: j.attempt,
        targetStepId,
        expectedTargetAttempt: latest(j, targetStepId)?.attempt ?? 0,
      },
    });
  }
  async function skip() {
    const s = (await step()).session,
      j = await journey();
    return ipc('learning_skip_optional_activity', {
      request: {
        sessionId,
        expectedStepId: s.currentStep,
        expectedState: s.state,
        expectedAttempt: j.attempt,
      },
    });
  }
  async function practice() {
    const view = await step();
    if (view.session.state === 'reading') await transition('checking_understanding');
    if ((await step()).session.state === 'checking_understanding')
      await transition('practicing', {
        answer: '按声明依赖核验真实输入；课程顺序不代表前置关系。',
      });
  }
  async function finish() {
    const view = await step();
    if (view.lab.activityKind === 'reflection') {
      if (view.session.state === 'reading') await transition('checking_understanding');
      await transition('reflecting', {
        answer: '理解前置依赖与独立学习；自述不代表软件或实机通过。',
      });
    } else {
      await practice();
      assert.equal((await action()).status, 'PASS');
      await transition('verifying');
      await transition('reflecting');
    }
    await transition('completed', {
      reflection: '保留原始尝试，按当前证据复核依赖，未操作开发板。',
    });
  }
  async function frame() {
    await execute(
      'return new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve(true))));'
    );
  }
  async function preview(title) {
    await execute(
      'const d=document.querySelector(".learn-directory").parentElement;d.open=true;return true;'
    );
    await execute(
      'const b=[...document.querySelectorAll(".learn-directory button")].find(b=>b.innerText.includes(arguments[0]));if(!b)throw new Error("chapter missing");b.click();',
      [title]
    );
    await wait(
      () =>
        execute(
          'return document.querySelector("[data-testid=local-activity-revisit]")?.innerText.includes(arguments[0]);',
          [title]
        ),
      'exact prerequisite chapter'
    );
  }
  const db = path.join(data, 'data/com.dshanpi.lynx/learning.sqlite');
  const history = () =>
    JSON.parse(
      run('python3', [
        '-c',
        `import sqlite3,json,sys,hashlib
from pathlib import Path
c=sqlite3.connect(Path(sys.argv[1]).as_uri()+'?mode=ro',uri=True)
result={}
for table in ['learning_attempts','learning_evidence']:
 result[table]={str(r[0]):hashlib.sha256(json.dumps(r[1:],ensure_ascii=False,separators=(',',':')).encode()).hexdigest() for r in c.execute('SELECT rowid,* FROM '+table+' WHERE session_id=? ORDER BY rowid',(sys.argv[2],))}
print(json.dumps(result));c.close()`,
        db,
        sessionId,
      ])
    );
  await transition('checking_understanding');
  const draft = '独立理论草稿：切换活动和重启后恢复。';
  await ipc('learning_save_draft', {
    sessionId,
    stepId: 'intro',
    attempt: 1,
    answer: draft,
    reflection: '草稿保留',
  });
  const before = history();
  await reject(() => choose('run'), /LEARNING_PREREQUISITES_REQUIRED/);
  assert.deepEqual(history(), before);
  await showSession();
  await preview('运行并核验真实输出');
  assert.equal(
    await execute(
      'return [...document.querySelectorAll("[data-testid=local-activity-revisit] button")].some(b=>b.innerText==="开始本课学习"&&b.disabled);'
    ),
    true
  );
  assert.equal((await step()).session.currentStep, 'intro');
  await frame();
  await shot('prerequisites-blocked-preview');
  await preview('独立选修阅读');
  await clickText('开始本课学习');
  await wait(
    async () => (await step()).session.currentStep === 'extra',
    'independent activity selected'
  );
  assert.equal(latest(await journey(), 'intro').state, 'checking_understanding');
  await closeApp();
  await startApp();
  await skip();
  assert.equal((await step()).session.currentStep, 'intro');
  assert.equal((await journey()).answer, draft);
  await showSession();
  await wait(
    () =>
      execute(
        'return [...document.querySelectorAll("textarea")].some(e=>e.value===arguments[0]);',
        [draft]
      ),
    'actual restored graph draft'
  );
  await execute(
    'const e=[...document.querySelectorAll("textarea")].find(e=>e.value===arguments[0]);e.scrollIntoView({block:"center"});return true;',
    [draft]
  );
  await wait(
    () => execute('return document.body.innerText.includes("草稿已保存到本地");'),
    'draft save acknowledgement'
  );
  await frame();
  await shot('prerequisites-restored-draft');
  await closeApp();
  await startApp();
  check(
    'actual chapter UI keeps blocked content readonly, starts independent activity explicitly, saves the original draft and restores it after optional skip and restart'
  );
  await finish();
  assert.equal((await step()).session.currentStep, 'source');
  await skip();
  const blocked = await report();
  assert.equal(blocked.softwareResult, 'INCOMPLETE');
  assert.equal(blocked.requiredCompletedSteps, 1);
  assert.equal(blocked.session.classification, 'INCOMPLETE');
  assert.equal(blocked.optionalSkippedSteps, 2);
  assert.equal(blocked.prerequisiteChecks.find((a) => a.stepId === 'run').available, false);
  await showSession();
  await wait(
    () => execute('return !!document.querySelector("[data-testid=local-report-prerequisites]");'),
    'blocked graph report'
  );
  await execute(
    'document.querySelector("[data-testid=local-report-prerequisites]").scrollIntoView({block:"center"});return true;'
  );
  await frame();
  await shot('prerequisites-skipped-ancestor');
  await closeApp();
  await startApp();
  await choose('source');
  assert.equal((await journey()).attempt, 2);
  await ipc('learning_bind_local_project', { sessionId, projectId });
  await finish();
  assert.equal((await step()).session.currentStep, 'build');
  check(
    'skipped optional prerequisite never unlocks required descendants; no available activity preserves the skip and incomplete report, and explicit rework recovers'
  );
  await practice();
  const sourceFile = path.join(sdk, 'lesson/src/main.c'),
    originalSource = await readFile(sourceFile);
  const sdkFile = path.join(sdk, 'shared-sdk/VERSION'),
    originalSdk = await readFile(sdkFile);
  const marker = path.join(sdk, 'lesson/build/invocations.log');
  const missing = async (file) =>
    stat(file).then(
      () => false,
      (e) => {
        if (e.code === 'ENOENT') return true;
        throw e;
      }
    );
  const rejectedInputs = [];
  await appendFile(sourceFile, '\n/* changed before dependent command */\n');
  try {
    rejectedInputs.push(await reject(action, /LEARNING_PREREQUISITE_INPUTS_CHANGED/));
    assert.ok(await missing(marker));
  } finally {
    await writeFile(sourceFile, originalSource);
  }
  await rename(sdkFile, sdkFile + '.absent');
  try {
    rejectedInputs.push(await reject(action, /LEARNING_(?:PREREQUISITE_)?INPUTS_CHANGED/));
    assert.ok(await missing(marker));
  } finally {
    await rename(sdkFile + '.absent', sdkFile);
  }
  await writeFile(sdkFile, 'changed external prerequisite SDK\n');
  try {
    rejectedInputs.push(await reject(action, /LEARNING_(?:PREREQUISITE_)?INPUTS_CHANGED/));
    assert.ok(await missing(marker));
  } finally {
    await writeFile(sdkFile, originalSdk);
  }
  assert.equal((await action()).status, 'PASS');
  assert.equal(await readFile(marker, 'utf8'), 'build invoked\n');
  await writeFile(sdkFile, 'changed before dependent advancement\n');
  try {
    await reject(() => transition('verifying'), /LEARNING_(?:PREREQUISITE_)?INPUTS_CHANGED/);
  } finally {
    await writeFile(sdkFile, originalSdk);
  }
  await transition('verifying');
  await transition('reflecting');
  await transition('completed', { reflection: '前置源码与外部 SDK 恢复后，真实构建和推进通过。' });
  assert.equal((await step()).session.currentStep, 'run');
  await finish();
  const initialReport = await report();
  assert.equal(initialReport.softwareResult, 'PASS');
  assert.equal(initialReport.requiredCompletedSteps, 3);
  assert.equal(initialReport.optionalSkippedSteps, 1);
  assert.deepEqual(
    initialReport.currentWorkspace.environmentChecks.map((c) => [c.stepId, c.status]),
    [['source', 'MATCH']]
  );
  check(
    'real source changes and missing/mismatched external prerequisite SDK block the dependent command before invocation; restored inputs allow actual compile and advancement'
  );
  const oldHistory = history();
  await choose('source');
  const invalidated = await report();
  assert.equal(invalidated.requiredCompletedSteps, 1);
  assert.equal(invalidated.softwareResult, 'INCOMPLETE');
  await appendFile(sourceFile, '\n/* learner change during prerequisite rework */\n');
  await finish();
  const changed = await report();
  assert.equal(changed.recordedSoftwareResult, 'PASS');
  assert.equal(changed.softwareResult, 'REVALIDATION_REQUIRED');
  await choose('run');
  await practice();
  const beforeReject = history();
  const runMarker = path.join(sdk, 'lesson/build/run-invocations.log');
  const invokedBefore = await readFile(runMarker, 'utf8');
  assert.equal(invokedBefore, 'run invoked\n');
  await reject(action, /LEARNING_PREREQUISITE_INPUTS_CHANGED/);
  const afterReject = history();
  assert.deepEqual(afterReject.learning_attempts, beforeReject.learning_attempts);
  for (const [id, sha] of Object.entries(beforeReject.learning_evidence))
    assert.equal(afterReject.learning_evidence[id], sha);
  const appended = Object.keys(afterReject.learning_evidence).filter(
    (id) => !(id in beforeReject.learning_evidence)
  );
  assert.equal(appended.length, 1);
  const rejectedEvidence = JSON.parse(
    run('python3', [
      '-c',
      `import sqlite3,json,sys
from pathlib import Path
c=sqlite3.connect(Path(sys.argv[1]).as_uri()+'?mode=ro',uri=True)
r=c.execute('SELECT status,source,detail_json FROM learning_evidence WHERE session_id=? AND id=?',(sys.argv[2],int(sys.argv[3]))).fetchone()
print(json.dumps(dict(status=r[0],source=r[1],detail=json.loads(r[2]))));c.close()`,
      db,
      sessionId,
      appended[0],
    ])
  );
  assert.equal(rejectedEvidence.status, 'ERROR');
  assert.equal(rejectedEvidence.source, 'software');
  assert.match(rejectedEvidence.detail.error, /LEARNING_PREREQUISITE_INPUTS_CHANGED/);
  assert.equal(rejectedEvidence.detail.exitCode, undefined);
  assert.equal(await readFile(runMarker, 'utf8'), invokedBefore);
  await choose('build');
  await finish();
  assert.equal((await step()).session.currentStep, 'run');
  assert.equal((await journey()).attempt, 2);
  await finish();
  const finalReport = await report();
  assert.equal(finalReport.softwareResult, 'PASS');
  assert.equal(finalReport.requiredCompletedSteps, 3);
  const after = history();
  for (const table of Object.keys(oldHistory))
    for (const [id, sha] of Object.entries(oldHistory[table])) assert.equal(after[table][id], sha);
  check(
    'restarting an optional ancestor invalidates current descendant completion without deleting history; early new execution cannot hide old intermediate sources, and actual rework restores PASS'
  );
  await closeApp();
  await startApp();
  const reopened = await report();
  assert.equal(reopened.softwareResult, 'PASS');
  await showSession();
  await wait(
    () =>
      execute(
        'return document.querySelector("[data-testid=local-report-sources]")?.innerText.includes("与执行时一致");'
      ),
    'final graph source currency'
  );
  await execute(
    'document.querySelector("[data-testid=local-report-sources]").scrollIntoView({block:"center"});return true;'
  );
  await frame();
  await shot('prerequisites-completed');
  evidence.prerequisites = {
    fixture,
    packages,
    sessionId,
    projectId,
    created,
    rejectedPackages,
    rejectedInputs,
    blocked,
    initialReport,
    invalidated,
    changed,
    finalReport: reopened,
    oldHistory,
    oldHistoryPreserved: true,
    hardwareActionsExecuted: false,
  };
  check(
    'final graph report survives native restart with exact required/optional progress and preserved attempts/evidence'
  );
}
