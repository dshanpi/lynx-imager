import { autoLayout, nodeId, type DeviceFlowTemplate } from './deviceFlowModel';

export function createSerialLoginTemplate(boardId: string): DeviceFlowTemplate {
  const trigger = nodeId();
  const open = nodeId();
  const login = nodeId();
  const verify = nodeId();
  const success = nodeId();
  const failed = nodeId();
  return autoLayout({
    kind: 'device_flow',
    version: 1,
    nodes: [
      { id: trigger, type: 'trigger.manual', label: '手动开始', x: 0, y: 0, config: {} },
      {
        id: open,
        type: 'serial.open',
        label: '打开设备串口',
        x: 0,
        y: 0,
        config: { boardId },
      },
      {
        id: login,
        type: 'serial.login',
        label: '自动登录 Shell',
        x: 0,
        y: 0,
        config: {
          boardId,
          username: 'root',
          credentialRef: '',
          loginPrompt: 'login:',
          passwordPrompt: 'Password:',
          prompt: '#',
          timeoutMs: 30000,
        },
      },
      {
        id: verify,
        type: 'verify.deterministic',
        label: '确认 Shell 提示符',
        x: 0,
        y: 0,
        config: { kind: 'output_match', pattern: '#' },
      },
      { id: success, type: 'result.success', label: '登录成功', x: 0, y: 0, config: {} },
      { id: failed, type: 'result.failed', label: '登录失败', x: 0, y: 0, config: {} },
    ],
    edges: [
      { id: `edge-${trigger}-${open}`, source: trigger, target: open },
      { id: `edge-${open}-${login}`, source: open, target: login },
      { id: `edge-${login}-${verify}`, source: login, target: verify },
      { id: `edge-${verify}-${success}`, source: verify, target: success, branch: 'success' },
      { id: `edge-${verify}-${failed}`, source: verify, target: failed, branch: 'failure' },
    ],
  });
}

export function createBlankSequenceTemplate(): DeviceFlowTemplate {
  const trigger = nodeId();
  const verify = nodeId();
  const success = nodeId();
  const failed = nodeId();
  return autoLayout({
    kind: 'device_flow',
    version: 1,
    nodes: [
      { id: trigger, type: 'trigger.manual', label: '手动开始', x: 0, y: 0, config: {} },
      {
        id: verify,
        type: 'verify.deterministic',
        label: '确定性验收',
        x: 0,
        y: 0,
        config: { kind: 'output_match', pattern: '' },
      },
      { id: success, type: 'result.success', label: '任务成功', x: 0, y: 0, config: {} },
      { id: failed, type: 'result.failed', label: '任务失败', x: 0, y: 0, config: {} },
    ],
    edges: [
      { id: `edge-${trigger}-${verify}`, source: trigger, target: verify },
      { id: `edge-${verify}-${success}`, source: verify, target: success, branch: 'success' },
      { id: `edge-${verify}-${failed}`, source: verify, target: failed, branch: 'failure' },
    ],
  });
}
