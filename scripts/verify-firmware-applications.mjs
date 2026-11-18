// Isolated Ubuntu native Tauri checks. No connected device, flashing or publication.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, mkdir, readFile, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { createServer } from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { ensureDesktopIdle } from './lib/desktop-test-safety.mjs';
const root = path.resolve(import.meta.dirname, '..');
const out = process.env.LYNX_FWAPP_EVIDENCE;
assert.ok(out && path.isAbsolute(out));
await mkdir(out, { recursive: true });
const data = await mkdtemp(path.join(os.tmpdir(), 'lynx-fwapp-test-'));
const binary = path.join(root, 'src-tauri/target/debug/lynx-imager');
const result = {
  platform: 'Ubuntu native Tauri',
  binarySha256: createHash('sha256')
    .update(await readFile(binary))
    .digest('hex'),
  fixture: 'isolated saved boards and synthetic package; no hardware actions',
  checks: [],
  screenshots: [],
  pass: false,
};
let driver, session, origin;
async function port() {
  const s = createServer();
  await new Promise((r) => s.listen(0, '127.0.0.1', r));
  const p = s.address().port;
  await new Promise((r) => s.close(r));
  return p;
}
async function req(method, route, body) {
  const r = await fetch(origin + route, {
    method,
    headers: { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(40000),
  });
  const v = await r.json();
  assert.ok(r.ok && !(v.value?.error && v.value?.message), JSON.stringify(v));
  return v.value;
}
const exec = (script, args = []) =>
  req('POST', `/session/${session}/execute/sync`, { script, args });
async function wait(fn, label) {
  const end = Date.now() + 30000;
  while (Date.now() < end) {
    try {
      const v = await fn();
      if (v) return v;
    } catch {}
    await new Promise((r) => setTimeout(r, 150));
  }
  throw Error(label);
}
async function ipc(command, args = {}) {
  const v = await req('POST', `/session/${session}/execute/async`, {
    script:
      'const done=arguments[arguments.length-1];window.__TAURI_INTERNALS__.invoke(arguments[0],arguments[1]).then(value=>done({ok:true,value})).catch(error=>done({ok:false,error:String(error)}));',
    args: [command, args],
  });
  assert.ok(v.ok, v.error);
  return v.value;
}
async function click(selector) {
  const e = await wait(
    () => req('POST', `/session/${session}/element`, { using: 'css selector', value: selector }),
    selector
  );
  await req(
    'POST',
    `/session/${session}/element/${e['element-6066-11e4-a52e-4f735466cecf']}/click`,
    {}
  );
}
async function shot(name, w, h) {
  await req('POST', `/session/${session}/window/rect`, { width: w, height: h, x: 0, y: 0 });
  await exec(
    'window.dispatchEvent(new Event("resize"));let e=document.querySelector(".firmware-page,.fw-lifecycle");while(e){e.scrollTop=0;e.scrollLeft=0;e=e.parentElement;}'
  );
  await new Promise((r) => setTimeout(r, 400));
  const geometry = await exec(
    'return {width:innerWidth,height:innerHeight,scale:devicePixelRatio,scrollWidth:document.documentElement.scrollWidth,bodyWidth:document.body.scrollWidth,cards:[...document.querySelectorAll(".firmware-application-card")].map(e=>{const r=e.getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,height:r.height};})};'
  );
  assert.ok(geometry.scrollWidth <= geometry.width + 1, 'page horizontal overflow');
  const png = await req('GET', `/session/${session}/screenshot`);
  await writeFile(path.join(out, name + '.png'), Buffer.from(png, 'base64'));
  result.screenshots.push({ name, ...geometry });
  if (process.env.LYNX_FWAPP_VISUAL_SCALE === '1.25' && /158[56]$/.test(name)) {
    await req('POST', `/session/${session}/window/rect`, { width: w, height: h - 36, x: 0, y: 0 });
    await exec(
      'const s=document.createElement("style");s.id="native-visual-scale";s.textContent="#root{zoom:1.25;width:100vw;height:100vh} #root .layout{height:100vh;min-height:0} #root *{transition:none!important;animation:none!important}";document.head.append(s);'
    );
    await new Promise((r) => setTimeout(r, 400));
    const zoomGeometry = await exec(
      'return {width:innerWidth,height:innerHeight,scale:devicePixelRatio,zoom:getComputedStyle(document.getElementById("root")).zoom,anchors:[...document.querySelectorAll(".firmware-title,.firmware-app-grid,.firmware-app-detail-layout,.fw-two-columns,.fw-build-environment,.fw-build-workspace")].map(e=>({class:e.className,x:e.getBoundingClientRect().x,y:e.getBoundingClientRect().y,width:e.getBoundingClientRect().width,height:e.getBoundingClientRect().height}))};'
    );
    await writeFile(
      path.join(out, name + '-125.png'),
      Buffer.from(await req('GET', `/session/${session}/screenshot`), 'base64')
    );
    result.screenshots.push({
      name: name + '-125',
      ...zoomGeometry,
      testScale: 'Native WebKit CSS zoom 125%; client area excludes 36px reference OS titlebar',
    });
    await exec('document.getElementById("native-visual-scale").remove();');
    await req('POST', `/session/${session}/window/rect`, { width: w, height: h, x: 0, y: 0 });
  }
}
function check(name) {
  result.checks.push(name);
  console.log('PASS ' + name);
}
try {
  const p = await port();
  origin = `http://127.0.0.1:${p}`;
  driver = spawn(
    path.join(os.homedir(), '.cargo/bin/tauri-driver'),
    ['--port', String(p), '--native-port', String(await port())],
    {
      env: {
        ...process.env,
        GDK_BACKEND: 'x11',
        XDG_CONFIG_HOME: data + '/config',
        XDG_DATA_HOME: data + '/data',
        XDG_CACHE_HOME: data + '/cache',
        XDG_STATE_HOME: data + '/state',
        LYNX_TEST_ALLOW_PARALLEL_INSTANCE: '1',
        LYNX_TEST_DISABLE_HARDWARE_AUTOSTART: '1',
        LYNX_TEST_AUTH_KEYRING_NAMESPACE: path.basename(data),
        LYNX_TEST_SUPERVISOR_KEYRING_NAMESPACE: path.basename(data),
      },
      stdio: 'ignore',
    }
  );
  await wait(() => req('GET', '/status'), 'driver');
  const s = await req('POST', '/session', {
    capabilities: { alwaysMatch: { 'tauri:options': { application: binary } } },
  });
  session = s.sessionId || s.id;
  await req('POST', `/session/${session}/timeouts`, { script: 35000 });
  await wait(
    () =>
      exec(
        'return !!window.__TAURI_INTERNALS__ && !!document.querySelector("[data-testid=app-sidebar]");'
      ),
    'desktop'
  );
  assert.equal(await exec('return location.origin;'), 'tauri://localhost');
  result.safetyBefore = await ipc('hardware_safety_status');
  assert.equal(result.safetyBefore.busy, false);
  await click('[data-testid="sidebar-settings"]');
  await wait(
    () => exec('return !!document.querySelector("[data-testid=settings-panel]");'),
    'settings'
  );
  for (const [id, value] of [
    ['settings-theme-mode', 'light'],
    ['settings-theme-style', 'lynx-light'],
  ])
    await exec(
      'const e=document.querySelector(`[data-testid="${arguments[0]}"]`);e.value=arguments[1];e.dispatchEvent(new Event("change",{bubbles:true}));',
      [id, value]
    );
  await click('.settings-footer .settings-btn-primary');
  await wait(
    () => exec('return !!document.querySelector("[data-testid=settings-save-success]");'),
    'theme saved'
  );
  await click('[data-testid="settings-close"]');
  await wait(
    () =>
      exec(
        'return !document.querySelector("[data-testid=settings-overlay]") && document.documentElement.dataset.theme==="lynx-light";'
      ),
    'light theme'
  );
  const base = {
    id: null,
    boardUid: '',
    name: 'F101 UI 验收 · 未连接',
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
  };
  const deviceId = await ipc('workbench_save_device', { device: base });
  console.log(
    'CATALOG_DIAGNOSTIC',
    JSON.stringify(await ipc('workbench_firmware_applications', { deviceId }))
  );
  await req('POST', `/session/${session}/refresh`, {});
  await wait(
    () => exec('return document.body.innerText.includes("F101 UI 验收");'),
    'saved fixture'
  );
  await click('[data-testid="sidebar-tool-workbench"]');
  await click('[data-testid="linux-tab-flash"]');
  await click('#firmware-apps-tab');
  await wait(
    () => exec('return document.querySelectorAll(".firmware-application-card").length===6;'),
    'six application cards'
  );
  check('native application tab shows six documentation entries');
  const catalog = await ipc('workbench_firmware_applications', { deviceId });
  assert.equal(catalog.hardwareProfile, 'unverified');
  assert.equal(catalog.publicationStatus, 'github_configured');
  check('alias does not grant compatibility or published status');
  assert.equal(catalog.repository, 'dshanpi/yuzukihdneko-zephyr');
  await click('.firmware-catalog-options > summary');
  assert.ok(await exec('return document.body.innerText.includes("尚未读取发布目录");'));
  if (process.env.LYNX_FWAPP_VERIFY_GITHUB_SOURCE === '1') {
    await click('[data-testid="firmware-release-refresh"]');
    await wait(
      () =>
        exec('return !document.querySelector("[data-testid=firmware-release-refresh]").disabled;'),
      'release query finished'
    );
    const releaseText = await exec(
      'return document.querySelector("[data-testid=firmware-release-source]").innerText;'
    );
    assert.ok(
      !(await exec(
        'return !!document.querySelector("[data-testid=firmware-release-source] [role=alert]");'
      )),
      releaseText
    );
    assert.ok(!releaseText.includes('尚未读取'));
    result.liveSource = { repository: catalog.repository, displayed: releaseText };
    check('real GitHub source query has explicit result without hardware or installation');
  }

  await click('.firmware-catalog-options > summary');
  await shot('P01-applications-1586', 1586, 992);
  const search = await req('POST', `/session/${session}/element`, {
    using: 'css selector',
    value: 'input[aria-label="搜索应用固件"]',
  });
  const searchId = search['element-6066-11e4-a52e-4f735466cecf'];
  await req('POST', `/session/${session}/element/${searchId}/value`, {
    text: 'NoSuchApplication',
    value: [...'NoSuchApplication'],
  });
  await wait(
    () => exec('return document.body.innerText.includes("没有匹配的应用");'),
    'empty search'
  );
  check('no-match search state');
  await req('POST', `/session/${session}/element/${searchId}/value`, {
    text: '\uE009a\uE000\uE003',
    value: ['\uE009', 'a', '\uE000', '\uE003'],
  });
  await wait(
    () => exec('return document.querySelectorAll(".firmware-application-card").length===6;'),
    'clear search'
  );
  for (const name of [
    'NekoBox',
    'USB 副屏',
    'MicroPython',
    'LVGL 示例',
    'Hello World',
    'Shell 示例',
  ]) {
    await click(`button[aria-label="查看 ${name} 版本"]`);
    await wait(
      () => exec('return !!document.querySelector(".firmware-app-install-card");'),
      'detail'
    );
    assert.equal(
      await exec('return document.querySelector(".firmware-app-download").disabled;'),
      true
    );
    assert.ok(await exec('return document.body.innerText.includes("未核对");'));
    if (name === 'NekoBox') {
      await shot('P02-nekobox-1585', 1585, 992);
      for (const [w, h] of [
        [1280, 800],
        [1920, 1080],
      ])
        await shot(`P02-nekobox-${w}`, w, h);
      await req('POST', `/session/${session}/window/rect`, { width: 1586, height: 992 });
    } else assert.ok(await exec('return document.body.innerText.includes("自动编排尚未接入");'));
    await click('.firmware-app-breadcrumb button');
  }
  check(
    'all six details preserve unknown compatibility, disabled installation and route-specific guidance'
  );
  for (const [w, h] of [
    [1280, 800],
    [1920, 1080],
  ])
    await shot(`P01-applications-${w}`, w, h);
  await click('#firmware-offline-tab');
  assert.ok(await exec('return !!document.querySelector("#firmware-offline");'));
  await click('#firmware-ai-tab');
  assert.ok(await exec('return !!document.querySelector("#firmware-ai");'));
  await click('#firmware-apps-tab');
  check('offline and AI tabs remain accessible');
  const fixture = path.join(data, 'candidate');
  await mkdir(fixture);
  const image = Buffer.alloc(600 * 1024, 90),
    loader = Buffer.alloc(128, 107);
  await writeFile(path.join(fixture, 'nekobox-full.img'), image);
  await writeFile(path.join(fixture, 'fes.img'), loader);
  const file = (name, bytes) => ({
    fileName: name,
    sizeBytes: bytes.length,
    sha256: createHash('sha256').update(bytes).digest('hex'),
  });
  const manifest = {
    schemaVersion: 1,
    applicationId: 'nekobox',
    version: 'ui-fixture-not-firmware',
    boardModel: 'yuzukineko',
    chip: 'f101',
    route: 'xip_spinor_full',
    flashCapacityBytes: 16 * 1024 * 1024,
    layout: { bootOffset: 0, parametersOffset: 0x20000, applicationOffset: 0x80000 },
    image: file('nekobox-full.img', image),
    loader: file('fes.img', loader),
    loaderSource: 'Synthetic test fixture; not an executable Loader or distributable firmware',
    sourceCommit: 'a'.repeat(40),
  };
  const manifestPath = path.join(fixture, 'candidate.json');
  await writeFile(manifestPath, JSON.stringify(manifest));
  const candidate = await ipc('workbench_inspect_firmware_candidate', { deviceId, manifestPath });
  assert.equal(candidate.integrity, 'verified');
  assert.equal(candidate.installAuthorized, false);
  assert.equal(candidate.hardwareCompatibility, 'unverified');
  check('real Rust candidate hashing does not grant install permission');
  await writeFile(path.join(fixture, 'nekobox-full.img'), Buffer.alloc(image.length, 91));
  await assert.rejects(
    () => ipc('workbench_inspect_firmware_candidate', { deviceId, manifestPath }),
    /HASH_MISMATCH/
  );
  check('actual IPC rejects substituted same-size image');
  await assert.rejects(
    () => ipc('workbench_firmware_applications', { deviceId: 999999 }),
    /BOARD_NOT_SAVED/
  );
  check('unknown board rejected by backend');
  if (process.env.LYNX_FWAPP_REAL_CANDIDATE) {
    const actual = await ipc('workbench_inspect_firmware_candidate', {
      deviceId,
      manifestPath: process.env.LYNX_FWAPP_REAL_CANDIDATE,
    });
    assert.equal(actual.integrity, 'verified');
    assert.equal(actual.installAuthorized, false);
    for (const [kind, filePath] of [
      ['image', actual.imagePath],
      ['loader', actual.loaderPath],
    ])
      assert.equal(
        createHash('sha256')
          .update(await readFile(filePath))
          .digest('hex'),
        actual.manifest[kind].sha256
      );
    result.realCandidate = {
      image: actual.manifest.image,
      loader: actual.manifest.loader,
      hardwareCompatibility: actual.hardwareCompatibility,
      installAuthorized: actual.installAuthorized,
    };
    check('existing NekoBox image and F101 Loader verified independently without hardware access');
  }

  // New lifecycle paths exercise the real backend in this disposable app data directory.
  await writeFile(path.join(fixture, 'nekobox-full.img'), image);
  const imported = await ipc('workbench_firmware_import', { deviceId, manifestPath });
  assert.equal(imported.manifest.image.sha256, manifest.image.sha256);
  await assert.rejects(
    () =>
      ipc('workbench_firmware_prepare', {
        deviceId,
        candidateId: imported.id,
        postAction: 'reboot',
      }),
    /PROFILE_REQUIRED/
  );
  await assert.rejects(
    () =>
      ipc('workbench_firmware_start', { deviceId, confirmationId: 'missing', confirmed: false }),
    /CONFIRMATION_REQUIRED/
  );
  await assert.rejects(
    () =>
      ipc('workbench_firmware_publish', {
        deviceId,
        request: {
          requestId: 'native-negative',
          candidateId: imported.id,
          version: '0.1.0-test.1',
          channel: 'test',
          notes: 'Isolated synthetic fixture. Never publish.',
          distributionLicense: 'not-distributable',
          rightsConfirmed: false,
        },
        confirmed: true,
      }),
    /PUBLISH_FIELDS_REQUIRED/
  );
  const profile = {
    model: 'yuzukineko',
    revision: 'SOFTWARE FIXTURE - NOT PHYSICAL IDENTITY',
    chip: 'f101',
    capacityBytes: 16777216,
    display: '1024x768',
    touch: 'gt967',
    operator: 'Isolated software test',
    evidence: 'Synthetic test profile only; no hardware identity claim',
  };
  await assert.rejects(
    () => ipc('workbench_firmware_save_profile', { deviceId, profile, confirmed: false }),
    /CONFIRMATION_REQUIRED/
  );
  await ipc('workbench_firmware_save_profile', { deviceId, profile, confirmed: true });
  await assert.rejects(
    () =>
      ipc('workbench_firmware_prepare', {
        deviceId,
        candidateId: imported.id,
        postAction: 'reboot',
      }),
    /FLASH_TARGET_UNBOUND/
  );
  const otherId = await ipc('workbench_save_device', {
    device: { ...base, name: 'Other isolated board' },
  });
  await assert.rejects(
    () => ipc('workbench_firmware_report', { deviceId: otherId, candidateId: imported.id }),
    /SCOPE_MISMATCH/
  );
  await assert.rejects(
    () =>
      ipc('workbench_firmware_acceptance', {
        deviceId,
        candidateId: imported.id,
        taskId: 'fake-success',
        operator: 'test',
        items: [{ item: 'boot', result: 'pass', evidence: 'not actually observed' }],
      }),
    /EVIDENCE_SCOPE_MISMATCH/
  );
  check(
    'profile, binding, explicit confirmation, cross-board evidence and publish-rights guards reject without hardware'
  );
  await exec(
    'window.dispatchEvent(new CustomEvent("lynx:firmware-candidate",{detail:{deviceId:arguments[0],candidateId:arguments[1]}}));',
    [deviceId, imported.id]
  );
  await wait(
    () => exec('return !!document.querySelector("[data-testid=firmware-confirm-page]");'),
    'P03'
  );
  await shot('P03-confirmation-1586', 1586, 992);
  await click('.fw-task-options > summary');
  await exec(
    '[...document.querySelectorAll(".fw-page-tabs button")].find(b=>b.textContent.includes("验收与记录")).click();'
  );
  await wait(
    () => exec('return !!document.querySelector("[data-testid=firmware-verify-page]");'),
    'P04'
  );
  assert.ok(
    await exec(
      'return document.body.innerText.includes("等待烧录与启动验收") && document.body.innerText.includes("未验证");'
    )
  );
  await click('.fw-task-options > summary');
  await shot('P04-unverified-1586', 1586, 992);
  await click('.fw-task-options > summary');
  await exec(
    '[...document.querySelectorAll(".fw-page-tabs button")].find(b=>b.textContent==="版本发布").click();'
  );
  await wait(
    () => exec('return !!document.querySelector("[data-testid=firmware-publication-page]");'),
    'P06'
  );
  assert.equal(
    await exec(
      'return [...document.querySelectorAll("button")].find(b=>b.textContent==="发布到应用目录").disabled;'
    ),
    true
  );
  await click('.fw-task-options > summary');
  assert.deepEqual(
    await exec(
      'return [...document.querySelectorAll(".sidebar-item.active")].map(e=>e.textContent.trim());'
    ),
    ['运行中心']
  );
  await shot('P06-publish-blocked-1586', 1586, 992);
  for (const [view, testid] of [
    ['烧录确认', 'firmware-confirm-page'],
    ['验收与记录', 'firmware-verify-page'],
    ['版本发布', 'firmware-publication-page'],
  ]) {
    await click('.fw-task-options > summary');
    await exec(
      '[...document.querySelectorAll(".fw-page-tabs button")].find(b=>b.textContent===arguments[0]).click();',
      [view]
    );
    await wait(
      () => exec('return !!document.querySelector(`[data-testid="${arguments[0]}"]`);', [testid]),
      testid
    );
    await click('.fw-task-options > summary');
    for (const width of [1280, 1920]) await shot(`${testid}-${width}`, width, 1080);
  }
  await req('POST', `/session/${session}/window/rect`, { width: 1586, height: 992 });

  const report = JSON.parse(
    await ipc('workbench_firmware_report', { deviceId, candidateId: imported.id })
  );
  assert.deepEqual(report.tasks, []);
  assert.deepEqual(report.acceptance, []);
  await req('POST', `/session/${session}/refresh`, {});
  const restored = await ipc('workbench_firmware_workspace', { deviceId });
  assert.ok(restored.candidates.some((c) => c.id === imported.id));
  check(
    'candidate, profile and unverified evidence survive UI reload; report does not invent passed hardware'
  );
  await click('[data-testid="sidebar-tool-workbench"]');
  await click('[data-testid="linux-tab-sdk"]');
  await exec(
    '[...document.querySelectorAll(".fw-page-tabs button")].find(b=>b.textContent==="构建任务").click();'
  );
  await wait(
    () => exec('return !!document.querySelector("[data-testid=firmware-build-page]");'),
    'P05'
  );
  assert.equal(
    await exec(
      'return [...document.querySelectorAll("button")].find(b=>b.textContent==="开始新构建").disabled;'
    ),
    true
  );
  await shot('P05-build-unconfigured-1586', 1586, 992);
  check('build page requires an existing saved SDK channel; disabled actions do not launch a task');
  if (process.env.LYNX_FWAPP_SDK_PASSWORD) {
    const fingerprint = process.env.LYNX_FWAPP_SDK_FINGERPRINT;
    assert.ok(fingerprint);
    const credentials = { password: process.env.LYNX_FWAPP_SDK_PASSWORD };
    const host = {
      id: null,
      name: 'F101 software build acceptance',
      address: '192.168.1.17',
      port: 22,
      username: 'ubuntu',
      authType: 'password',
      privateKeyPath: '',
      hostFingerprint: fingerprint,
      defaultWorkDir: '/home/ubuntu/F101-zephyr',
      aiProvider: 'codex',
      aiExecutable: 'codex',
      aiArgsJson: '[]',
    };
    const trust = await ipc('workbench_test_ssh', { host, credentials, allowUntrusted: false });
    const hostId = await ipc('workbench_save_host', {
      host,
      connectionTestSessionId: trust.auditSessionId,
    });
    const projectId = await ipc('workbench_save_project', {
      project: {
        id: null,
        name: 'F101 native recipe validation',
        hostId,
        deviceId,
        sdkDir: '/home/ubuntu/F101-zephyr',
        buildCommand: 'true',
        firmwarePath: '',
        localDownloadDir: path.join(data, 'downloads'),
        verifySha256: true,
        localGatewayPort: await port(),
        remoteForwardPort: await port(),
        mcpServerName: 'lynx-f101-native-recipe',
        aiProvider: 'codex',
      },
    });
    const requestId = 'native-software-' + Date.now();
    const build = await ipc('workbench_firmware_build_start', {
      deviceId,
      projectId,
      requestId,
      credentials,
    });
    const repeated = await ipc('workbench_firmware_build_start', {
      deviceId,
      projectId,
      requestId,
      credentials,
    });
    assert.equal(repeated.id, build.id);
    await assert.rejects(
      () =>
        ipc('workbench_firmware_build_start', {
          deviceId,
          projectId,
          requestId: requestId + '-duplicate',
          credentials,
        }),
      /ALREADY_RUNNING/
    );
    const deadline = Date.now() + 30 * 60 * 1000;
    let finished;
    while (Date.now() < deadline) {
      try {
        finished = await ipc('workbench_firmware_build_status', {
          deviceId,
          buildId: build.id,
          credentials,
        });
      } catch (e) {
        if (!String(e).includes('RECEIPT_PENDING')) throw e;
      }
      if (finished?.receipt?.status === 'success' || finished?.receipt?.status === 'failed') break;
      await new Promise((r) => setTimeout(r, 5000));
    }
    assert.equal(finished?.receipt?.status, 'success', JSON.stringify(finished));
    const built = await ipc('workbench_firmware_build_candidate', {
      deviceId,
      buildId: build.id,
      loaderPath: process.env.LYNX_FWAPP_LOADER,
      credentials,
    });
    assert.equal(built.buildId, build.id);
    assert.equal(
      built.manifest.image.sha256,
      finished.receipt.artifacts.find((a) => a.fileName === 'nekobox-full.img').sha256
    );
    assert.equal(
      createHash('sha256')
        .update(await readFile(built.imagePath))
        .digest('hex'),
      built.manifest.image.sha256
    );
    assert.deepEqual(built.taskIds, []);
    await writeFile(path.join(out, 'native-build-receipt.json'), JSON.stringify(finished, null, 2));
    check(
      'real SSH recipe, task deduplication, locked inputs, artifact transfer/hash and send-to-test candidate; no flash'
    );
  }
  result.safetyAfter = await ensureDesktopIdle(ipc);
  assert.equal(result.safetyAfter.hardware.busy, false);
  const tasks = await ipc('workbench_list_tasks', { limit: 100 });
  assert.ok(!tasks.some((t) => t.kind === 'flash' || t.kind === 'power'));
  check('software integration issued no flashing, erase, reboot or power task');
  result.pass = true;
} catch (error) {
  result.error = String(error.stack || error);
  process.exitCode = 1;
  if (session) {
    try {
      await writeFile(path.join(out, 'failure.txt'), await exec('return document.body.innerText;'));
      await writeFile(
        path.join(out, 'failure.png'),
        Buffer.from(await req('GET', `/session/${session}/screenshot`), 'base64')
      );
    } catch {}
  }
} finally {
  if (session) {
    try {
      await ensureDesktopIdle(ipc);
      await req('DELETE', `/session/${session}`);
      driver.kill('SIGTERM');
    } catch (error) {
      result.cleanupError = String(error);
      process.exitCode = 1;
    }
  } else if (driver) driver.kill('SIGTERM');
  await writeFile(path.join(out, 'result.json'), JSON.stringify(result, null, 2) + '\n');
  console.log(
    JSON.stringify({ pass: result.pass, checks: result.checks.length, error: result.error })
  );
}
