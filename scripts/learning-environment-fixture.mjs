import { nextFixturePort } from './lib/fixture-ports.mjs';
// Real desktop/SSH observations of a signed synthetic BSP course; no physical device.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile, writeFile, rename } from 'node:fs/promises';
import path from 'node:path';

export async function verifyEnvironmentFixture(ctx) {
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
    learningStoreDigest,
    evidence,
  } = ctx;
  // Close the previous learning UI so draft/archive timers cannot affect read-only proof.
  await closeApp();
  await startApp();
  const packages = path.join(data, 'packages-environment');
  run('python3', [
    'scripts/generate-learning-smoke-course.py',
    packages,
    '--scenario',
    'environment',
  ]);
  const fixture = JSON.parse(await readFile(path.join(packages, 'fixture.json'), 'utf8'));
  const original = (await ipc('workbench_list_projects')).find((p) => p.id === originalProjectId);
  const sdk = path.join(packages, 'prepared-sdk');
  const projectId = await ipc('workbench_save_project', {
    project: {
      ...original,
      id: null,
      name: '合成 BSP 环境检查工程（无硬件）',
      mcpServerName: 'learning-environment-acceptance',
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
  const before = learningStoreDigest();
  const inspect = () => ipc('learning_inspect_project', { sessionId });
  const initial = await inspect();
  assert.equal(initial.projectId, projectId);
  assert.equal(initial.stepId, 'source');
  assert.equal(initial.attempt, 1);
  assert.equal(initial.source.kind, 'directory');
  assert.equal(initial.checks.length, 5);
  assert.ok(initial.checks.every((c) => c.status === 'MATCH'));
  for (const tool of initial.tools.filter((t) => t.status === 'OBSERVED')) {
    assert.ok(tool.path.startsWith('/usr/') || tool.path.startsWith('/bin/'));
    assert.equal(
      createHash('sha256')
        .update(await readFile(tool.path))
        .digest('hex'),
      tool.sha256
    );
  }
  const after = learningStoreDigest();
  assert.equal(after, before, 'inspection does not write any learning database table');
  const versionFile = path.join(sdk, 'lesson/sdk/VERSION');
  const version = await readFile(versionFile);
  await rename(versionFile, versionFile + '.absent');
  let missing;
  try {
    missing = await inspect();
    assert.equal(missing.checks.find((c) => c.spec.id === 'sdk').status, 'MISSING');
    assert.ok(missing.checks.filter((c) => c.spec.id !== 'sdk').every((c) => c.status === 'MATCH'));
  } finally {
    await rename(versionFile + '.absent', versionFile);
  }
  await writeFile(versionFile, 'Different synthetic SDK revision\n');
  const mismatch = await inspect();
  assert.equal(mismatch.checks.find((c) => c.spec.id === 'sdk').status, 'MISMATCH');
  assert.notEqual(mismatch.fingerprint, initial.fingerprint);
  await showSession();
  await clickText('工程与版本');
  await clickText('连接 SSH 并检查源码');
  await wait(
    () =>
      execute(
        'return document.querySelector("[data-testid=local-environment-checks]")?.innerText.includes("与要求不符")'
      ),
    'actual SDK mismatch UI'
  );
  await execute(
    'document.querySelector("[data-testid=local-environment-checks]").scrollIntoView({block:"center"});'
  );
  await execute(
    'return new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve(true))));'
  );
  await shot('environment-mismatch');
  assert.equal(
    await execute(
      'return document.querySelector("[data-testid=local-environment-checks]")?.innerText.includes("与要求不符")'
    ),
    true
  );
  await writeFile(versionFile, version);
  await clickText('连接 SSH 并检查源码');
  await wait(
    () =>
      execute(
        'return document.querySelectorAll("[data-testid=local-environment-checks] tbody tr").length===5 && [...document.querySelectorAll("[data-testid=local-environment-checks] tbody tr")].every(e=>e.innerText.includes("符合声明"))'
      ),
    'restored SDK requirements all match'
  );
  await execute(
    'document.querySelector("[data-testid=local-environment-checks]").scrollIntoView({block:"center"});'
  );
  await execute(
    'return new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve(true))));'
  );
  await shot('environment-restored');
  assert.equal(
    await execute(
      'return [...document.querySelectorAll("[data-testid=local-environment-checks] tbody tr")].filter(e=>e.innerText.includes("符合声明")).length'
    ),
    5
  );
  const restored = await inspect();
  assert.ok(restored.checks.every((c) => c.status === 'MATCH'));
  assert.equal((await ipc('learning_get_local_step', { sessionId })).session.state, 'reading');
  assert.equal((await ipc('learning_get_local_step', { sessionId })).evidenceCount, 0);
  evidence.checks.push({
    name: 'signed schema 7 BSP course: actual compiler version and digest, non-Git SDK, missing/mismatched/restored file and readonly database',
    status: 'PASS',
  });

  // Git is optional metadata; its real state is observed without replacing SDK checks.
  run('git', ['-C', sdk, 'init', '-q']);
  const unborn = await inspect();
  assert.equal(unborn.source.kind, 'unborn');
  run('git', ['-C', sdk, 'add', 'lesson']);
  run('git', [
    '-C',
    sdk,
    '-c',
    'user.name=Environment Fixture',
    '-c',
    'user.email=fixture@example.invalid',
    'commit',
    '-qm',
    'synthetic SDK',
  ]);
  await writeFile(path.join(sdk, 'untracked-note.txt'), 'Synthetic untracked note\n');
  const git = await inspect();
  assert.equal(git.source.kind, 'git');
  assert.equal(git.source.untracked, true);
  assert.equal(git.source.dirty, true);
  assert.match(git.source.commit, /^[a-f0-9]{40,64}$/);
  await closeApp();
  await startApp();
  const reopened = await inspect();
  assert.equal(reopened.source.commit, git.source.commit);
  assert.ok(reopened.checks.every((c) => c.status === 'MATCH'));
  assert.equal((await ipc('learning_get_local_step', { sessionId })).session.state, 'reading');
  evidence.checks.push({
    name: 'real Git empty/committed/untracked state and environment checks survive explicit safe app restart without advancing learning',
    status: 'PASS',
  });
  const transition = (nextState, extra = {}) =>
    ipc('learning_transition_step', { sessionId, nextState, ...extra });
  const actions = [];
  for (const stepId of ['source', 'build', 'run']) {
    assert.equal((await ipc('learning_get_local_step', { sessionId })).session.currentStep, stepId);
    const current = await inspect();
    assert.ok(current.checks.every((c) => c.status === 'MATCH'));
    await transition('checking_understanding');
    await transition('practicing', {
      answer: '环境检查记录实际版本与摘要；编译、设备验证仍需各自证据。',
    });
    const result = await ipc('learning_run_step_action', { sessionId, credentials: {} });
    assert.equal(result.status, 'PASS');
    actions.push(result);
    await transition('verifying');
    await transition('reflecting');
    await transition('completed', {
      reflection: '本课实际生成并核对合成 ARM 配置与设备树，没有部署到 T113-Pro。',
    });
  }
  const report = await ipc('learning_generate_local_report', { sessionId });
  assert.equal(report.completedSteps, 3);
  assert.equal(report.softwareResult, 'PASS');
  assert.equal(report.hilResult, 'NOT_REQUIRED');
  await showSession();
  await wait(
    () =>
      execute(
        'return !!document.querySelector(".learn-records [data-testid=local-report-currency]");'
      ),
    'completed environment course report'
  );
  await clickText('生成最终学习报告');
  await wait(
    () =>
      execute(
        'return document.querySelector(".local-course-report")?.textContent.includes("PASS")'
      ),
    'environment course final report'
  );
  await shot('environment-completed');
  evidence.environment = {
    fixture,
    sessionId,
    initial,
    missing,
    mismatch,
    restored,
    unborn,
    git,
    reopened,
    readonlyStore: { before, after },
    actions,
    report,
    hardwareActionsExecuted: false,
  };
  evidence.checks.push({
    name: 'schema 7 BSP course completes real ARM compilation, device-tree roundtrip and evidence report after environment repair',
    status: 'PASS',
  });
}
