import { ensureDesktopIdle } from './lib/desktop-test-safety.mjs';
// Isolated real Tauri + local SSH acceptance. No physical board or production service.
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  mkdtemp,
  mkdir,
  readFile,
  writeFile,
  readdir,
  realpath,
  chmod,
  stat,
} from 'node:fs/promises';
import YAML from 'yaml';
import { createServer } from 'node:net';
import os from 'node:os';
import path from 'node:path';

const root = process.cwd();
const out = process.env.LYNX_DEVICE_SETTINGS_OUTPUT;
assert.ok(out && path.isAbsolute(out));
await mkdir(out, { recursive: true });
const data = await mkdtemp(path.join(os.tmpdir(), 'lynx-device-settings-'));
const binary = await realpath(
  process.env.LYNX_DEVICE_SETTINGS_BINARY || path.join(root, 'src-tauri/target/debug/lynx-imager')
);
const result = {
  sourceHead: execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim(),
  applicationSha256: createHash('sha256')
    .update(await readFile(binary))
    .digest('hex'),
  data,
  synthetic: true,
  physicalHardwareActions: false,
  productionMutationsRequested: false,
  mockedIpc: false,
  checks: [],
};
const delay = (ms) => new Promise((r) => setTimeout(r, ms));
const run = (command, args) => execFileSync(command, args, { encoding: 'utf8' });
const env = {
  ...process.env,
  GDK_BACKEND: 'x11',
  XDG_CONFIG_HOME: data + '/config',
  XDG_DATA_HOME: data + '/data',
  XDG_CACHE_HOME: data + '/cache',
  XDG_STATE_HOME: data + '/state',
  LYNX_TEST_ALLOW_PARALLEL_INSTANCE: '1',
  LYNX_TEST_AUTH_KEYRING_NAMESPACE: path.basename(data),
};
let driver, sshd, session, origin;
async function freePort() {
  const server = createServer();
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  const port = server.address().port;
  await new Promise((resolve) => server.close(resolve));
  return port;
}
async function req(method, route, body) {
  const response = await fetch(origin + route, {
    method,
    headers: { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(40000),
  });
  const v = await response.json();
  if (!response.ok || v.value?.error) throw Error(JSON.stringify(v));
  return v.value;
}
const execute = (script, args = []) =>
  req('POST', `/session/${session}/execute/sync`, { script, args });
async function ipc(command, args = {}) {
  const v = await req('POST', `/session/${session}/execute/async`, {
    script:
      'const done=arguments[arguments.length-1];window.__TAURI_INTERNALS__.invoke(arguments[0],arguments[1]).then(value=>done({ok:true,value})).catch(error=>done({ok:false,error:String(error)}));',
    args: [command, args],
  });
  assert.ok(v.ok, v.error);
  return v.value;
}
async function wait(fn, description, timeout = 30000) {
  const end = Date.now() + timeout;
  let last;
  while (Date.now() < end) {
    try {
      const v = await fn();
      if (v) return v;
    } catch (e) {
      last = e;
    }
    await delay(150);
  }
  throw Error(description + ': ' + (last?.message || 'timeout'));
}
function check(name, actual, expected = true) {
  assert.deepEqual(actual, expected, name);
  result.checks.push({ name, pass: true });
}
async function click(selector) {
  const element = await req('POST', `/session/${session}/element`, {
    using: 'css selector',
    value: selector,
  });
  await req(
    'POST',
    `/session/${session}/element/${element['element-6066-11e4-a52e-4f735466cecf']}/click`,
    {}
  );
}
async function input(selector, value) {
  await execute(
    `const e=document.querySelector(arguments[0]);if(!e)throw Error('input missing');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(e,arguments[1]);e.dispatchEvent(new Event('input',{bubbles:true}));`,
    [selector, value]
  );
  await delay(100);
}
async function screenshot(name) {
  await writeFile(
    path.join(out, name + '.png'),
    Buffer.from(await req('GET', `/session/${session}/screenshot`), 'base64')
  );
}
function clipboardText() {
  return execFileSync(
    '/usr/bin/python3',
    [
      '-c',
      "import gi;gi.require_version('Gtk','3.0');from gi.repository import Gtk,Gdk;print(Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD).wait_for_text() or '',end='')",
    ],
    { encoding: 'utf8', timeout: 5000, env: { ...process.env, GDK_BACKEND: 'x11' } }
  );
}
async function verifyAddressCopy(kind, endpoint) {
  const selected = await execute(
    `
    const element=document.querySelector(arguments[0]);
    const range=document.createRange();range.selectNodeContents(element);
    const selection=window.getSelection();selection.removeAllRanges();selection.addRange(range);
    return {text:selection.toString().trim(),userSelect:getComputedStyle(element).userSelect || getComputedStyle(element).webkitUserSelect};
  `,
    [`[data-testid=linux-${kind}-mcp-address]`]
  );
  check(`${kind} MCP URL is selectable as exact text`, selected, {
    text: endpoint,
    userSelect: 'text',
  });
  await click(`[data-testid=linux-copy-${kind}-mcp-address]`);
  await wait(
    () => clipboardText() === endpoint,
    `${kind} exact URL reaches actual private-display clipboard`
  );
  check(`${kind} copy button writes only its own URL`, clipboardText(), endpoint);
}
async function verifyMcpInitialize(endpoint) {
  const response = await fetch(endpoint, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Accept: 'application/json, text/event-stream' },
    body: JSON.stringify({
      jsonrpc: '2.0',
      id: 1,
      method: 'initialize',
      params: {
        protocolVersion: '2024-11-05',
        capabilities: {},
        clientInfo: { name: 'lynx-device-settings-acceptance', version: '1.0.0' },
      },
    }),
    signal: AbortSignal.timeout(8000),
  });
  assert.equal(response.status, 200);
  const reader = response.body.getReader(),
    decoder = new TextDecoder();
  let text = '',
    message;
  try {
    while (!message) {
      const chunk = await reader.read();
      if (chunk.done) break;
      text += decoder.decode(chunk.value, { stream: true });
      assert.ok(text.length < 65536);
      for (const candidate of [
        text.trim(),
        ...text
          .split('\n')
          .filter((line) => line.startsWith('data:'))
          .map((line) => line.slice(5).trim()),
      ]) {
        try {
          const value = JSON.parse(candidate);
          if (value.id === 1) message = value;
        } catch {
          /* wait for complete event */
        }
      }
    }
  } finally {
    await reader.cancel();
  }
  assert.ok(
    message?.result?.capabilities?.tools,
    'actual MCP initialization must expose tools capability'
  );
  const id = response.headers.get('mcp-session-id');
  if (id) {
    assert.equal((await ipc('hardware_safety_status')).busy, false);
    await fetch(endpoint, {
      method: 'DELETE',
      headers: { 'mcp-session-id': id },
      signal: AbortSignal.timeout(3000),
    });
  }
}
async function settings() {
  await click('[data-testid=linux-header-device-settings]');
  await wait(
    () => execute('return !!document.querySelector("[data-testid=linux-settings-dialog]");'),
    'settings open'
  );
}
async function closeOwnedApp() {
  if (!session) return;
  await wait(async () => {
    result.desktopIdleBeforeClose = await ensureDesktopIdle(ipc);
    result.safetyBeforeClose = result.desktopIdleBeforeClose.hardware;
    return result.safetyBeforeClose.busy === false;
  }, 'hardware idle before close');
  const owned = [];
  for (const pid of await readdir('/proc')) {
    if (!/^\d+$/.test(pid)) continue;
    try {
      if (
        (await realpath(`/proc/${pid}/exe`)) === binary &&
        (await readFile(`/proc/${pid}/environ`, 'utf8'))
          .split('\0')
          .includes('XDG_DATA_HOME=' + env.XDG_DATA_HOME)
      )
        owned.push(Number(pid));
    } catch {}
  }
  assert.equal(owned.length, 1);
  process.kill(owned[0], 'SIGTERM');
  await wait(async () => !(await readdir('/proc')).includes(String(owned[0])), 'owned app exit');
  await req('DELETE', `/session/${session}`).catch(() => {});
  session = null;
  driver.kill('SIGTERM');
  if (sshd) sshd.kill('SIGTERM');
}
try {
  for (const key of ['host', 'client'])
    run('ssh-keygen', ['-q', '-t', 'ed25519', '-N', '', '-f', data + '/' + key]);
  await writeFile(data + '/authorized_keys', await readFile(data + '/client.pub'));
  const sshPort = await freePort();
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
    stdio: ['ignore', 'ignore', 'ignore'],
  });
  const port = await freePort(),
    nativePort = await freePort();
  origin = `http://127.0.0.1:${port}`;
  driver = spawn(
    path.join(os.homedir(), '.cargo/bin/tauri-driver'),
    ['--port', String(port), '--native-port', String(nativePort)],
    { cwd: root, env, stdio: ['ignore', 'ignore', 'ignore'] }
  );
  await wait(() => req('GET', '/status'), 'driver ready');
  const created = await req('POST', '/session', {
    capabilities: { alwaysMatch: { 'tauri:options': { application: binary } } },
  });
  session = created.sessionId || created.id;
  await writeFile(
    out + '/control.json',
    JSON.stringify({ origin, session, data, driverPid: driver.pid, sshdPid: sshd.pid })
  );
  await wait(
    () => execute('return !!document.querySelector("[data-testid=app-sidebar]");'),
    'desktop ready'
  );
  check('embedded desktop frontend', await execute('return location.origin;'), 'tauri://localhost');
  check('isolated auth namespace has no real signed-in account', await ipc('auth_get_user'), null);
  await req('POST', `/session/${session}/window/rect`, { width: 1440, height: 1000 });
  await wait(async () => !(await ipc('hardware_safety_status')).busy, 'startup idle');
  if (process.env.LYNX_DEVICE_SETTINGS_THEME) {
    const theme = process.env.LYNX_DEVICE_SETTINGS_THEME;
    assert.ok(['light', 'dark'].includes(theme));
    result.initialTheme = theme;
    await click('[data-testid=sidebar-settings]');
    await wait(
      () => execute('return !!document.querySelector("[data-testid=settings-panel]");'),
      'theme settings'
    );
    for (const [id, value] of [
      ['settings-theme-mode', theme],
      ['settings-theme-style', `lynx-${theme}`],
    ]) {
      await execute(
        `const e=document.querySelector('[data-testid="'+arguments[0]+'"]');if(!e)throw Error('Theme field missing');Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype,'value').set.call(e,arguments[1]);e.dispatchEvent(new Event('change',{bubbles:true}));`,
        [id, value]
      );
    }
    await click('.settings-footer .settings-btn-primary');
    await wait(
      () => execute('return !!document.querySelector("[data-testid=settings-save-success]");'),
      'theme persisted'
    );
    await click('[data-testid=settings-close]');
    await wait(
      () =>
        execute(
          'return !document.querySelector("[data-testid=settings-overlay]")&&document.documentElement.dataset.theme==="lynx-"+arguments[0];',
          [theme]
        ),
      `${theme} theme active`
    );
  }
  const model = await readFile('src/Components/Workbench/linuxWorkbenchModel.ts', 'utf8');
  const empty = (name) =>
    new Function(
      'return (' +
        model.match(new RegExp('export const ' + name + '[^=]*= (\\{[\\s\\S]*?\\n\\});'))[1] +
        ')'
    )();
  const device = {
    ...empty('EMPTY_LINUX_DEVICE'),
    name: 'Local Acceptance Board',
    powerProfileJson: '',
  };
  const deviceId = await ipc('workbench_save_device', { device });
  const otherId = await ipc('workbench_save_device', {
    device: { ...device, name: 'Other Isolated Board' },
  });
  const publicKey = (await readFile(data + '/host.pub', 'utf8')).split(/\s+/)[1];
  const host = {
    ...empty('EMPTY_WORKBENCH_HOST'),
    name: 'Loopback SDK',
    address: '127.0.0.1',
    port: sshPort,
    username: os.userInfo().username,
    authType: 'private_key',
    privateKeyPath: data + '/client',
    hostFingerprint:
      'SHA256:' +
      createHash('sha256')
        .update(Buffer.from(publicKey, 'base64'))
        .digest('base64')
        .replace(/=+$/, ''),
    defaultWorkDir: data,
  };
  const trust = await ipc('workbench_test_ssh', { host, credentials: {}, allowUntrusted: false });
  check('actual SSH host trusted', trust.trusted);
  const hostId = await ipc('workbench_save_host', {
    host,
    connectionTestSessionId: trust.auditSessionId,
  });
  const localPort = await freePort(),
    remotePort = await freePort();
  assert.notEqual(localPort, 8765);
  assert.notEqual(localPort, remotePort);
  const project = {
    ...empty('EMPTY_WORKBENCH_PROJECT'),
    name: 'Local Acceptance SDK',
    hostId,
    deviceId,
    sdkDir: data,
    localGatewayPort: localPort,
    remoteForwardPort: remotePort,
    mcpServerName: 'lynx-local-acceptance-sdk',
  };
  const projectId = await ipc('workbench_save_project', { project });
  await ipc('gateway_start', { projectId, deviceId, port: localPort });
  result.safetyBeforeLocalOnly = await ipc('hardware_safety_status');
  assert.equal(result.safetyBeforeLocalOnly.busy, false);
  const existingTunnel = (await ipc('workbench_list_tunnel_statuses')).find(
    (item) => item.projectId === projectId && item.state !== 'stopped'
  );
  if (existingTunnel) await ipc('workbench_stop_tunnel', { projectId });
  const local = await ipc('gateway_channel_status', { projectId, deviceId });
  check('local HTTP verification', local.verified);
  await verifyMcpInitialize(local.endpoint);
  check('local address accepts an actual MCP initialize request', true);
  check('runtime port is actual non-default port', local.port, localPort);
  check(
    'other board has no usable listener',
    (await ipc('gateway_channel_status', { projectId, deviceId: otherId })).endpoint,
    null
  );
  check(
    'local readiness does not claim a remote tunnel',
    (await ipc('workbench_list_tunnel_statuses')).some(
      (t) => t.projectId === projectId && t.state === 'connected'
    ),
    false
  );
  await execute(
    'window.localStorage.setItem("lynx.workbench.selectedDeviceId",String(arguments[0]));location.reload();',
    [deviceId]
  );
  await wait(
    () => execute('return !!document.querySelector("[data-testid=sidebar-tool-workbench]");'),
    'sidebar after reload'
  );
  await click('[data-testid=sidebar-tool-workbench]');
  await wait(
    () => execute('return !!document.querySelector("[data-testid=linux-header-device-settings]");'),
    'saved board view'
  );
  const connectedTunnel = await wait(
    async () =>
      (await ipc('workbench_list_tunnel_statuses')).find(
        (t) => t.projectId === projectId && t.state === 'connected'
      ),
    'actual reverse SSH verification'
  );
  const activeRemotePort = connectedTunnel.remotePort;
  assert.ok(activeRemotePort > 0 && activeRemotePort !== localPort);
  await settings();
  await wait(
    () =>
      execute(
        `return [...document.querySelectorAll('.channel-map code')].some(e=>e.textContent===arguments[0]);`,
        [local.endpoint]
      ),
    'local address shown'
  );
  await wait(
    () =>
      execute(
        `return [...document.querySelectorAll('.channel-map code')].some(e=>e.textContent===arguments[0]);`,
        [`http://127.0.0.1:${activeRemotePort}/mcp`]
      ),
    'current remote address reaches UI'
  );
  const texts = await execute('return document.querySelector(".channel-map").innerText;');
  check(
    'Codex command uses the actual runtime endpoint',
    texts.includes(`codex -c 'mcp_servers={"lynx-local-acceptance-sdk"={url="${local.endpoint}"}}'`)
  );
  check(
    'separate local and remote status labels',
    texts.includes('本机连接') && texts.includes('远端 SDK 连接')
  );
  check(
    'remote address differs and is shown',
    texts.includes(`http://127.0.0.1:${activeRemotePort}/mcp`)
  );
  await verifyMcpInitialize(`http://127.0.0.1:${activeRemotePort}/mcp`);
  check('remote address accepts MCP through the actual SSH forward', true);
  await verifyAddressCopy('local', local.endpoint);
  await verifyAddressCopy('remote', `http://127.0.0.1:${activeRemotePort}/mcp`);
  await req('POST', `/session/${session}/window/rect`, { width: 1920, height: 1300 });
  for (const client of ['codex', 'zcode', 'claude_code', 'deepseek_harness', 'custom']) {
    await click(`[data-testid=linux-mcp-client-${client}]`);
    for (const side of ['local', 'remote']) {
      const endpoint =
        side === 'local' ? local.endpoint : `http://127.0.0.1:${activeRemotePort}/mcp`;
      const value = await execute('return document.querySelector(arguments[0]).textContent;', [
        `[data-testid=linux-${side}-mcp-command]`,
      ]);
      assert.ok(value.includes(endpoint));
      if (client === 'zcode' || client === 'custom') {
        assert.deepEqual(JSON.parse(value), {
          mcpServers: { 'lynx-local-acceptance-sdk': { type: 'http', url: endpoint } },
        });
      } else {
        assert.ok(
          value.startsWith(
            client === 'codex'
              ? 'codex '
              : client === 'claude_code'
                ? 'claude --strict-mcp-config '
                : '(mcp_tmp='
          )
        );
      }
      if (client === 'deepseek_harness') {
        const fakeBin = path.join(data, 'fake-dsh');
        await mkdir(fakeBin, { recursive: true });
        const fakeDsh = path.join(fakeBin, 'dsh');
        await writeFile(
          fakeDsh,
          '#!/bin/sh\n[ "$1" = web ] && [ "$2" = --patch ] || exit 90\ncp "$3" "$LYNX_PATCH_CAPTURE"\nprintf %s "$3" > "$LYNX_PATH_CAPTURE"\n'
        );
        await chmod(fakeDsh, 0o700);
        const captured = path.join(data, `patch-${side}.yml`);
        const temporaryPath = path.join(data, `patch-path-${side}.txt`);
        const parentTrap = path.join(data, `parent-trap-${side}.txt`);
        execFileSync('/bin/sh', ['-c', `trap 'touch "$LYNX_PARENT_TRAP_CAPTURE"' EXIT\n${value}`], {
          env: {
            ...process.env,
            PATH: `${fakeBin}:${process.env.PATH}`,
            LYNX_PATCH_CAPTURE: captured,
            LYNX_PATH_CAPTURE: temporaryPath,
            LYNX_PARENT_TRAP_CAPTURE: parentTrap,
          },
        });
        const patch = YAML.parse(await readFile(captured, 'utf8'));
        assert.equal(patch[0].name, '@deepseek-ai/dsh-mcp-client');
        assert.equal(patch[0].config.url, endpoint);
        assert.equal(patch[0].config.serverName, 'lynx-local-acceptance-sdk');
        await stat(parentTrap);
        const readFilePath = await readFile(temporaryPath, 'utf8');
        await assert.rejects(() => stat(readFilePath), { code: 'ENOENT' });
        check(
          `${side} POSIX generated patch executes with fixture CLI, exact YAML, preserved parent trap and deleted temporary file`,
          true
        );
      }
      await click(`[data-testid=linux-copy-${side}-mcp-command]`);
      await wait(() => clipboardText() === value, `${client} ${side} exact config clipboard`);
      check(`${client} ${side} scoped config and actual clipboard`, true);
    }
    await execute('document.querySelector(".channel-map").scrollIntoView({block:"start"});');
    await screenshot(`mcp-client-${client}`);
  }
  await click('[data-testid=linux-mcp-client-codex]');
  await wait(
    () =>
      execute(
        `const b=document.querySelector('[data-testid=linux-mcp-client-codex]');return b.getAttribute('aria-pressed')==='true' && getComputedStyle(b).color===getComputedStyle(b.querySelector('.mcp-client-radio')).borderTopColor;`
      ),
    'selected client color transition settles to approved outline'
  );
  result.selectedClientStyle = await execute(
    `const b=document.querySelector('[data-testid=linux-mcp-client-codex]');const r=b.querySelector('.mcp-client-radio');return {button:getComputedStyle(b).color,radio:getComputedStyle(r).borderTopColor,background:getComputedStyle(b).backgroundColor,accent:getComputedStyle(b).getPropertyValue('--color-accent'),radioWidth:getComputedStyle(r).borderTopWidth};`
  );
  check(
    'selected tool retains approved outlined styling and visible semantic radio',
    await execute(
      `const b=document.querySelector('[data-testid=linux-mcp-client-codex]');return getComputedStyle(b).color===getComputedStyle(b.querySelector('.mcp-client-radio')).borderTopColor;`
    )
  );
  await click('[data-testid=linux-generic-mcp-config] summary');
  for (const side of ['local', 'remote']) {
    const value = await execute('return document.querySelector(arguments[0]).textContent;', [
      `[data-testid=linux-${side}-generic-config]`,
    ]);
    assert.equal(
      JSON.parse(value).mcpServers['lynx-local-acceptance-sdk'].url,
      side === 'local' ? local.endpoint : `http://127.0.0.1:${activeRemotePort}/mcp`
    );
    await click(`[data-testid=linux-copy-${side}-generic-config]`);
    await wait(() => clipboardText() === value, `${side} generic configuration clipboard`);
    check(`${side} generic import only includes verified channel`, true);
  }
  await click('[data-testid=linux-generic-mcp-config] summary');
  await execute('document.querySelector(".channel-map").scrollIntoView({block:"start"});');
  await screenshot('mcp-connections');
  result.layoutMatrix = [];
  for (const [width, scale] of [
    [1440, 1],
    [1090, 1],
    [1440, 1.25],
    [1440, 1.5],
  ]) {
    await req('POST', `/session/${session}/window/rect`, { width, height: 1000 });
    await execute('document.documentElement.style.zoom=String(arguments[0]);', [scale]);
    await delay(200);
    await execute(
      'document.querySelector("[data-testid=linux-local-mcp-address]").closest(".channel-command").scrollIntoView({block:"center"});'
    );
    const geometry = await execute(`
      const rect=e=>{const r=e.getBoundingClientRect();return {left:r.left,right:r.right,top:r.top,bottom:r.bottom,width:r.width,height:r.height}};
      const rows=[...document.querySelectorAll('.channel-map .mcp-address-row')].map(row=>({row:rect(row),code:rect(row.querySelector('code')),button:rect(row.querySelector('button')),scrollWidth:row.scrollWidth,clientWidth:row.clientWidth,styles:{display:getComputedStyle(row).display,height:getComputedStyle(row).height,minHeight:getComputedStyle(row).minHeight,flex:getComputedStyle(row).flex,rows:getComputedStyle(row).gridTemplateRows,parentDisplay:getComputedStyle(row.parentElement).display,parentRows:getComputedStyle(row.parentElement).gridTemplateRows,codeDisplay:getComputedStyle(row.querySelector('code')).display,codeHeight:getComputedStyle(row.querySelector('code')).height}}));
      const commands=[...document.querySelectorAll('.channel-map .mcp-command-field')].map(field=>({field:rect(field),code:rect(field.querySelector('code')),button:rect(field.querySelector('button')),scrollWidth:field.scrollWidth,clientWidth:field.clientWidth,userSelect:getComputedStyle(field.querySelector('code')).userSelect || getComputedStyle(field.querySelector('code')).webkitUserSelect}));
      const cards=[...document.querySelectorAll('.mcp-peer-cards>.mcp-connection-card')].map(rect);
      const dialog=document.querySelector('[data-testid=linux-settings-dialog]');
      return {rows,commands,cards,dialog:rect(dialog),viewport:innerWidth,pageWidth:document.documentElement.scrollWidth};
    `);
    result.layoutMatrix.push({ width, scale, geometry });
    await writeFile(
      out + '/layout-latest.json',
      JSON.stringify({ width, scale, geometry }, null, 2)
    );
    assert.equal(geometry.rows.length, 2);
    assert.equal(geometry.commands.length, 2);
    for (const command of geometry.commands) {
      assert.ok(
        command.button.bottom <= command.code.top,
        'command copy control is above the complete value'
      );
      assert.ok(
        Math.abs(command.code.width - command.field.width) < 2,
        'command has full card field width'
      );
      assert.ok(
        Math.abs(command.button.right - command.field.right) < 2,
        'command copy control right edge aligns'
      );
      assert.ok(
        command.code.bottom <= command.field.bottom + 1,
        'wrapped command remains inside its field'
      );
      assert.ok(command.scrollWidth <= command.clientWidth + 1, 'command field never overflows');
      assert.equal(command.userSelect, 'text');
    }
    assert.equal(geometry.cards.length, 2);
    if (Math.abs(geometry.cards[0].top - geometry.cards[1].top) < 2) {
      assert.ok(
        Math.abs(geometry.cards[0].width - geometry.cards[1].width) < 2,
        'peer cards have equal widths'
      );
      assert.ok(
        Math.abs(geometry.rows[0].code.top - geometry.rows[1].code.top) < 2,
        'peer address fields align'
      );
      assert.ok(
        Math.abs(geometry.commands[0].code.top - geometry.commands[1].code.top) < 2,
        'peer command fields align'
      );
      assert.ok(
        Math.abs(geometry.commands[0].code.bottom - geometry.commands[1].code.bottom) < 2,
        'peer command fields have equal height'
      );
    }
    for (const row of geometry.rows) {
      assert.ok(
        Math.abs(row.code.top - row.button.top) < 2,
        'value and copy control top edges align'
      );
      assert.ok(Math.abs(row.row.right - row.button.right) < 2, 'copy control right edge aligns');
      assert.ok(row.code.right <= row.button.left, 'value never overlaps button');
      assert.ok(row.code.bottom <= row.row.bottom + 1, 'wrapped command stays inside its own row');
      assert.ok(row.button.bottom <= row.row.bottom + 1, 'copy control stays inside its own row');
      assert.ok(row.scrollWidth <= row.clientWidth + 1, 'row has no horizontal overflow');
    }
    assert.ok(
      geometry.dialog.left >= -1 && geometry.dialog.right <= geometry.viewport + 1,
      'dialog stays inside viewport'
    );
    check(`MCP row geometry ${width}px content scale ${scale}`, true);
    await screenshot(`mcp-layout-${width}-${Math.round(scale * 100)}`);
  }
  await execute('document.documentElement.style.zoom="";');
  await req('POST', `/session/${session}/window/rect`, { width: 1440, height: 1000 });

  await input('[data-testid=linux-board-name]', '');
  await click('[data-testid=linux-board-save]');
  check(
    'invalid board save retains dialog',
    await execute('return !!document.querySelector("[data-testid=linux-settings-dialog]");')
  );
  check(
    'invalid save reports reason',
    await execute(
      'return document.querySelector("[data-testid=linux-board-save-feedback]").textContent.includes("填写单板名称");'
    )
  );
  await input('[data-testid=linux-board-name]', 'Local Acceptance Board');
  await click('[data-testid=linux-board-save]');
  await wait(
    () => execute('return !document.querySelector("[data-testid=linux-settings-dialog]");'),
    'successful board save closes'
  );
  check('successful board save closes', true);
  await settings();
  await execute(
    `const b=[...document.querySelectorAll('[data-testid=linux-settings-dialog] button')].find(e=>e.textContent.trim()==='保存通道并自动连接');if(!b)throw Error('save channel button missing');b.click();`
  );
  await wait(
    () => execute('return !document.querySelector("[data-testid=linux-settings-dialog]");'),
    'successful channel save and verified connection closes'
  );
  check('successful channel save closes', true);
  await settings();
  await delay(800);
  check(
    'reopened settings stays open',
    await execute('return !!document.querySelector("[data-testid=linux-settings-dialog]");')
  );
  result.safetyBeforeSshOutage = await ipc('hardware_safety_status');
  assert.equal(result.safetyBeforeSshOutage.busy, false);
  sshd.kill('SIGTERM');
  await wait(
    async () =>
      !(await ipc('workbench_list_tunnel_statuses')).some(
        (t) => t.projectId === projectId && t.state === 'connected'
      ),
    'test SSH outage recognized',
    45000
  );
  await wait(
    () =>
      execute(
        `const p=document.querySelector('.channel-map');return p && !p.innerText.includes(arguments[0]) && p.innerText.includes(arguments[1]);`,
        [`http://127.0.0.1:${activeRemotePort}/mcp`, local.endpoint]
      ),
    'remote command cleared while local stays ready'
  );
  check(
    'local service remains independently verified',
    (await ipc('gateway_channel_status', { projectId, deviceId })).verified
  );
  check(
    'offline remote address copy is disabled',
    await execute(
      'return document.querySelector("[data-testid=linux-copy-remote-mcp-address]").disabled;'
    )
  );
  const preservedClipboard = clipboardText();
  await execute('document.querySelector("[data-testid=linux-copy-remote-mcp-address]").click();');
  check('disabled address copy preserves clipboard', clipboardText(), preservedClipboard);
  await click('[data-testid=linux-copy-local-mcp-address]');
  await wait(
    () => clipboardText() === local.endpoint,
    'local address remains copyable during remote outage'
  );
  check('local address copy still works during remote outage', clipboardText(), local.endpoint);
  check(
    'offline remote node cannot keep the verified local indicator color',
    await execute(`
    const local=document.querySelector('.channel-node.is-local small i');
    const remote=document.querySelector('.channel-node.is-remote small i');
    return !!local && !!remote && getComputedStyle(local).backgroundColor !== getComputedStyle(remote).backgroundColor;
  `)
  );
  await execute('document.querySelector(".channel-map").scrollIntoView({block:"start"});');
  await screenshot('local-ready-remote-offline');
  check(
    'offline remote has no copyable launch command',
    await execute(
      `return ![...document.querySelectorAll('.channel-map code')].some(e=>e.textContent.includes('codex')&&e.textContent.includes(arguments[0]));`,
      [`:${activeRemotePort}/`]
    )
  );
  result.localPort = localPort;
  result.remotePort = activeRemotePort;
  result.status = 'PASS';
} catch (error) {
  if (session) await screenshot('failure').catch(() => {});
  result.status = 'FAIL';
  result.error = String(error.stack || error);
} finally {
  try {
    await closeOwnedApp();
  } catch (error) {
    result.cleanupError = String(error);
  }
  if (!session) {
    if (driver) driver.kill('SIGTERM');
    if (sshd) sshd.kill('SIGTERM');
  }
  await writeFile(out + '/result.json', JSON.stringify(result, null, 2) + '\n');
}
console.log(
  JSON.stringify({
    status: result.status,
    checks: result.checks.length,
    error: result.error,
    cleanupError: result.cleanupError,
  })
);
if (result.status !== 'PASS' || result.cleanupError) process.exitCode = 1;
