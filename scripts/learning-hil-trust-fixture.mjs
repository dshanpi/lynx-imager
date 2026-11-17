import { nextFixturePort } from './lib/fixture-ports.mjs';
// Signed synthetic course, real Tauri IPC and loopback SSH. No hardware operations.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';

export async function verifyHilTrustFixture(ctx) {
  const {
    data,
    hostId,
    ipc,
    reject,
    run,
    wait,
    execute,
    clickText,
    shot,
    showSession,
    startApp,
    closeApp,
    evidence,
  } = ctx;
  const packages = path.join(data, 'packages-hil');
  run('python3', [
    'scripts/generate-learning-smoke-course.py',
    packages,
    '--scenario',
    'hil-trust',
  ]);
  const fixture = JSON.parse(await readFile(path.join(packages, 'fixture.json'), 'utf8'));
  const deviceId = await ipc('workbench_save_device', {
    device: {
      id: null,
      boardUid: '',
      name: 'Synthetic HIL trust fixture - no hardware',
      boardImagePath: '',
      chipVendor: 'allwinner',
      serialPort: '',
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
  const projectId = await ipc('workbench_save_project', {
    project: {
      id: null,
      name: 'Synthetic HIL trust - no device actions',
      hostId,
      deviceId,
      sdkDir: path.join(packages, 'prepared-sdk'),
      buildCommand: 'true',
      firmwarePath: '',
      localDownloadDir: data,
      verifySha256: true,
      localGatewayPort: await nextFixturePort(),
      remoteForwardPort: await nextFixturePort(),
      mcpServerName: 'hil-trust-fixture',
      aiProvider: 'codex',
    },
  });
  await ipc('learning_import_local_course', { path: path.join(packages, 'course.lynx-course') });
  const created = await ipc('learning_start_session', {
    courseId: fixture.courseId,
    version: fixture.version,
    projectId,
  });
  const sessionId = created.sessionId;
  await ipc('learning_transition_step', { sessionId, nextState: 'checking_understanding' });
  await ipc('learning_transition_step', {
    sessionId,
    nextState: 'practicing',
    answer: '人工声称不能证明设备实测。',
  });
  const software = await ipc('learning_run_step_action', { sessionId, credentials: {} });
  assert.equal(software.status, 'PASS');
  assert.equal((await ipc('learning_get_local_step', { sessionId })).session.state, 'practicing');
  const prepare = await ipc('learning_prepare_hil_action', { sessionId });
  await ipc('learning_confirm_hil_action', {
    request: {
      sessionId,
      approvalToken: prepare.approvalToken,
      confirmation: prepare.confirmationPhrase,
    },
  });
  const request = {
    sessionId,
    expectedStepId: 'source',
    expectedAttempt: 1,
    evidenceKind: 'wiring-observation',
    status: 'PASS',
    summary: '合成人工声称通过',
    observation: '未连接硬件；这些文字不是设备回执。',
    durationSeconds: 7200,
    artifactSha256: prepare.image.sha256,
  };
  await reject(
    'learning_attach_hil_evidence',
    { request: { ...request, expectedAttempt: 2 } },
    /LEARNING_ACTION_CONTEXT_CHANGED/
  );
  await reject(
    'learning_attach_hil_evidence',
    { request: { ...request, source: 'hil-device' } },
    /unknown field/
  );
  const observations = [];
  for (const evidenceKind of prepare.requiredEvidenceKinds) {
    const value = await ipc('learning_attach_hil_evidence', {
      request: { ...request, evidenceKind },
    });
    assert.equal(value.source, 'user-asserted');
    assert.equal(value.status, 'RECORDED');
    observations.push(value);
  }
  const assessment = await ipc('learning_finalize_hil_step', { sessionId });
  assert.equal(assessment.status, 'NOT_VERIFIED');
  await reject(
    'learning_transition_step',
    { sessionId, nextState: 'collecting_evidence' },
    /LEARNING_ACTION_REQUIRED/
  );
  const report = await ipc('learning_generate_local_report', { sessionId });
  assert.equal(report.hilResult, 'NOT_VERIFIED');
  assert.equal(report.hilPassedSteps, 0);
  assert.equal(report.session.state, 'practicing');
  await closeApp();
  await startApp();
  const reopened = await ipc('learning_get_local_step', { sessionId });
  assert.equal(reopened.session.state, 'practicing');
  assert.equal(
    reopened.evidence.filter((e) => e.source === 'user-asserted' && e.status === 'RECORDED').length,
    3
  );
  await showSession();
  // A new UI session requires a fresh explicit confirmation before showing the form.
  await clickText('核对观察目标并生成确认');
  await wait(
    () => execute('return !!document.querySelector(".local-course-hil-confirm code");'),
    'observation confirmation'
  );
  await execute(`const box=document.querySelector('.local-course-hil-confirm input');
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(box,document.querySelector('.local-course-hil-confirm code').textContent);
    box.dispatchEvent(new Event('input',{bubbles:true}));`);
  await clickText('确认本次观察范围');
  await wait(
    () => execute('return document.body.innerText.includes("保存人工观察（未验证）")'),
    'unverified observation UI'
  );
  await execute(`const form=document.querySelector('.local-course-hil-evidence-form');
    const select=form.querySelectorAll('select')[1];select.value='FAIL';select.dispatchEvent(new Event('change',{bubbles:true}));
    const input=form.querySelector('input');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,'界面保存的人工失败观察');input.dispatchEvent(new Event('input',{bubbles:true}));
    const text=form.querySelector('textarea');Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set.call(text,'合成观察，没有连接设备');text.dispatchEvent(new Event('input',{bubbles:true}));`);
  await clickText('保存人工观察（未验证）');
  await wait(
    async () =>
      (await ipc('learning_get_local_step', { sessionId })).evidence.some(
        (e) =>
          e.source === 'user-asserted' &&
          e.status === 'RECORDED' &&
          e.summary.includes('界面保存的人工失败观察')
      ),
    'UI manual observation is recorded without a hardware grade'
  );
  const assessmentsBefore = (await ipc('learning_get_local_step', { sessionId })).evidence.filter(
    (e) => e.source === 'hil-assessment'
  ).length;
  await clickText('检查实机证据缺口');
  await wait(
    async () =>
      (await ipc('learning_get_local_step', { sessionId })).evidence.filter(
        (e) => e.source === 'hil-assessment' && e.status === 'NOT_VERIFIED'
      ).length ===
      assessmentsBefore + 1,
    'UI writes a new unverified assessment'
  );
  await wait(
    () =>
      execute(
        'return document.querySelector(".local-course-hil-evidence-list")?.innerText.includes("NOT_VERIFIED")'
      ),
    'UI assessment stays unverified'
  );
  await execute('document.querySelector(".local-course-hil").scrollIntoView({block:"start"});');
  await shot('hil-manual-unverified');
  evidence.checks.push({
    name: 'real signed HIL course and software execution: matching manual PASS/hash/7200s never grants HIL pass or progression',
    status: 'PASS',
  });

  // Seed historical records only while the newly created isolated application is closed.
  await closeApp();
  const database = path.join(data, 'data/com.dshanpi.lynx/learning.sqlite');
  const raw = run('python3', [
    '-c',
    `import sqlite3,sys,json,hashlib
db=sqlite3.connect(sys.argv[1]); sid=sys.argv[2]
assert db.execute('SELECT course_id FROM learning_sessions WHERE session_id=?',(sid,)).fetchone()[0]=='lynx-local-hil-trust-acceptance'
cols=[r[1] for r in db.execute('PRAGMA table_info(learning_evidence)') if r[1]!='id']
row=dict(zip(cols,db.execute('SELECT '+','.join(cols)+' FROM learning_evidence WHERE session_id=? ORDER BY id DESC LIMIT 1',(sid,)).fetchone()))
for kind,source in [('wiring-observation','hil-device'),('hil-verification','hil-validator')]:
 row.update(kind=kind,source=source,status='PASS',summary='Synthetic historical record',detail_json='{}',digest=hashlib.sha256(source.encode()).hexdigest())
 db.execute('INSERT INTO learning_evidence('+','.join(cols)+') VALUES('+','.join('?' for _ in cols)+')',[row[c] for c in cols])
db.execute("UPDATE learning_attempts SET state='completed' WHERE session_id=?",(sid,))
db.execute("UPDATE learning_sessions SET state='completed',classification='FULL_PASS' WHERE session_id=?",(sid,));db.commit()
print(json.dumps(db.execute("SELECT source,status,summary,detail_json,digest FROM learning_evidence WHERE session_id=? AND source IN ('hil-device','hil-validator') ORDER BY id",(sid,)).fetchall()))
db.close()`,
    database,
    sessionId,
  ]).trim();
  await startApp();
  const legacyReport = await ipc('learning_generate_local_report', { sessionId });
  assert.equal(legacyReport.hilResult, 'NOT_VERIFIED');
  assert.equal(legacyReport.recordedHilResult, 'PASS');
  assert.equal(legacyReport.hilPassedSteps, 0);
  assert.equal(legacyReport.session.classification, 'HIL_NOT_VERIFIED');
  assert.equal(legacyReport.session.recordedClassification, 'FULL_PASS');
  const journey = await ipc('learning_get_journey', { sessionId });
  assert.ok(
    journey.evidence.some(
      (e) =>
        e.recorded?.source === 'hil-device' &&
        e.recorded.status === 'PASS' &&
        e.source === 'user-asserted' &&
        e.status === 'RECORDED'
    )
  );
  assert.ok(
    journey.evidence.some(
      (e) =>
        e.recorded?.source === 'hil-validator' &&
        e.recorded.status === 'PASS' &&
        e.status === 'NOT_VERIFIED'
    )
  );
  const after = run('python3', [
    '-c',
    `import sqlite3,sys,json
from pathlib import Path
db=sqlite3.connect(Path(sys.argv[1]).as_uri()+'?mode=ro',uri=True)
print(json.dumps(db.execute("SELECT source,status,summary,detail_json,digest FROM learning_evidence WHERE session_id=? AND source IN ('hil-device','hil-validator') ORDER BY id",(sys.argv[2],)).fetchall()))`,
    database,
    sessionId,
  ]).trim();
  assert.equal(after, raw, 'legacy raw evidence is retained byte for byte');
  await showSession();
  await wait(
    () =>
      execute(
        'return !!document.querySelector(".learn-records [data-testid=local-report-currency]");'
      ),
    'initial session and automatic report have loaded'
  );
  await clickText('生成最终学习报告');
  await wait(
    () =>
      execute(
        'return document.querySelector(".local-course-report")?.innerText.includes("NOT_VERIFIED")'
      ),
    'legacy report is unverified'
  );
  await shot('hil-legacy-unverified');
  const safety = await ipc('hardware_safety_status');
  assert.equal(safety.busy, false);
  evidence.hilTrust = {
    fixture,
    sessionId,
    observations,
    assessment,
    report,
    legacyReport,
    legacyRawPreserved: raw === after,
    syntheticLegacyRows: true,
    hardwareActionsExecuted: false,
    safety,
    journey,
  };
  evidence.checks.push({
    name: 'native restart preserves historical raw PASS rows but projects unverified HIL, original classification and evidence origin',
    status: 'PASS',
  });
}
