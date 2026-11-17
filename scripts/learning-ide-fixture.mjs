// Test-only VS Code fixture: isolated profile, loopback SSH, generated files only.
// Uses the public extension API to verify remote file reads and saved edits.
import { mkdir, writeFile, readFile, chmod, readdir } from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
const hash = (bytes) => createHash('sha256').update(bytes).digest('hex');

export async function prepareIdeFixture({ data, sdk, port, env, extensions }) {
  const home = path.join(data, 'ide-home'),
    profile = path.join(data, 'ide-profile');
  const extension = path.join(data, 'ide-test-extension');
  const config = path.join(home, '.ssh/config');
  const target = path.join(sdk, 'lesson/src/IDE 示例 %2e%2e#?.c');
  const course = path.join(sdk, 'lesson/src/main.c');
  const proof = path.join(data, 'ide-proof.json');
  await mkdir(path.dirname(config), { recursive: true });
  await mkdir(path.join(profile, 'User'), { recursive: true });
  await mkdir(extension, { recursive: true });
  const hostKey = (await readFile(data + '/host.pub', 'utf8')).trim().split(/\s+/);
  await writeFile(home + '/.ssh/known_hosts', `[127.0.0.1]:${port} ${hostKey[0]} ${hostKey[1]}\n`);
  await writeFile(
    config,
    `Host lynx-lesson\n HostName 127.0.0.1\n User ${os.userInfo().username}\n Port ${port}\n IdentityFile ${data}/client\n IdentitiesOnly yes\n UserKnownHostsFile ${home}/.ssh/known_hosts\n StrictHostKeyChecking yes\n`
  );
  await writeFile(
    path.join(profile, 'User/settings.json'),
    JSON.stringify({
      'remote.SSH.configFile': config,
      'remote.SSH.remotePlatform': { 'lynx-lesson': 'linux' },
      'remote.SSH.useLocalServer': false,
      'remote.SSH.useExecServer': true,
      'telemetry.telemetryLevel': 'off',
      'update.mode': 'none',
      'extensions.autoCheckUpdates': false,
      'extensions.autoUpdate': false,
      'workbench.startupEditor': 'none',
      'security.workspace.trust.enabled': false,
      // Only this generated fixture profile accepts our known loopback URL automatically.
      'security.promptForRemoteFileProtocolHandling': false,
    })
  );
  await writeFile(
    extension + '/package.json',
    JSON.stringify({
      name: 'lynx-isolated-ide-acceptance',
      publisher: 'lynx-test-only',
      version: '0.0.1',
      engines: { vscode: '^1.100.0' },
      main: './extension.js',
      activationEvents: ['*'],
      extensionKind: ['ui'],
    })
  );
  await writeFile(
    extension + '/extension.js',
    `const vscode=require('vscode');
const fs=require('fs'); const crypto=require('crypto');
const target=${JSON.stringify(target)},course=${JSON.stringify(course)},proof=${JSON.stringify(proof)};
const sdk=${JSON.stringify(sdk)},folderProof=${JSON.stringify(data + '/ide-folder-proof.json')};
let running=false;
const hash=b=>crypto.createHash('sha256').update(b).digest('hex');
exports.activate=async()=>{
  const checkFolder=()=>{
    const folder=vscode.workspace.workspaceFolders?.find(f=>f.uri.scheme==='vscode-remote'&&f.uri.authority==='ssh-remote+lynx-lesson'&&f.uri.path===sdk);
    if(folder&&vscode.env.remoteName==='ssh-remote')fs.writeFileSync(folderProof,JSON.stringify({status:'PASS',uri:folder.uri.toString(),path:folder.uri.path,remoteName:vscode.env.remoteName}));
  };
  vscode.workspace.onDidChangeWorkspaceFolders(checkFolder); checkFolder();
  const check=async editor=>{
    if(running||fs.existsSync(proof)||!editor||editor.document.uri.scheme!=='vscode-remote'||editor.document.uri.path!==target)return;
    running=true;
    try {
      const uri=editor.document.uri;
      if(uri.authority!=='ssh-remote+lynx-lesson'||vscode.env.remoteName!=='ssh-remote')throw Error('Wrong remote authority');
      if(editor.document.getText()!=='/* isolated IDE literal path */\\n')throw Error('Wrong target contents');
      const sourceUri=uri.with({path:course});
      const doc=await vscode.workspace.openTextDocument(sourceUri);
      await vscode.window.showTextDocument(doc);
      const before=doc.getText();
      const edit=new vscode.WorkspaceEdit();
      edit.insert(sourceUri,new vscode.Position(0,0),'/* saved through VS Code Remote SSH */\\n');
      if(!await vscode.workspace.applyEdit(edit)||!await doc.save())throw Error('Remote save failed');
      const bytes=await vscode.workspace.fs.readFile(sourceUri);
      fs.writeFileSync(proof,JSON.stringify({status:'PASS',remoteName:vscode.env.remoteName,authority:uri.authority,target:uri.path,course:sourceUri.path,beforeSha256:hash(before),afterSha256:hash(bytes),saved:!doc.isDirty}));
    } catch(e) {fs.writeFileSync(proof,JSON.stringify({status:'FAIL',error:String(e)}));}
  };
  vscode.window.onDidChangeActiveTextEditor(check);
  await check(vscode.window.activeTextEditor);
};
`
  );
  // Register a URL handler only inside this test's XDG profile. Tauri still uses
  // its real system opener; the handler captures the URI and starts real Code.
  await mkdir(data + '/data/applications', { recursive: true });
  const handler = data + '/ide-url-handler.py';
  await writeFile(
    handler,
    `#!/usr/bin/python3
import json,subprocess,sys
from pathlib import Path
data=Path(${JSON.stringify(data)})
data.joinpath('ide-request.json').write_text(json.dumps({'uri':sys.argv[1]}))
with data.joinpath('ide-launch.log').open('ab') as log:
 subprocess.Popen(['/usr/bin/code','--user-data-dir',${JSON.stringify(profile)},'--extensions-dir',${JSON.stringify(extensions)},'--extensionDevelopmentPath',${JSON.stringify(extension)},'--disable-gpu','--open-url',sys.argv[1]],stdout=log,stderr=log)
`
  );
  await chmod(handler, 0o700);
  await writeFile(
    data + '/data/applications/lynx-test-ide.desktop',
    `[Desktop Entry]\nType=Application\nName=LYNX isolated IDE test\nExec=${handler} %u\nTerminal=false\nMimeType=x-scheme-handler/vscode;\n`
  );
  await writeFile(
    data + '/config/mimeapps.list',
    '[Default Applications]\nx-scheme-handler/vscode=lynx-test-ide.desktop;\n'
  );
  env.HOME = home;
  return { data, sdk, config, target, course, proof, profile, env };
}

