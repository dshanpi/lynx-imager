import assert from 'node:assert/strict';
import { createServer } from 'node:http';

// Only disposable profiles and a loopback HTTP fixture; no real AI keys or hardware.
export async function runExtendedButtonAudit(api) {
  const { execute, invokeTauri: ipc, click, capture, waitFor, selectTab } = api;
  const visible = `e=>{const r=e.getBoundingClientRect();return r.width>0&&r.height>0&&getComputedStyle(e).visibility!=='hidden';}`;
  async function button(name, scope = 'body', prefix = false) {
    const ok = await execute(
      `
      const visible=${visible};
      const all=[...document.querySelectorAll(arguments[1]+' button')].filter(visible);
      const matches=all.filter(e=>(e.getAttribute('aria-label')||'')===arguments[0] || (arguments[2]?e.textContent.trim().startsWith(arguments[0]):e.textContent.trim()===arguments[0]));
      if(matches.length!==1 || matches[0].disabled)throw Error('Button missing/ambiguous/disabled: '+arguments[0]+' count='+matches.length+' reason='+(matches[0]?.title||''));
      matches[0].scrollIntoView({block:'center'});matches[0].click();return true;
    `,
      [name, scope, prefix]
    );
    assert.equal(ok, true);
  }
  async function field(scope, label, value) {
    assert.equal(
      await execute(
        `
      const e=[...document.querySelectorAll(arguments[0]+' label')].find(e=>e.textContent.trim().startsWith(arguments[1]))?.querySelector('input,textarea');
      if(!e || e.disabled)throw Error('Field unavailable: '+arguments[1]);
      const proto=e instanceof HTMLTextAreaElement?HTMLTextAreaElement.prototype:HTMLInputElement.prototype;
      Object.getOwnPropertyDescriptor(proto,'value').set.call(e,arguments[2]);
      e.dispatchEvent(new Event('input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}));return e.value===arguments[2];
    `,
        [scope, label, value]
      ),
      true
    );
  }
  const verify = (bl, name, description, operation) =>
    capture(bl, name, [{ description, operation }]);

  await click('sidebar-toggle');
  await verify('BL-00', 'sidebar-collapse-button', '侧栏折叠按钮实际生效', () =>
    execute(
      `return document.querySelector('[data-testid=app-sidebar]').classList.contains('sidebar--collapsed');`
    )
  );
  await click('sidebar-toggle');
  for (const id of ['development-workbenches', 'learning-center', 'ai-channel']) {
    await click('sidebar-group-' + id);
    await verify('BL-00', 'group-' + id + '-collapse', '导航分组可折叠', () =>
      execute(
        `return document.querySelector('[data-testid="sidebar-group-${id}"]').getAttribute('aria-expanded')==='false';`
      )
    );
    await click('sidebar-group-' + id);
    await verify('BL-00', 'group-' + id + '-expand', '导航分组可展开', () =>
      execute(
        `return document.querySelector('[data-testid="sidebar-group-${id}"]').getAttribute('aria-expanded')==='true';`
      )
    );
  }
  await click('sidebar-tool-workbench');
  const linuxBefore = await ipc('workbench_list_devices', {});
  await click('linux-new-board');
  await verify(
    'BL-01',
    'linux-new-board-cancel-ready',
    '新建板卡表单打开且没有自动保存',
    async () =>
      !!(await execute(`return document.querySelector('[data-testid=linux-settings-dialog]');`)) &&
      (await ipc('workbench_list_devices', {})).length === linuxBefore.length
  );
  await button('关闭', '[data-testid=linux-settings-dialog] .workbench-settings-header');
  await verify(
    'BL-01',
    'linux-new-board-cancel',
    '关闭新建表单不创建单板',
    async () =>
      !(await execute(`return !!document.querySelector('[data-testid=linux-settings-dialog]');`)) &&
      (await ipc('workbench_list_devices', {})).length === linuxBefore.length
  );
  await execute(
    `const e=[...document.querySelectorAll('.board-item-select')].find(e=>e.textContent.includes('BL Gate Linux Board'));if(!e)throw Error('Saved Linux selection missing');e.click();`
  );
  await verify('BL-01', 'linux-select-existing-after-cancel', '取消新建后可重新选择原单板', () =>
    execute(`return !!document.querySelector('[data-testid=linux-tab-overview]');`)
  );
  await click('sidebar-tool-mcu-workbench');
  const mcuBefore = await ipc('mcu_list_boards', {});
  await click('mcu-new-board');
  await click('mcu-settings-close');
  await verify(
    'BL-06',
    'mcu-new-board-cancel',
    '取消新建 MCU 不写入设备',
    async () =>
      !(await execute(`return !!document.querySelector('[data-testid=mcu-settings-dialog]');`)) &&
      (await ipc('mcu_list_boards', {})).length === mcuBefore.length
  );

  await execute(
    `const e=[...document.querySelectorAll('.board-item-select')].find(e=>e.textContent.includes('BL Gate MCU'));if(!e)throw Error('Saved MCU selection missing');e.click();`
  );
  await waitFor('MCU selected after cancel', () =>
    execute(`return !!document.querySelector('[data-testid=mcu-tab-overview]');`)
  );
  await verify('BL-06', 'mcu-select-existing-after-cancel', '取消新建后可重新选择原 MCU 单板', () =>
    execute(`return !!document.querySelector('[data-testid=mcu-tab-overview]');`)
  );

  await click('sidebar-tool-ai-supervisor-config');
  await selectTab('ai-config-tab-policy');
  await button('新建监督策略');
  await field('.ais-policy-unified', '策略名称', '');
  await button('保存策略与权限');
  await verify(
    'BL-10',
    'policy-invalid-save',
    '空策略名称被拒绝而非静默保存',
    async () => !(await ipc('supervisor_list_policies', {})).some((p) => p.name === '')
  );
  const policyName = 'LYNX isolated button audit policy';
  await field('.ais-policy-unified', '策略名称', policyName);
  await execute(
    `const e=[...document.querySelectorAll('.ais-policy-board-scope label')].find(e=>e.textContent.includes('BL Gate Linux Board'))?.querySelector('input[type=checkbox]');if(!e)throw Error('No saved fixture board');if(!e.checked)e.click();`
  );
  await button('保存策略与权限');
  await waitFor('UI policy persisted', async () =>
    (await ipc('supervisor_list_policies', {})).find((p) => p.name === policyName)
  );
  await verify(
    'BL-10',
    'policy-save-button',
    '策略保存按钮写入隔离数据库且固定单板范围',
    async () =>
      (await ipc('supervisor_list_policies', {})).find((p) => p.name === policyName)?.boardScope
        .length > 0
  );
  await selectTab('ai-config-tab-notifications');
  await button('保存通知设置');
  await verify('BL-10', 'notification-save-button', '通知设置保存后保留当前策略', async () =>
    (await ipc('supervisor_list_policies', {})).some((p) => p.name === policyName)
  );

  let mode = 'ok';
  const calls = [];
  const violations = [];
  let savedId;
  const fixtureKey = 'lynx-ui-fixture-not-a-real-key';
  const server = createServer((req, res) => {
    res.setHeader('Content-Type', 'application/json');
    res.setHeader('Connection', 'close');
    calls.push({ method: req.method, path: req.url });
    if (req.headers.authorization !== 'Bearer ' + fixtureKey) {
      violations.push('fixture auth did not match');
      res.writeHead(401);
      res.end('{}');
      return;
    }
    if (mode === 'unauthorized') {
      res.writeHead(401);
      res.end('{}');
      return;
    }
    if (mode === 'invalid-json') {
      res.end('not-json');
      return;
    }
    if (req.method === 'GET' && req.url === '/v1/models') {
      res.end(
        JSON.stringify({ object: 'list', data: [{ id: 'lynx-fixture-model', object: 'model' }] })
      );
      return;
    }
    if (req.method === 'POST' && req.url === '/v1/chat/completions') {
      let body = '';
      req.on('data', (b) => (body += b));
      req.on('end', () => {
        try {
          const input = JSON.parse(body);
          if (input.model !== 'lynx-fixture-model') throw Error();
        } catch {
          violations.push('fixture model did not match');
          res.writeHead(400);
          res.end('{}');
          return;
        }
        res.end(
          JSON.stringify({
            id: 'local-test',
            object: 'chat.completion',
            choices: [
              { index: 0, message: { role: 'assistant', content: 'OK' }, finish_reason: 'stop' },
            ],
            usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 },
          })
        );
      });
      return;
    }
    violations.push('unexpected fixture route');
    res.writeHead(404);
    res.end('{}');
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  try {
    await selectTab('ai-config-tab-providers');
    await button('新建 AI 服务连接');
    await field('.ais-provider-editor', '连接名称', 'LYNX isolated UI provider');
    await field(
      '.ais-provider-editor',
      'API Base URL',
      `http://127.0.0.1:${server.address().port}/v1`
    );
    await field('.ais-provider-editor', '模型', 'lynx-fixture-model');
    await field('.ais-provider-editor', 'API Key', '');
    await button('保存连接', '.ais-provider-actions');
    await verify(
      'BL-10',
      'provider-empty-key-rejected',
      '新连接缺少密钥时不产生持久连接',
      async () =>
        !(await ipc('supervisor_list_providers', {})).some(
          (p) => p.name === 'LYNX isolated UI provider'
        )
    );
    await field('.ais-provider-editor', 'API Key', fixtureKey);
    await button('保存连接', '.ais-provider-actions');
    const saved = await waitFor('fixture provider saved with isolated keyring', async () =>
      (await ipc('supervisor_list_providers', {})).find(
        (p) => p.name === 'LYNX isolated UI provider'
      )
    );
    savedId = saved.id;
    assert.equal(saved.hasApiKey, true);
    await verify(
      'BL-10',
      'provider-save-button',
      '连接保存后密码输入框清空，投影只返回密钥存在标记',
      () =>
        execute(
          `return document.querySelector('.ais-provider-editor input[type=password]').value==='';`
        )
    );
    await button('测试连接', '.ais-provider-actions');
    await waitFor(
      'provider two endpoint test',
      async () =>
        (await ipc('supervisor_list_providers', {})).find((p) => p.id === savedId)
          ?.lastTestStatus === 'available'
    );
    assert.deepEqual(
      calls.map((c) => c.path),
      ['/v1/models', '/v1/chat/completions']
    );
    await verify(
      'BL-10',
      'provider-test-button',
      '实际 HTTP 模型列表和对话测试成功，不依赖前端假成功',
      async () =>
        (await ipc('supervisor_list_providers', {})).find((p) => p.id === savedId)
          ?.lastTestStatus === 'available'
    );
    mode = 'unauthorized';
    await button('测试连接', '.ais-provider-actions');
    await waitFor(
      'provider rejected credentials shown',
      async () =>
        (await ipc('supervisor_list_providers', {})).find((p) => p.id === savedId)
          ?.lastTestStatus === 'authentication_failed'
    );
    await verify(
      'BL-10',
      'provider-unauthorized',
      '401 明确变为认证失败，保留连接配置供修正',
      async () =>
        (await ipc('supervisor_list_providers', {})).find((p) => p.id === savedId)?.hasApiKey ===
        true
    );
    mode = 'invalid-json';
    await button('测试连接', '.ais-provider-actions');
    await waitFor(
      'provider malformed response shown',
      async () =>
        (await ipc('supervisor_list_providers', {})).find((p) => p.id === savedId)
          ?.lastTestStatus === 'invalid_response'
    );
    await verify(
      'BL-10',
      'provider-invalid-response',
      '成功状态码但非法 JSON 不得显示连接成功',
      async () =>
        (await ipc('supervisor_list_providers', {})).find((p) => p.id === savedId)
          ?.lastTestStatus === 'invalid_response'
    );
    mode = 'ok';
    await button('测试连接', '.ais-provider-actions');
    await waitFor(
      'provider recovery succeeds',
      async () =>
        (await ipc('supervisor_list_providers', {})).find((p) => p.id === savedId)
          ?.lastTestStatus === 'available'
    );
    await button('删除当前连接');
    await waitFor(
      'provider UI delete',
      async () => !(await ipc('supervisor_list_providers', {})).some((p) => p.id === savedId)
    );
    savedId = null;
    await verify(
      'BL-10',
      'provider-delete-button',
      '删除按钮清理隔离测试连接和对应测试密钥',
      async () =>
        !(await ipc('supervisor_list_providers', {})).some(
          (p) => p.name === 'LYNX isolated UI provider'
        )
    );
    assert.deepEqual(violations, []);
  } finally {
    if (savedId) await ipc('supervisor_delete_provider', { providerId: savedId });
    await new Promise((resolve) => server.close(resolve));
  }

  await click('sidebar-tool-ai-supervisor-automatic');
  await button('新建设备流程');
  await execute(`
    const e=document.querySelector('[aria-label="新任务固定设备"]');
    if(!e)throw Error('No new-task board selector');
    const option=[...e.options].find(o=>o.textContent.includes('BL Gate Linux Board'));
    if(!option)throw Error('Fixture Linux scope missing');
    Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype,'value').set.call(e,option.value);e.dispatchEvent(new Event('change',{bubbles:true}));
  `);
  await button('空白顺序流程', 'body', true);
  await verify(
    'BL-11',
    'blank-sequence-template-button',
    '空白模板生成确定性验收和成功/失败分支',
    () =>
      execute(
        `return document.querySelectorAll('.ais-flow-sequence-step').length>0 || document.querySelector('.ais-flow-sequence')?.textContent.includes('确定性验收');`
      )
  );
  await button('高级画布');
  await button('放大');
  await button('缩小');
  await button('自动竖向布局');
  await button('撤销');
  await button('重做');
  await button('适配视图');
  await verify(
    'BL-11',
    'canvas-horizontal-toolbar',
    '画布操作栏按钮标签保持横向，名称与按钮分行布局',
    () =>
      execute(
        `const actions=document.querySelector('.ais-flow-toolbar-actions'),identity=document.querySelector('.ais-flow-identity');return actions.getBoundingClientRect().top>=identity.getBoundingClientRect().bottom&&[...actions.querySelectorAll('button')].every(b=>getComputedStyle(b).whiteSpace==='nowrap'&&b.scrollWidth<=b.clientWidth+1&&b.getBoundingClientRect().height<=46);`
      )
  );
  await verify(
    'BL-11',
    'canvas-toolbar-buttons',
    '画布缩放、布局、撤销重做和适配均保留四个模板节点',
    () => execute(`return document.querySelectorAll('.ais-flow-node').length===4;`)
  );
  const portsFit = () =>
    execute(`
    const stage=document.querySelector('.ais-flow-stage'),base=stage.getBoundingClientRect();
    const zoom=new DOMMatrix(getComputedStyle(stage).transform).a;
    const ports=[...document.querySelectorAll('.ais-flow-node button.ais-node-input,.ais-flow-node button.ais-node-output')].map(port=>{const r=port.getBoundingClientRect();return {input:port.classList.contains('ais-node-input'),x:(r.left+r.right)/2,y:(r.top+r.bottom)/2,round:Math.abs(r.width-r.height)<1.5&&Number.parseFloat(getComputedStyle(port).borderTopLeftRadius)>=Math.min(port.offsetWidth,port.offsetHeight)/2,large:r.width>=31.5&&r.height>=31.5};});
    const close=(point,input)=>ports.some(p=>p.input===input&&Math.abs(p.x-(base.left+point.x*zoom))<2&&Math.abs(p.y-(base.top+point.y*zoom))<2);
    const paths=[...document.querySelectorAll('.ais-flow-edge')];
    return ports.length===6&&ports.every(p=>p.round&&p.large)&&paths.length===3&&paths.every(path=>close(path.getPointAtLength(0),false)&&close(path.getPointAtLength(path.getTotalLength()),true));
  `);
  await verify(
    'BL-11',
    'canvas-round-link-targets',
    '连线端口至少 32px、保持圆形且与图形端点对齐',
    portsFit
  );
  for (let i = 0; i < 16; i++) {
    const value = await execute(
      `return Number.parseInt([...document.querySelectorAll('.ais-flow-toolbar-actions > span')].find(e=>e.textContent.includes('%'))?.textContent||'100');`
    );
    if (value < 50) break;
    await button('缩小');
  }
  await verify(
    'BL-11',
    'canvas-overview-zoom',
    '缩略视图隐藏连线按钮并提示放大，节点和连线保持',
    () =>
      execute(
        `return !!document.querySelector('.ais-flow-overview-hint')&&document.querySelectorAll('.ais-node-input,.ais-node-output').length===0&&document.querySelectorAll('.ais-flow-node').length===4&&document.querySelectorAll('.ais-flow-edge').length===3;`
      )
  );
  for (let i = 0; i < 16; i++) {
    if (
      await execute(
        `return document.querySelectorAll('.ais-node-input,.ais-node-output').length===6;`
      )
    )
      break;
    await button('放大');
  }
  await verify('BL-11', 'canvas-edit-zoom-restored', '放大后重新显示完整尺寸的连线端口', portsFit);
  await button('顺序编排');
  await execute(
    `const e=document.querySelector('[aria-label="任务名称"]');if(!e)throw Error('No task name');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(e,'LYNX isolated draft');e.dispatchEvent(new Event('input',{bubbles:true}));`
  );
  await button('保存草稿');
  await waitFor('flow draft feedback', () =>
    execute(
      `return document.querySelector('.ais-flow-save-feedback')?.textContent.includes('草稿已保存');`
    )
  );
  await verify('BL-11', 'flow-draft-save-button', '未完善的流程只能保存为禁用草稿', async () =>
    (await ipc('supervisor_list_automatic_tasks', {})).some(
      (t) => t.taskTemplate?.kind === 'device_flow' && t.enabled === false
    )
  );

  const fixtureTask = (await ipc('supervisor_list_automatic_tasks', {})).find(
    (t) => t.name === 'LYNX isolated draft'
  );
  assert.ok(fixtureTask?.id);
  await click('ai-automatic-task-' + fixtureTask.id);
  await verify('BL-11', 'flow-select-saved-task', '任务列表可选择已保存的禁用草稿', () =>
    execute(
      `return document.querySelector('[aria-label="任务名称"]').value==='LYNX isolated draft';`
    )
  );
  await button('高级画布');
  await button('从手动开始开始连线');
  await verify('BL-11', 'flow-link-selection-highlight', '选择输出端口显示连线模式与高亮', () =>
    execute(
      `const e=document.querySelector('.ais-node-output.active');return !!e&&getComputedStyle(e,'::before').boxShadow!=='none';`
    )
  );
  await button('连接到确定性验收');
  await verify('BL-11', 'flow-duplicate-link-blocked', '重复连线操作给出反馈且不新增边', () =>
    execute(
      `return document.querySelectorAll('.ais-flow-edge').length===3&&document.querySelector('.ais-flow-link-feedback')?.textContent.includes('已经连接');`
    )
  );
  await execute(
    `const e=document.querySelector('.ais-flow-edge-hit');e.dispatchEvent(new MouseEvent('click',{bubbles:true}));`
  );
  await button('删除选中连线');
  await verify('BL-11', 'flow-delete-selected-link', '删除连线按钮只移除所选边', () =>
    execute(`return document.querySelectorAll('.ais-flow-edge').length===2;`)
  );
  await button('撤销');
  await verify('BL-11', 'flow-undo-link-delete', '撤销恢复刚删除的连线', () =>
    execute(`return document.querySelectorAll('.ais-flow-edge').length===3;`)
  );
  await execute(
    `const e=[...document.querySelectorAll('.ais-palette-node')].find(e=>e.getAttribute('aria-label')?.startsWith('打开串口：'));if(!e||e.getAttribute('aria-disabled')==='true')throw Error('Serial-open palette item unavailable');e.scrollIntoView({block:'center'});e.click();`
  );
  await verify('BL-11', 'flow-add-palette-node', '节点库点击添加步骤而不执行硬件动作', () =>
    execute(`return document.querySelectorAll('.ais-flow-node').length===5;`)
  );
  await button('删除节点', '.ais-flow-inspector');
  await verify('BL-11', 'flow-delete-selected-node', '删除所选新增节点保留原四节点', () =>
    execute(`return document.querySelectorAll('.ais-flow-node').length===4;`)
  );
  await button('串口自动登录', '.ais-flow-toolbar-actions');
  await verify(
    'BL-11',
    'flow-serial-template-button',
    '串口模板生成六个节点且未自动执行或保存',
    async () =>
      (await execute(`return document.querySelectorAll('.ais-flow-node').length===6;`)) &&
      (await ipc('supervisor_list_runs', { limit: 100 })).length === 0 &&
      (await ipc('supervisor_list_automatic_tasks', {})).find((t) => t.id === fixtureTask.id)
        ?.taskTemplate.nodes.length === 4
  );
  await button('新建设备流程');
  await click('ai-automatic-task-' + fixtureTask.id);
  await button('高级画布');
  await verify('BL-11', 'flow-reload-saved-task', '重新选择草稿恢复已保存四节点与固定单板', () =>
    execute(
      `return document.querySelectorAll('.ais-flow-node').length===4&&document.querySelector('.ais-flow-inspector').textContent.includes('BL Gate Linux Board');`
    )
  );
  await button('删除任务', '.ais-flow-inspector');
  await waitFor(
    'fixture task deleted',
    async () =>
      !(await ipc('supervisor_list_automatic_tasks', {})).some((t) => t.id === fixtureTask.id)
  );
  await verify(
    'BL-11',
    'flow-delete-fixture-task',
    '删除任务按钮移除隔离测试草稿',
    async () =>
      (await ipc('supervisor_list_automatic_tasks', {})).length === 0 &&
      (await ipc('workbench_list_tasks', { limit: 100 })).length === 0
  );
}
