import type {
  McuBoardConfig,
  SupervisorAutomaticTask,
  SupervisorPolicy,
  WorkbenchDevice,
} from '../../Platform/IPC/Commands';
import { triggerTypeForNode, type DeviceFlowTemplate } from './deviceFlowModel';

export interface DeviceFlowBoardOption {
  value: string;
  label: string;
}

export interface DeviceFlowBoardOptions {
  linux: DeviceFlowBoardOption[];
  mcu: DeviceFlowBoardOption[];
}

export function buildDeviceFlowBoardOptions(
  devices: WorkbenchDevice[],
  mcuBoards: McuBoardConfig[]
): DeviceFlowBoardOptions {
  return {
    linux: devices
      .filter((device): device is WorkbenchDevice & { id: number } => device.id !== null)
      .map((device) => ({ value: `linux:${device.id}`, label: `${device.name} · Linux/SoC` })),
    mcu: mcuBoards.map((board) => ({ value: `mcu:${board.id}`, label: `${board.name} · MCU` })),
  };
}

function allBoardOptions(options: DeviceFlowBoardOptions) {
  return [...options.linux, ...options.mcu];
}

export function deviceFlowBoardDisplayName(options: DeviceFlowBoardOptions, scope: string) {
  return (
    allBoardOptions(options).find((option) => option.value === scope)?.label ??
    (scope ? '原关联设备已不存在' : '未绑定设备')
  );
}

export function deviceFlowBoardAvailable(options: DeviceFlowBoardOptions, scope: string) {
  return allBoardOptions(options).some((option) => option.value === scope);
}

export function taskWithDeviceFlow(
  next: DeviceFlowTemplate,
  base: SupervisorAutomaticTask,
  preferredBoardId?: string
): SupervisorAutomaticTask {
  const trigger = next.nodes.find((node) => node.type.startsWith('trigger.'));
  const boardScopes = [
    ...new Set(
      next.nodes
        .map((node) => String(node.config.boardId ?? ''))
        .filter((boardId) => boardId.length > 0)
    ),
  ];
  const stored = { ...next, boardScopes };
  const primaryBoardId =
    preferredBoardId !== undefined
      ? preferredBoardId
      : base.boardId && (boardScopes.length === 0 || boardScopes.includes(base.boardId))
        ? base.boardId
        : (boardScopes[0] ?? '');
  return {
    ...base,
    triggerType: trigger ? triggerTypeForNode(trigger.type) : base.triggerType,
    trigger: trigger?.config ?? base.trigger,
    boardId: primaryBoardId,
    taskTemplate: stored,
  };
}

interface DeviceFlowTaskIssueInput {
  draft: SupervisorAutomaticTask;
  flowIssues: string[];
  fixedBoardAvailable: boolean;
  policies: SupervisorPolicy[];
  boardDisplayName: (scope: string) => string;
}

export function deviceFlowTaskIssues({
  draft,
  flowIssues,
  fixedBoardAvailable,
  policies,
  boardDisplayName,
}: DeviceFlowTaskIssueInput) {
  return [
    ...flowIssues,
    ...(!draft.name.trim() ? ['任务名称不能为空。'] : []),
    ...(!draft.boardId
      ? ['任务尚未选择固定设备。']
      : fixedBoardAvailable
        ? []
        : ['任务原来关联的设备在当前设备库中不存在。']),
    ...(!draft.policyId ? ['任务尚未选择监督策略。'] : []),
    ...(draft.boardId && draft.policyId
      ? policies.some(
          (policy) => policy.id === draft.policyId && policy.boardScope.includes(draft.boardId)
        )
        ? []
        : [`当前监督策略未包含固定设备“${boardDisplayName(draft.boardId)}”。`]
      : []),
  ];
}

export function deviceFlowDraftSaveIssues(issues: string[]) {
  return issues.filter(
    (issue) =>
      issue.startsWith('任务尚未选择固定设备') ||
      issue.startsWith('任务名称不能为空') ||
      issue.startsWith('任务原来关联的设备') ||
      issue.startsWith('任务尚未选择监督策略') ||
      issue.startsWith('当前监督策略未包含固定设备')
  );
}

export function compatiblePolicyIdForBoard(
  policies: SupervisorPolicy[],
  currentPolicyId: number | null,
  boardId: string
) {
  const currentPolicyMatches = policies.some(
    (policy) => policy.id === currentPolicyId && policy.boardScope.includes(boardId)
  );
  const compatiblePolicy = policies.find(
    (policy) => policy.id && policy.boardScope.includes(boardId)
  );
  return currentPolicyMatches ? currentPolicyId : (compatiblePolicy?.id ?? currentPolicyId);
}
