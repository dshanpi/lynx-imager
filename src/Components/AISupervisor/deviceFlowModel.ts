export interface DeviceFlowNode {
  id: string;
  type: string;
  label: string;
  x: number;
  y: number;
  config: Record<string, unknown>;
}

export interface DeviceFlowEdge {
  id: string;
  source: string;
  target: string;
  branch?: 'success' | 'failure' | 'default';
}

export interface DeviceFlowTemplate {
  [key: string]: unknown;
  kind: 'device_flow';
  version: 1;
  nodes: DeviceFlowNode[];
  edges: DeviceFlowEdge[];
}

export interface DeviceFlowNodeRules {
  deviceDomain?: 'linux' | 'mcu';
  highRisk?: boolean;
  executable?: boolean;
}

interface StoredTask {
  boardId: string;
  triggerType: 'manual_api' | 'schedule' | 'webhook' | 'device';
  trigger: Record<string, unknown>;
  taskTemplate: Record<string, unknown>;
  successConditions: Record<string, unknown>[];
}

export const FLOW_NODE_WIDTH = 190;
export const FLOW_NODE_HEIGHT = 85;
export const FLOW_NODE_GAP = 20;

export const nodeId = () => `node-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;

export const scopeMatchesDomain = (scope: string, domain?: DeviceFlowNodeRules['deviceDomain']) =>
  Boolean(domain && scope.startsWith(`${domain}:`));

export const triggerTypeForNode = (type: string): StoredTask['triggerType'] => {
  if (type === 'trigger.schedule') return 'schedule';
  if (type === 'trigger.webhook') return 'webhook';
  if (type === 'trigger.device' || type === 'trigger.serial_match') return 'device';
  return 'manual_api';
};

export const placeWithoutCoveringExistingNode = (
  nodes: DeviceFlowNode[],
  requestedX: number,
  requestedY: number
) => {
  const baseX = Math.max(10, requestedX);
  const baseY = Math.max(10, requestedY);
  const overlaps = (x: number, y: number) =>
    nodes.some(
      (node) =>
        x < node.x + FLOW_NODE_WIDTH + FLOW_NODE_GAP &&
        x + FLOW_NODE_WIDTH + FLOW_NODE_GAP > node.x &&
        y < node.y + FLOW_NODE_HEIGHT + FLOW_NODE_GAP &&
        y + FLOW_NODE_HEIGHT + FLOW_NODE_GAP > node.y
    );

  for (let row = 0; row < 14; row += 1) {
    for (const column of [0, 1, -1, 2, -2]) {
      const x = Math.max(10, baseX + column * (FLOW_NODE_WIDTH + FLOW_NODE_GAP));
      const y = baseY + row * (FLOW_NODE_HEIGHT + FLOW_NODE_GAP);
      if (!overlaps(x, y)) return { x, y };
    }
  }
  return { x: baseX, y: baseY + nodes.length * (FLOW_NODE_HEIGHT + FLOW_NODE_GAP) };
};

export function primarySequence(flow: DeviceFlowTemplate) {
  const sequence: DeviceFlowNode[] = [];
  const visited = new Set<string>();
  let current =
    flow.nodes.find((node) => node.type.startsWith('trigger.')) ??
    flow.nodes.find((node) => !flow.edges.some((edge) => edge.target === node.id));
  while (current && !visited.has(current.id)) {
    sequence.push(current);
    visited.add(current.id);
    const outgoing = flow.edges.filter((edge) => edge.source === current?.id);
    const nextEdge =
      outgoing.find((edge) => !edge.branch || edge.branch === 'default') ??
      outgoing.find((edge) => edge.branch === 'success') ??
      outgoing[0];
    current = nextEdge ? flow.nodes.find((node) => node.id === nextEdge.target) : undefined;
  }
  return sequence;
}

export function autoLayout(flow: DeviceFlowTemplate): DeviceFlowTemplate {
  const incoming = new Map(flow.nodes.map((node) => [node.id, 0]));
  const outgoing = new Map<string, string[]>();
  for (const edge of flow.edges) {
    incoming.set(edge.target, (incoming.get(edge.target) ?? 0) + 1);
    outgoing.set(edge.source, [...(outgoing.get(edge.source) ?? []), edge.target]);
  }
  const levels = new Map<string, number>();
  const queue = flow.nodes
    .filter((node) => (incoming.get(node.id) ?? 0) === 0)
    .map((node) => node.id);
  while (queue.length) {
    const id = queue.shift()!;
    const level = levels.get(id) ?? 0;
    for (const target of outgoing.get(id) ?? []) {
      levels.set(target, Math.max(levels.get(target) ?? 0, level + 1));
      incoming.set(target, (incoming.get(target) ?? 1) - 1);
      if (incoming.get(target) === 0) queue.push(target);
    }
  }
  const rows = new Map<number, DeviceFlowNode[]>();
  for (const node of flow.nodes) {
    const level = levels.get(node.id) ?? 0;
    rows.set(level, [...(rows.get(level) ?? []), node]);
  }
  return {
    ...flow,
    nodes: flow.nodes.map((node) => {
      const level = levels.get(node.id) ?? 0;
      const row = rows.get(level) ?? [node];
      const index = row.findIndex((item) => item.id === node.id);
      const rowWidth = row.length * 220;
      return {
        ...node,
        x: Math.max(30, 500 - rowWidth / 2 + index * 220),
        y: 50 + level * 130,
      };
    }),
  };
}

function ensureVerticalLayout(flow: DeviceFlowTemplate): DeviceFlowTemplate {
  if (flow.nodes.length < 2) return flow;
  const xs = flow.nodes.map((node) => node.x);
  const ys = flow.nodes.map((node) => node.y);
  const horizontalSpread = Math.max(...xs) - Math.min(...xs);
  const verticalSpread = Math.max(...ys) - Math.min(...ys);
  return horizontalSpread > Math.max(260, verticalSpread * 1.4) ? autoLayout(flow) : flow;
}

export function readDeviceFlow(
  task: StoredTask,
  rulesForNode: (type: string) => DeviceFlowNodeRules | undefined
): DeviceFlowTemplate {
  const template = task.taskTemplate as Partial<DeviceFlowTemplate>;
  if (
    template.kind === 'device_flow' &&
    Array.isArray(template.nodes) &&
    Array.isArray(template.edges)
  ) {
    return ensureVerticalLayout({
      kind: 'device_flow',
      version: 1,
      nodes: template.nodes.map((node) => {
        const domain = rulesForNode(node.type)?.deviceDomain;
        if (
          !String(node.config.boardId ?? '') &&
          task.boardId &&
          scopeMatchesDomain(task.boardId, domain)
        ) {
          return { ...node, config: { ...node.config, boardId: task.boardId } };
        }
        return node;
      }),
      edges: template.edges,
    });
  }
  const trigger = nodeId();
  const workflow = nodeId();
  const verify = nodeId();
  const result = nodeId();
  return {
    kind: 'device_flow',
    version: 1,
    nodes: [
      {
        id: trigger,
        type: `trigger.${task.triggerType === 'manual_api' ? 'manual' : task.triggerType}`,
        label: '任务触发',
        x: 330,
        y: 50,
        config: task.trigger,
      },
      {
        id: workflow,
        type: 'flash.workbench',
        label: '现有 Workbench 工作流',
        x: 330,
        y: 180,
        config: {
          ...task.taskTemplate,
          ...(task.boardId ? { boardId: task.boardId } : {}),
          approvalMode: 'required',
        },
      },
      {
        id: verify,
        type: 'verify.deterministic',
        label: '确定性验收',
        x: 330,
        y: 310,
        config: { conditions: task.successConditions },
      },
      { id: result, type: 'result.success', label: '任务成功', x: 330, y: 440, config: {} },
    ],
    edges: [
      { id: `edge-${trigger}-${workflow}`, source: trigger, target: workflow },
      { id: `edge-${workflow}-${verify}`, source: workflow, target: verify },
      { id: `edge-${verify}-${result}`, source: verify, target: result, branch: 'success' },
    ],
  };
}

export function validateDeviceFlow(
  flow: DeviceFlowTemplate,
  rulesForNode: (type: string) => DeviceFlowNodeRules | undefined
) {
  const issues: string[] = [];
  const ids = new Set(flow.nodes.map((node) => node.id));
  const triggers = flow.nodes.filter((node) => node.type.startsWith('trigger.'));
  if (triggers.length !== 1) issues.push('流程必须且只能包含一个触发器。');
  if (!flow.nodes.some((node) => node.type.startsWith('result.')))
    issues.push('流程至少需要一个结果节点。');
  for (const edge of flow.edges)
    if (!ids.has(edge.source) || !ids.has(edge.target))
      issues.push(`连线 ${edge.id} 引用了不存在的节点。`);
  for (const trigger of triggers)
    if (flow.edges.some((edge) => edge.target === trigger.id))
      issues.push(`触发器“${trigger.label}”必须是流程起点，不能连接在其他节点后面。`);
  for (const result of flow.nodes.filter((node) => node.type.startsWith('result.')))
    if (flow.edges.some((edge) => edge.source === result.id))
      issues.push(`结果节点“${result.label}”必须是流程终点，不能连接后续节点。`);
  const connected = new Set(flow.edges.flatMap((edge) => [edge.source, edge.target]));
  for (const node of flow.nodes)
    if (flow.nodes.length > 1 && !connected.has(node.id))
      issues.push(`节点“${node.label}”尚未连线。`);
  for (const node of flow.nodes) {
    const outgoingEdges = flow.edges.filter((edge) => edge.source === node.id);
    if (node.type === 'verify.deterministic' || node.type === 'control.condition') {
      for (const branch of ['success', 'failure', 'default'])
        if (outgoingEdges.filter((edge) => (edge.branch ?? 'default') === branch).length > 1)
          issues.push(`节点“${node.label}”的${branch}分支存在多条连线。`);
    } else if (outgoingEdges.length > 1) {
      issues.push(`节点“${node.label}”只能连接一个后续节点。`);
    }
  }
  for (const node of flow.nodes) {
    const rules = rulesForNode(node.type);
    if (rules?.executable === false)
      issues.push(`节点“${node.label}”尚未接入执行器，当前版本不能保存运行。`);
    if (rules?.deviceDomain && !String(node.config.boardId ?? ''))
      issues.push(`节点“${node.label}”尚未绑定固定设备。`);
    if (rules?.highRisk && node.config.approvalMode !== 'required')
      issues.push(`高风险节点“${node.label}”必须设为需要审批。`);
    if (
      (node.type === 'serial.send' && !String(node.config.text ?? '').trim()) ||
      (node.type === 'serial.expect' && !String(node.config.pattern ?? '').trim()) ||
      ((node.type === 'serial.command' || node.type === 'adb.command') &&
        !String(node.config.command ?? '').trim()) ||
      (node.type === 'serial.login' &&
        (!String(node.config.username ?? '').trim() || !String(node.config.prompt ?? '').trim())) ||
      (node.type === 'verify.deterministic' && !String(node.config.pattern ?? '').trim())
    )
      issues.push(`节点“${node.label}”还有必填配置未完成。`);
  }
  const outgoing = new Map<string, string[]>();
  for (const edge of flow.edges)
    outgoing.set(edge.source, [...(outgoing.get(edge.source) ?? []), edge.target]);
  if (triggers.length === 1) {
    const reachable = new Set<string>();
    const visit = (id: string) => {
      if (reachable.has(id)) return;
      reachable.add(id);
      (outgoing.get(id) ?? []).forEach(visit);
    };
    visit(triggers[0].id);
    for (const node of flow.nodes)
      if (!reachable.has(node.id))
        issues.push(`节点“${node.label}”无法从触发器到达，请检查连线方向。`);
  }
  const visiting = new Set<string>();
  const visited = new Set<string>();
  const cyclic = (id: string): boolean => {
    if (visiting.has(id)) return true;
    if (visited.has(id)) return false;
    visiting.add(id);
    if ((outgoing.get(id) ?? []).some(cyclic)) return true;
    visiting.delete(id);
    visited.add(id);
    return false;
  };
  if (flow.nodes.some((node) => cyclic(node.id)))
    issues.push('流程不允许无界环；请使用“有界重试”节点。');
  return [...new Set(issues)];
}
