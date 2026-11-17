// Real Tauri and generated course only. No hardware, cloud grades or synthetic PASS rows.
import assert from 'node:assert/strict';
import path from 'node:path';
import { appendFile } from 'node:fs/promises';

export async function verifyRevisit(ctx) {
  const {
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
  } = ctx;
  const sessionId = evidence.report.session.sessionId;
  const step = () => ipc('learning_get_local_step', { sessionId });
  const journey = () => ipc('learning_get_journey', { sessionId });
  const report = () => ipc('learning_generate_local_report', { sessionId });
  const transition = (nextState, extra = {}) =>
    ipc('learning_transition_step', { sessionId, nextState, ...extra });
  const latest = (j, id) =>
    j.attempts.filter((a) => a.stepId === id).sort((a, b) => b.attempt - a.attempt)[0];
  const db = path.join(data, 'data/com.dshanpi.lynx/learning.sqlite');
  function history() {
    return JSON.parse(
      run('python3', [
        '-c',
        `import sqlite3,json,sys,hashlib
from pathlib import Path
c=sqlite3.connect(Path(sys.argv[1]).as_uri()+'?mode=ro',uri=True)
result={}
for table in ['learning_attempts','learning_evidence']:
 result[table]={str(row[0]):hashlib.sha256(json.dumps(row[1:],ensure_ascii=False,separators=(',',':')).encode()).hexdigest() for row in c.execute('SELECT rowid,* FROM '+table+' WHERE session_id=? ORDER BY rowid',(sys.argv[2],))}
print(json.dumps(result));c.close()`,
        db,
        sessionId,
      ])
    );
  }
  async function revisit(targetStepId) {
    const view = await step(),
      j = await journey();
    return ipc('learning_revisit_activity', {
      request: {
        sessionId,
        expectedStepId: view.session.currentStep,
        expectedState: view.session.state,
        expectedAttempt: j.attempt,
        targetStepId,
        expectedTargetAttempt: latest(j, targetStepId).attempt,
      },
    });
  }
  async function complete() {
    const view = await step(),
      j = await journey();
    if (view.lab.required === false) {
      await ipc('learning_skip_optional_activity', {
        request: {
          sessionId,
          expectedStepId: view.session.currentStep,
          expectedState: view.session.state,
          expectedAttempt: j.attempt,
        },
      });
      return;
    }
    if (view.session.state === 'reading') await transition('checking_understanding');
    if (view.lab.activityKind === 'reflection')
      await transition('reflecting', { answer: '重新理解理论内容，自述不替代实际执行。' });
    else {
      await transition('practicing', { answer: '本次重新核对源代码并执行，保留历史。' });
      const action = await ipc('learning_run_step_action', { sessionId, credentials: {} });
      assert.equal(action.status, 'PASS');
      await transition('verifying');
      await transition('reflecting');
    }
    await transition('completed', {
      reflection: '本次重新完成；旧尝试与证据保留，未进行任何硬件动作。',
    });
  }
  await closeApp();
  await startApp();
  const original = await step(),
    originalJourney = await journey(),
    oldHistory = history();
  assert.equal(original.session.state, 'completed');
  const [first, second] = originalJourney.labs;
  assert.ok(first && second);
  await showSession(sessionId);
  await wait(
    () => execute('return document.querySelectorAll(".learn-chapter-review button").length>0;'),
    'completed record chapters'
  );
  await execute(
    'const b=[...document.querySelectorAll(".learn-chapter-review button")].find(b=>b.innerText.includes(arguments[0]));if(!b)throw new Error("exact chapter missing");b.click();',
    [first.title]
  );
  await wait(
    () =>
      execute('return !!document.querySelector("[data-testid=local-activity-revisit] button");'),
    'explicit revisit entry'
  );
  const preview = await step();
  assert.equal(preview.session.currentStep, original.session.currentStep);
  assert.equal((await journey()).attempts.length, originalJourney.attempts.length);
  const firstDocument = await ipc('learning_read_document', { sessionId, stepId: first.stepId });
  assert.ok(firstDocument?.markdown);
  const firstHeading = firstDocument.markdown
    .split('\n')
    .find((line) => line.startsWith('# '))
    .slice(2);
  await wait(
    () =>
      execute(
        'return [...document.querySelectorAll(".learn-panel .learn-document-markdown")].some(e=>e.innerText.includes(arguments[0]));',
        [firstHeading]
      ),
    'signed selected chapter document'
  );
  await clickText('重新学习本课');
  await wait(async () => {
    const s = await step();
    return s.session.currentStep === first.stepId && s.session.state === 'reading';
  }, 'new selected reading attempt');
  await wait(
    () =>
      execute(
        'return document.querySelector(".local-course-session-shell")?.innerText.includes("阅读目标");'
      ),
    'workspace new attempt rendered'
  );
  const restarted = await journey();
  assert.equal(restarted.attempt, latest(originalJourney, first.stepId).attempt + 1);
  assert.equal(restarted.answer, '');
  const pendingReport = await report();
  if (first.required !== false) {
    assert.equal(pendingReport.completedSteps, evidence.report.completedSteps - 1);
    if (first.activityKind !== 'reflection')
      assert.equal(pendingReport.softwareResult, 'INCOMPLETE');
  }
  assert.equal(
    Object.keys(history().learning_evidence).length,
    Object.keys(oldHistory.learning_evidence).length + 1,
    'only explicit navigation event, no execution'
  );
  evidence.checks.push({
    name:
      scenario +
      ': exact record chapter preview stays readonly; explicit revisit creates a clean attempt and changes current progress without losing historical PASS',
    status: 'PASS',
  });
  await closeApp();
  await startApp();
  await transition('checking_understanding');
  const firstDraft = '原课未完成草稿：切换后仍应恢复。';
  await ipc('learning_save_draft', {
    sessionId,
    stepId: first.stepId,
    attempt: restarted.attempt,
    answer: firstDraft,
    reflection: '原课复盘草稿',
  });
  await revisit(second.stepId);
  const secondJourney = await journey();
  assert.equal(secondJourney.attempt, latest(originalJourney, second.stepId).attempt + 1);
  await ipc('learning_save_draft', {
    sessionId,
    stepId: second.stepId,
    attempt: secondJourney.attempt,
    answer: '第二课新尝试草稿',
    reflection: '第二课复盘',
  });
  await revisit(first.stepId);
  const resumed = await journey();
  assert.equal(resumed.attempt, restarted.attempt);
  assert.equal(resumed.answer, firstDraft);
  assert.equal((await step()).session.state, 'checking_understanding');
  await closeApp();
  await startApp();
  const reopened = await journey();
  assert.equal(reopened.answer, firstDraft);
  assert.equal(reopened.attempt, restarted.attempt);
  await showSession(sessionId);
  await wait(
    () =>
      execute(
        'return [...document.querySelectorAll("textarea")].some(e=>e.value===arguments[0]);',
        [firstDraft]
      ),
    'native restored selected draft'
  );
  await execute(
    'return new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve(true))));'
  );
  await shot('revisit-restored-draft');
  await closeApp();
  await startApp();
  if (scenario === 'c')
    await appendFile(
      path.join(data, 'packages/prepared-sdk/lesson/src/main.c'),
      '\n/* Explicit learner change during revisit acceptance. */\n'
    );
  await complete();
  const advanced = await step(),
    advancedJourney = await journey();
  assert.equal(advanced.session.currentStep, second.stepId);
  assert.equal(advancedJourney.attempt, secondJourney.attempt);
  assert.equal(advancedJourney.answer, '第二课新尝试草稿');
  await complete();
  let oldLaterSources;
  if (scenario === 'c') {
    oldLaterSources = await report();
    assert.equal(oldLaterSources.recordedSoftwareResult, 'PASS');
    assert.equal(oldLaterSources.softwareResult, 'REVALIDATION_REQUIRED');
    assert.deepEqual(
      oldLaterSources.currentWorkspace.sourceChecks.map((c) => [c.stepId, c.status]),
      [
        ['source', 'MATCH'],
        ['build', 'MATCH'],
        ['run', 'CHANGED'],
      ]
    );
    await showSession(sessionId);
    await wait(
      () =>
        execute(
          'return document.querySelector("[data-testid=local-report-sources]")?.innerText.includes("已变化，需重新验证");'
        ),
      'old later execution source mismatch'
    );
    await execute(
      'document.querySelector("[data-testid=local-report-sources]").scrollIntoView({block:"center"});return new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve(true))));'
    );
    await shot('revisit-old-source-rejected');
    await closeApp();
    await startApp();
    await revisit('run');
    await complete();
    evidence.checks.push({
      name: 'c: real source edit and early re-execution cannot hide a later old-input PASS; explicit final-step revalidation restores the report',
      status: 'PASS',
    });
  }
  const finished = await report();
  assert.equal(finished.session.state, 'completed');
  assert.equal(finished.softwareResult, evidence.report.softwareResult);
  assert.equal(finished.completedSteps, evidence.report.completedSteps);
  assert.equal(finished.optionalSkippedSteps, evidence.report.optionalSkippedSteps);
  const newHistory = history();
  for (const table of Object.keys(oldHistory))
    for (const [id, digest] of Object.entries(oldHistory[table]))
      assert.equal(newHistory[table][id], digest, table + ' old row ' + id + ' remains unchanged');
  evidence.checks.push({
    name:
      scenario +
      ': existing unfinished attempts and drafts survive selection and restart; actual completion resumes the other pending attempt, then recomputes completion without duplicate rows',
    status: 'PASS',
  });
  await closeApp();
  await startApp();
  const finalReport = await report();
  assert.equal(finalReport.softwareResult, finished.softwareResult);
  assert.equal(finalReport.completedSteps, finished.completedSteps);
  await showSession(sessionId);
  await wait(
    () =>
      execute(
        'return !!document.querySelector(".learn-records [data-testid=local-report-currency]");'
      ),
    'revisited final report'
  );
  await execute(
    'document.querySelector(".learn-summary-main").scrollIntoView({block:"start"});return new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve(true))));'
  );
  await shot('revisit-completed');
  evidence.revisit = {
    scenario,
    sessionId,
    firstStep: first.stepId,
    secondStep: second.stepId,
    original: original.session,
    restartedAttempt: restarted.attempt,
    pendingReport,
    resumedAttempt: resumed.attempt,
    secondAttempt: secondJourney.attempt,
    reopenedDraftMatched: true,
    oldHistory,
    oldHistoryPreserved: true,
    finalReport,
    hardwareActionsExecuted: false,
    oldLaterSources,
  };
  evidence.checks.push({
    name:
      scenario +
      ': final reopened report and immutable original attempt/evidence rows verified; explicit navigation never manufactures software or HIL results',
    status: 'PASS',
  });
}