export async function verifyIdeFixture(
  fixture,
  { ipc, reject, wait, sessionId, evidence, storeDigest, output }
) {
  const { data, target, course, config } = fixture;
  await writeFile(target, '/* isolated IDE literal path */\n');
  const beforeSource = await readFile(course);
  const before = await storeDigest();
  await reject('learning_open_ide', { sessionId }, /LEARN_IDE_ALIAS_REQUIRED/);
  await reject('learning_open_ide', { sessionId, sshAlias: 'missing' }, /LEARN_IDE_ALIAS_MISMATCH/);
  await reject(
    'learning_open_ide',
    { sessionId, sshAlias: 'lynx-lesson', relativeFile: '../private' },
    /LEARN_IDE_PATH_INVALID/
  );
  const valid = await readFile(config, 'utf8');
  await writeFile(config, valid.replace(/ Port \d+/, ' Port invalid'));
  await reject(
    'learning_open_ide',
    { sessionId, sshAlias: 'lynx-lesson' },
    /LEARN_IDE_ALIAS_MISMATCH/
  );
  await writeFile(config, valid);
  await assert.rejects(readFile(data + '/ide-request.json'), { code: 'ENOENT' });
  await ipc('learning_open_ide', {
    sessionId,
    sshAlias: 'lynx-lesson',
    relativeFile: path.relative(fixture.sdk, target),
  });
  await wait(async () => {
    try {
      return JSON.parse(await readFile(data + '/ide-request.json', 'utf8'));
    } catch {
      return false;
    }
  }, 'real system URL handler');
  const request = JSON.parse(await readFile(data + '/ide-request.json', 'utf8'));
  assert.equal(
    decodeURIComponent(new URL(request.uri).pathname),
    '/ssh-remote+lynx-lesson' + target + ':1'
  );
  evidence.ide = { request, beforeSourceSha256: hash(beforeSource) };
  evidence.checks.push({
    name: 'real system opener receives exact custom-port alias and literal filename; invalid requests never invoke handler',
    status: 'PASS',
  });
  await wait(
    async () => {
      try {
        return JSON.parse(await readFile(fixture.proof, 'utf8'));
      } catch {
        return false;
      }
    },
    'VS Code Remote SSH extension reads target and saves course source',
    60000
  );
  const proof = JSON.parse(await readFile(fixture.proof, 'utf8'));
  evidence.ide.proof = proof;
  assert.equal(proof.status, 'PASS', JSON.stringify(proof));
  assert.equal(proof.beforeSha256, hash(beforeSource));
  assert.equal(proof.afterSha256, hash(await readFile(course)));
  assert.notEqual(proof.beforeSha256, proof.afterSha256);
  assert.equal(proof.saved, true);
  assert.equal(
    await storeDigest(),
    before,
    'IDE operations must not write learning progress/evidence'
  );
  evidence.checks.push({
    name: 'actual VS Code Remote SSH reads literal target and saves generated course source; learning database unchanged',
    status: 'PASS',
  });
  const capture = spawnSync('import', [
    '-display',
    fixture.env.DISPLAY,
    '-window',
    'root',
    path.join(output, 'ide-saved-source.png'),
  ]);
  assert.equal(capture.status, 0, 'Capture only the isolated test display');
  await ipc('learning_open_ide', { sessionId, sshAlias: 'lynx-lesson', relativeFile: '' });
  await wait(
    async () => {
      try {
        return JSON.parse(await readFile(data + '/ide-folder-proof.json', 'utf8'));
      } catch {
        return false;
      }
    },
    'VS Code opens bound remote project as a folder',
    60000
  );
  evidence.ide.folder = JSON.parse(await readFile(data + '/ide-folder-proof.json', 'utf8'));
  assert.equal(evidence.ide.folder.status, 'PASS');
  assert.equal(evidence.ide.folder.path, fixture.sdk);
  assert.equal(await storeDigest(), before);
  evidence.checks.push({
    name: 'empty file opens the actual bound remote project folder without writing learning progress',
    status: 'PASS',
  });
  // No dirty editors remain; close only our isolated Code instance.
  const owned = [];
  for (const pid of await readdir('/proc')) {
    if (!/^\d+$/.test(pid)) continue;
    try {
      const args = (await readFile(`/proc/${pid}/cmdline`, 'utf8')).split('\0');
      if (
        args[0] === '/usr/share/code/code' &&
        args.includes(fixture.profile) &&
        !args.some((a) => a.startsWith('--type='))
      )
        owned.push(Number(pid));
    } catch {
      /* Process exited. */
    }
  }
  assert.equal(owned.length, 1, 'Own isolated VS Code main process');
  process.kill(owned[0], 'SIGTERM');
  await wait(
    async () => !(await readdir('/proc')).includes(String(owned[0])),
    'isolated Code exits with saved file'
  );
}
