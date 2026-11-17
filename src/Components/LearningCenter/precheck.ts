import { invokeCommand } from '../../Platform/IPC';
import type {
  WorkbenchDevice,
  WorkbenchHost,
  WorkbenchProject,
} from '../../Platform/IPC/contracts/Workbench';
import type { LearningInventory, LearningLab, LearningPrecheckItem } from './types';

interface ReadOnlyInventory {
  devices: WorkbenchDevice[];
  hosts: WorkbenchHost[];
  projects: WorkbenchProject[];
}

export async function readLearningInventory(signal?: AbortSignal): Promise<ReadOnlyInventory> {
  const options = { timeoutMs: 5_000, signal };
  const [devices, hosts, projects] = await Promise.all([
    invokeCommand('workbench_list_devices', undefined, options),
    invokeCommand('workbench_list_hosts', undefined, options),
    invokeCommand('workbench_list_projects', undefined, options),
  ]);
  return { devices, hosts, projects };
}

export function summarizeLearningInventory(inventory: ReadOnlyInventory): LearningInventory {
  return {
    devices: inventory.devices.flatMap((device) =>
      device.id == null ? [] : [{ id: device.id, name: device.name }]
    ),
    hosts: inventory.hosts.flatMap((host) =>
      host.id == null ? [] : [{ id: host.id, name: host.name }]
    ),
    linkedProjectCount: inventory.projects.filter(
      (project) => project.hostId != null && project.deviceId != null
    ).length,
  };
}

export function buildLearningPrecheck(
  appVersion: string,
  isAuthenticated: boolean,
  lab: LearningLab,
  inventory: LearningInventory,
  selectedBoardId: number | null,
  selectedHostId: number | null
): LearningPrecheckItem[] {
  const board = inventory.devices.find((item) => item.id === selectedBoardId);
  const host = inventory.hosts.find((item) => item.id === selectedHostId);
  const packageReady = lab.packageState === 'downloaded';
  const versionReady = compareVersions(appVersion, lab.minLynxVersion) >= 0;

  return [
    {
      id: 'lynx',
      label: 'LYNX 版本',
      value: `v${appVersion}`,
      status: versionReady ? 'pass' : 'warning',
      detail: versionReady
        ? '当前版本满足实验要求。'
        : `实验要求 v${lab.minLynxVersion} 或更高版本。`,
      actionLabel: versionReady ? undefined : '更新',
    },
    {
      id: 'account',
      label: '100ASK 账号',
      value: isAuthenticated ? '已登录' : '公开预览',
      status: isAuthenticated || !lab.entitlementRequired ? 'pass' : 'needs-config',
      detail: isAuthenticated ? '已复用当前 LYNX 登录身份。' : '私有实验需要登录并校验权益。',
    },
    {
      id: 'package',
      label: '实验包',
      value: lab.packageVersion || '未发布',
      status: packageReady ? 'pass' : 'blocked',
      detail: packageReady
        ? '签名实验包已经在本机完成校验和安装。'
        : lab.packageState === 'ready'
          ? '官网已发布实验包，但 LYNX 尚未完成本地下载、验签和安装。'
          : '尚未接入可安装的签名实验包，不能开始执行。',
    },
    {
      id: 'sdk-host',
      label: 'SDK 主机',
      value: host?.name || '未选择',
      status: host ? 'pass' : 'needs-config',
      detail: host
        ? '已按名称固定目标 SDK 主机；本次检查未建立 SSH。'
        : '请选择已保存的 SDK 主机。',
    },
    {
      id: 'sdk',
      label: 'Linux SDK',
      value: lab.sdkRequirement,
      status: inventory.linkedProjectCount > 0 ? 'pass' : 'warning',
      detail:
        inventory.linkedProjectCount > 0
          ? `找到 ${inventory.linkedProjectCount} 个已绑定 SDK 与板卡的工程。`
          : '尚无同时绑定 SDK 主机和开发板的工程。',
    },
    {
      id: 'board',
      label: '开发板',
      value: board?.name || '未选择',
      status: board ? 'pass' : 'needs-config',
      detail: board
        ? '已按设备名称选择；savedBoardId 仅在内部固定作用域。'
        : `请选择与 ${lab.boardProfiles.map((item) => item.displayName).join('、')} 匹配的已保存板卡。`,
    },
    {
      id: 'serial',
      label: '串口',
      value: '执行前确认',
      status: 'warning',
      detail: '只读 inventory 不探测串口在线状态；执行能力接入后由固定板卡绑定确认。',
      actionLabel: '选择串口',
    },
  ];
}

function compareVersions(left: string, right: string): number {
  const normalize = (value: string) =>
    value
      .replace(/^v/, '')
      .split('.')
      .map((part) => Number.parseInt(part, 10) || 0);
  const a = normalize(left);
  const b = normalize(right);
  for (let index = 0; index < Math.max(a.length, b.length); index += 1) {
    const delta = (a[index] || 0) - (b[index] || 0);
    if (delta !== 0) return delta;
  }
  return 0;
}
