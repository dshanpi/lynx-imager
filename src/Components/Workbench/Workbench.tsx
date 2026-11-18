import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { confirm, open } from '@tauri-apps/plugin-dialog';
import { LynxIcon, type LynxIconName } from '../../CoreUI';
import {
  CameraDeviceCandidate,
  LinuxDeviceOverview,
  LocalMcpStatus,
  WorkbenchCredentials,
  WorkbenchDevice,
  WorkbenchFlasherDevice,
  WorkbenchHost,
  WorkbenchProject,
  WorkbenchTaskHistory,
  WorkbenchTunnelStatus,
} from '../../Platform/IPC/Commands';
import { invokeCommand, normalizeIpcError } from '../../Platform/IPC';
import { formatErrorMessage } from '../../Utils';
import BoardConsole from './BoardConsole';
import AdbFileTransfer from './AdbFileTransfer';
import SdkTerminal from './SdkTerminal';
import { BoardThumbnail } from './BoardThumbnail';
import { useSshPresence } from './useSshPresence';
import AdvancedOverview, { recordOverview, type MonitorSample } from './AdvancedOverview';
import { DeviceAiChannel } from '../DeviceAiChannel';
import {
  EMPTY_LINUX_DEVICE as emptyDevice,
  EMPTY_WORKBENCH_HOST as emptyHost,
  EMPTY_WORKBENCH_PROJECT as emptyProject,
  LINUX_CHIP_VENDORS as CHIP_VENDORS,
  formatUsbBinding,
  linuxBoardSpecs,
  linuxVendorMark as vendorMark,
  linuxFlashProtocolLabel as flashProtocolLabel,
  linuxUsedPercent as usedPercent,
  linuxVendorHintFromName as vendorHintFromName,
  linuxVendorLabel as vendorLabel,
  readK230Medium as k230Medium,
  readAllwinnerFlashOptions as allwinnerFlashOptions,
  readStm32DfuOptions as stm32DfuOptions,
  writeK230Args as k230Args,
  writeAllwinnerFlashArgs as allwinnerFlashArgs,
  writeStm32DfuArgs as stm32DfuArgs,
} from './linuxWorkbenchModel';
import { displaySshFingerprint, testAndConfirmSshHost } from './sshHostTrust';
import { localMcpConnection, remoteMcpConnection } from './mcpConnections';
import { buildMcpClientConfig, MCP_CLIENTS, type McpClientId } from './mcpClientProfiles';
import './Workbench.css';
import './AdvancedWorkbench.css';

export interface WorkbenchProps {
  isActive: boolean;
  onOpenSettings?: () => void;
  onOpenRecords?: () => void;
  flashTarget?: { deviceId: number; sequence: number };
  onStatusChange?: (status: { target: string; task: string }) => void;
}

type LinuxDeviceView = 'overview' | 'serial' | 'sdk' | 'flash' | 'ai' | 'workspace';
type HeaderPowerState = 'on' | 'off' | 'unknown';

type SerialPort = Awaited<ReturnType<typeof listSerialPorts>>[number];
type AdbDevice = Awaited<ReturnType<typeof listAdbDevices>>[number];
type UsbDevice = WorkbenchFlasherDevice;
type Ch347Device = Awaited<ReturnType<typeof listCh347Devices>>[number];

type Ch347PowerProfile = {
  type: 'ch347_gpio';
  deviceIndex: number;
  gpio: number;
  activeHigh: boolean;
  roleMode: 'independent' | 'singleRelay' | 'dualPower';
};

const BOARD_SAVE_TIMEOUT_MS = 10_000;
const GATEWAY_RELOAD_TIMEOUT_MS = 5_000;
const CONFIG_REFRESH_TIMEOUT_MS = 5_000;
const ACTIVE_INTERFACE_SCAN_INTERVAL_MS = 5_000;
const BACKGROUND_INTERFACE_SCAN_INTERVAL_MS = 15_000;
const SERIAL_PRESENCE_INTERVAL_MS = 2_500;
const TUNNEL_STATUS_INTERVAL_MS = 3_000;
const GATEWAY_WATCHDOG_INTERVAL_MS = 15_000;

function stableSnapshot<T>(current: T, next: T): T {
  return JSON.stringify(current) === JSON.stringify(next) ? current : next;
}

function isFlashTask(task: WorkbenchTaskHistory) {
  const kind = task.kind.toLowerCase();
  return kind.includes('flash') || kind === 'mainline_boot';
}

function powerStateFromResponse(value: string): HeaderPowerState {
  const match = value.match(/(?:^|[;\s])state=(on|off)(?:[;\s]|$)/i);
  if (match) return match[1].toLowerCase() as HeaderPowerState;
  if (/\bpower\s*=\s*on\b/i.test(value)) return 'on';
  if (/\bpower\s*=\s*off\b/i.test(value)) return 'off';
  return value === 'on' || value === 'off' ? value : 'unknown';
}

function ipcFailureDetail(value: unknown) {
  const error = normalizeIpcError(value);
  const code = error.machineCode || error.name;
  const message = error.message.replace(new RegExp(`^${code}:`), '');
  if (code === 'DEVICE_BINDING_CONFLICT') {
    const owner = message.match(/assigned to board (.+)$/)?.[1];
    if (message.startsWith('serial is already assigned')) {
      return `调试串口已被单板“${owner || '其他单板'}”占用。CH344 多路串口必须为每块板选择不同的 COM 口。`;
    }
    if (message.startsWith('flash USB')) {
      return `烧录接口已被单板“${owner || '其他单板'}”占用，请选择独立接口。`;
    }
    if (message.startsWith('ADB')) {
      return `ADB 接口已被单板“${owner || '其他单板'}”占用，请选择独立设备。`;
    }
  }
  if (code === 'DEVICE_NAME_CONFLICT') {
    return '单板名称已存在，请使用不同名称。';
  }
  return `${code}：${message}`;
}

function linuxOverviewErrorText(value: unknown) {
  const error = normalizeIpcError(value);
  switch (error.machineCode) {
    case 'SERIAL_OVERVIEW_SHELL_NOT_CONFIRMED':
    case 'SERIAL_OVERVIEW_LOGIN_REQUIRED':
    case 'SERIAL_OVERVIEW_PASSWORD_REQUIRED':
      return '等待串口登录：请在“串口终端”完成登录并停留在 Linux Shell 提示符，然后重新刷新。';
    case 'SERIAL_OVERVIEW_RT_SMART_UNSUPPORTED':
      return '当前串口是 RT-Smart 控制台，设备概览不会发送 Linux 探测命令。';
    default:
      return formatErrorMessage(value);
  }
}

async function listSerialPorts() {
  return invokeCommand('serial_list_ports');
}

async function listAdbDevices() {
  return invokeCommand('adb_list_devices');
}

async function listCh347Devices() {
  return invokeCommand('ch347_list_devices');
}

function normalizeCh347Gpio(value: unknown): number {
  const gpio = Number(value);
  return Number.isInteger(gpio) && gpio >= 4 && gpio <= 7 ? gpio : 4;
}

function ch347PowerProfile(board: WorkbenchDevice): Ch347PowerProfile {
  try {
    const value = JSON.parse(board.powerProfileJson) as Partial<Ch347PowerProfile>;
    if (value.type === 'ch347_gpio') {
      return {
        type: 'ch347_gpio',
        deviceIndex: Number(value.deviceIndex ?? 0),
        gpio: normalizeCh347Gpio(value.gpio ?? board.powerChannel),
        activeHigh: value.activeHigh !== false,
        roleMode:
          value.roleMode === 'singleRelay' || value.roleMode === 'dualPower'
            ? value.roleMode
            : 'independent',
      };
    }
  } catch {
    // A malformed/legacy profile is shown as the other adapter and remains untouched.
  }
  return {
    type: 'ch347_gpio',
    deviceIndex: 0,
    gpio: 4,
    activeHigh: true,
    roleMode: 'independent',
  };
}

function withCh347Power(board: WorkbenchDevice, patch: Partial<Ch347PowerProfile>) {
  const profile = { ...ch347PowerProfile(board), ...patch };
  return {
    ...board,
    powerController: `ch347:${profile.deviceIndex}`,
    powerProfileJson: JSON.stringify(profile),
    powerChannel: String(profile.gpio),
    bootChannel: profile.roleMode === 'singleRelay' ? String(profile.gpio) : board.bootChannel,
    resetChannel:
      profile.roleMode === 'singleRelay'
        ? String(profile.gpio)
        : profile.roleMode === 'dualPower'
          ? ''
          : board.resetChannel,
  };
}

function withCh347Device(board: WorkbenchDevice, controller: Ch347Device) {
  return {
    ...withCh347Power(board, { deviceIndex: controller.index }),
    powerController: `lynx-power:${controller.serialNumber || controller.index}`,
    powerControllerVid: controller.vid,
    powerControllerPid: controller.pid,
    powerControllerSerial: controller.serialNumber,
    powerControllerInterfaceNumber: controller.interfaceNumber,
  };
}

function ch347ChannelClaim(board: WorkbenchDevice) {
  const profile = ch347PowerProfile(board);
  let parsedType = '';
  try {
    parsedType = (JSON.parse(board.powerProfileJson) as { type?: string }).type ?? '';
  } catch {
    return null;
  }
  if (parsedType !== 'ch347_gpio') return null;
  const channels = [Number(board.powerChannel || profile.gpio)];
  if (profile.roleMode === 'dualPower') channels.push(Number(board.bootChannel));
  if (profile.roleMode === 'independent') {
    if (board.bootChannel) channels.push(Number(board.bootChannel));
    if (board.resetChannel) channels.push(Number(board.resetChannel));
  }
  return {
    controller: board.powerControllerSerial
      ? `serial:${board.powerControllerSerial}`
      : `index:${profile.deviceIndex}`,
    channels: channels.filter(
      (channel) => Number.isInteger(channel) && channel >= 0 && channel < 8
    ),
  };
}

async function listUsbDevices(
  configs: Array<{ kind: string; path: string }>,
  canScan: () => boolean
) {
  const unique = configs.filter(
    (item, index) =>
      item.kind &&
      configs.findIndex(
        (candidate) => candidate.kind === item.kind && candidate.path === item.path
      ) === index
  );
  const results: PromiseSettledResult<UsbDevice[]>[] = [];
  for (const { kind, path } of unique) {
    if (!canScan()) break;
    try {
      results.push({
        status: 'fulfilled',
        value: await invokeCommand('workbench_scan_flasher_devices', { kind, path }),
      });
    } catch (reason) {
      results.push({ status: 'rejected', reason });
    }
  }
  const devices = results.flatMap((result) => (result.status === 'fulfilled' ? result.value : []));
  return {
    devices: devices.filter(
      (item, index) =>
        devices.findIndex(
          (candidate) => candidate.flasherKind === item.flasherKind && candidate.key === item.key
        ) === index
    ),
    failures: results.flatMap((result, index) =>
      result.status === 'rejected'
        ? [`${unique[index].kind}：${formatErrorMessage(result.reason)}`]
        : []
    ),
  };
}

function slug(value: string) {
  const normalized = value
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, '-')
    .replace(/^-+|-+$/g, '');
  return normalized || 'board';
}

function mcpName(value: string) {
  return `lynx-${slug(value).replace(/^lynx-+/, '')}`;
}

function nextPort(values: number[], current: number | undefined, first: number) {
  return Math.max(first - 1, current ?? first - 1, ...values) + 1;
}

function availablePort(preferred: number, used: number[], first: number) {
  if (preferred >= first && !used.includes(preferred)) return preferred;
  return nextPort(used, undefined, first);
}

function availableMcpName(preferred: string, used: string[]) {
  if (!used.includes(preferred)) return preferred;
  let suffix = 2;
  while (used.includes(`${preferred}-${suffix}`)) suffix += 1;
  return `${preferred}-${suffix}`;
}

function matchingSerialPort(board: WorkbenchDevice, ports: SerialPort[]) {
  if (!board.serialPort && !board.serialNumber && !board.serialVid) return undefined;
  if (board.serialNumber) {
    const matches = ports.filter(
      (port) =>
        port.serialNumber === board.serialNumber &&
        (!board.serialVid || port.vid === board.serialVid) &&
        (!board.serialPid || port.pid === board.serialPid)
    );
    const candidates =
      board.serialInterfaceNumber != null
        ? matches.filter((port) => port.interfaceNumber === board.serialInterfaceNumber)
        : matches;
    return (
      candidates.find((port) => port.name === board.serialPort) ??
      (candidates.length === 1 ? candidates[0] : undefined)
    );
  }
  if (board.serialVid || board.serialPid) {
    const matches = ports.filter(
      (port) =>
        (!board.serialVid || port.vid === board.serialVid) &&
        (!board.serialPid || port.pid === board.serialPid) &&
        (board.serialInterfaceNumber == null ||
          port.interfaceNumber === board.serialInterfaceNumber)
    );
    return (
      matches.find((port) => port.name === board.serialPort) ??
      (matches.length === 1 ? matches[0] : undefined)
    );
  }
  return ports.find((port) => port.name === board.serialPort);
}

function isSerialOnline(board: WorkbenchDevice, ports: SerialPort[]) {
  return !!matchingSerialPort(board, ports);
}

// Virtual ports may be absent from USB enumeration while LYNX has an open serial handle.
function useBoundSerialSession(active: boolean, deviceId: number | null, port: string) {
  const [seen, setSeen] = useState<{ deviceId: number; port: string; open: boolean } | null>(null);
  useEffect(() => {
    setSeen(null);
    if (!active || deviceId == null || !port) return;
    const owner = deviceId;
    let current = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const refresh = async () => {
      const open = await invokeCommand('serial_is_open', { port }).catch(() => false);
      if (!current) return;
      setSeen((previous) =>
        previous?.deviceId === owner && previous.port === port && previous.open === open
          ? previous
          : { deviceId: owner, port, open }
      );
      timer = setTimeout(refresh, SERIAL_PRESENCE_INTERVAL_MS);
    };
    void refresh();
    return () => {
      current = false;
      clearTimeout(timer);
    };
  }, [active, deviceId, port]);
  return !!(active && seen?.deviceId === deviceId && seen?.port === port && seen.open);
}

function sameSerialBinding(left: WorkbenchDevice, right: WorkbenchDevice) {
  if (left.serialNumber && right.serialNumber) {
    if (
      left.serialNumber === right.serialNumber &&
      (!left.serialVid || !right.serialVid || left.serialVid === right.serialVid) &&
      (!left.serialPid || !right.serialPid || left.serialPid === right.serialPid)
    ) {
      if (
        left.serialPort &&
        right.serialPort &&
        left.serialPort.toLowerCase() !== right.serialPort.toLowerCase()
      ) {
        return false;
      }
      if (left.serialInterfaceNumber != null && right.serialInterfaceNumber != null) {
        return left.serialInterfaceNumber === right.serialInterfaceNumber;
      }
      return !!left.serialPort && left.serialPort === right.serialPort;
    }
    return false;
  }
  return !!left.serialPort && left.serialPort === right.serialPort;
}

function isUsbOnline(board: WorkbenchDevice, interfaces: UsbDevice[]) {
  return (
    !!board.usbBinding &&
    interfaces.some(
      (item) => item.flasherKind === board.flasherKind && item.key === board.usbBinding
    )
  );
}

function isAdbOnline(board: WorkbenchDevice, adbDevices: AdbDevice[]) {
  return (
    !!board.adbSerial &&
    adbDevices.some((item) => item.serial === board.adbSerial && item.state === 'device')
  );
}

export const Workbench: React.FC<WorkbenchProps> = ({
  isActive,
  onOpenRecords,
  onOpenSettings,
  onStatusChange,
  flashTarget,
}) => {
  const [devices, setDevices] = useState<WorkbenchDevice[]>([]);
  const [hosts, setHosts] = useState<WorkbenchHost[]>([]);
  const [projects, setProjects] = useState<WorkbenchProject[]>([]);
  const [device, setDevice] = useState<WorkbenchDevice>(emptyDevice);
  const [host, setHost] = useState<WorkbenchHost>(emptyHost);
  const [project, setProject] = useState<WorkbenchProject>(emptyProject);
  const [mcpClients, setMcpClients] = useState<Record<string, McpClientId>>({});
  const [serialPorts, setSerialPorts] = useState<SerialPort[]>([]);
  const [adbDevices, setAdbDevices] = useState<AdbDevice[]>([]);
  const [usbDevices, setUsbDevices] = useState<UsbDevice[]>([]);
  const [ch347Devices, setCh347Devices] = useState<Ch347Device[]>([]);
  const [editorSerialPorts, setEditorSerialPorts] = useState<SerialPort[]>([]);
  const [editorAdbDevices, setEditorAdbDevices] = useState<AdbDevice[]>([]);
  const [editorUsbDevices, setEditorUsbDevices] = useState<UsbDevice[]>([]);
  const [editorCh347Devices, setEditorCh347Devices] = useState<Ch347Device[]>([]);
  const [editorCameraDevices, setEditorCameraDevices] = useState<CameraDeviceCandidate[]>([]);
  const [tunnelStatuses, setTunnelStatuses] = useState<WorkbenchTunnelStatus[]>([]);
  const [taskHistory, setTaskHistory] = useState<WorkbenchTaskHistory[]>([]);
  const [linuxOverview, setLinuxOverview] = useState<LinuxDeviceOverview | null>(null);
  const [linuxOverviewLoading, setLinuxOverviewLoading] = useState(false);
  const [linuxOverviewError, setLinuxOverviewError] = useState('');
  const [credentials, setCredentials] = useState<WorkbenchCredentials>({});
  const [serialLoginPassword, setSerialLoginPassword] = useState('');
  const [serialPasswordConfigured, setSerialPasswordConfigured] = useState(false);
  const [deleteSerialPassword, setDeleteSerialPassword] = useState(false);
  const [message, setMessage] = useState('');
  const [error, setError] = useState('');
  const [settingsOpen, setSettingsOpen] = useState(false);
  const settingsSessionRef = useRef(0);
  const channelSaveOwnerRef = useRef<{
    session: number;
    deviceId: number;
    projectId: number;
  } | null>(null);
  const [localMcpStatus, setLocalMcpStatus] = useState<LocalMcpStatus | null>(null);
  const [localMcpError, setLocalMcpError] = useState('');
  const [remoteMcpError, setRemoteMcpError] = useState('');
  const [activeView, setActiveView] = useState<LinuxDeviceView>('overview');
  const [firmwareHandoff, setFirmwareHandoff] = useState<{
    deviceId: number;
    candidateId: string;
  } | null>(null);
  useEffect(() => {
    const handoff = (event: Event) => {
      const d = (event as CustomEvent<{ deviceId: number; candidateId: string }>).detail;
      if (isActive && d?.deviceId === device.id && typeof d.candidateId === 'string') {
        setFirmwareHandoff(d);
        setActiveView('flash');
      }
    };
    const serial = (event: Event) => {
      if (isActive && (event as CustomEvent<{ deviceId: number }>).detail?.deviceId === device.id)
        setActiveView('serial');
    };
    window.addEventListener('lynx:firmware-candidate', handoff);
    window.addEventListener('lynx:firmware-serial', serial);
    return () => {
      window.removeEventListener('lynx:firmware-candidate', handoff);
      window.removeEventListener('lynx:firmware-serial', serial);
    };
  }, [isActive, device.id]);
  const handledFlashRequest = useRef<number | null>(null);
  const [navigationError, setNavigationError] = useState('');
  useEffect(() => {
    if (!isActive || !flashTarget || handledFlashRequest.current === flashTarget.sequence) return;
    let current = true;
    setNavigationError('');
    void invokeCommand('workbench_list_devices', undefined, { timeoutMs: 8000 })
      .then((saved) => {
        if (!current) return;
        handledFlashRequest.current = flashTarget.sequence;
        const target = saved.find((item) => item.id === flashTarget.deviceId);
        if (!target) {
          setNavigationError(`未找到设备 #${flashTarget.deviceId}，请检查保存的单板配置。`);
          return;
        }
        creatingBoardRef.current = false;
        setDevices(saved);
        setDevice(target);
        setActiveView('flash');
      })
      .catch((cause) => {
        if (!current) return;
        handledFlashRequest.current = flashTarget.sequence;
        setNavigationError(
          `无法读取烧录目标 #${flashTarget.deviceId}：${formatErrorMessage(cause)}`
        );
      });
    return () => {
      current = false;
    };
  }, [isActive, flashTarget]);
  const [sshOpenRequestId, setSshOpenRequestId] = useState(0);
  const [workspacePrimarySshOnline, setWorkspacePrimarySshOnline] = useState(false);

  useEffect(() => {
    const openDiagnosticHandoff = () => {
      if (!isActive) return;
      setActiveView('sdk');
      setMessage('诊断提示已复制；请选择或启动 Codex、Claude、DeepSeek 会话后粘贴。');
    };
    window.addEventListener('lynx-diagnostic-ai-handoff', openDiagnosticHandoff);
    return () => window.removeEventListener('lynx-diagnostic-ai-handoff', openDiagnosticHandoff);
  }, [isActive]);
  const viewBeforeWorkspaceRef = useRef<Exclude<LinuxDeviceView, 'workspace'>>('overview');
  const handlePrimaryWorkspaceOnline = useCallback((online: boolean) => {
    setWorkspacePrimarySshOnline((current) => (current === online ? current : online));
  }, []);
  const handleSshOpenRequest = useCallback((requestId: number) => {
    setSshOpenRequestId((current) => (current === requestId ? 0 : current));
  }, []);
  const [boardSearch, setBoardSearch] = useState('');
  const [powerBusy, setPowerBusy] = useState(false);
  const [powerResult, setPowerResult] = useState('');
  const [powerState, setPowerState] = useState<HeaderPowerState>('unknown');
  const [savingBoard, setSavingBoard] = useState(false);
  const [boardSaveFeedback, setBoardSaveFeedback] = useState('');
  const [savingChannel, setSavingChannel] = useState(false);
  const [testingHost, setTestingHost] = useState(false);
  const testingHostRef = useRef(false);
  const hostSelectionRef = useRef({ host, projectId: project.id, deviceId: device.id });
  useEffect(() => {
    hostSelectionRef.current = { host, projectId: project.id, deviceId: device.id };
  }, [host, project.id, device.id]);
  const [channelSaveFeedback, setChannelSaveFeedback] = useState('');
  const [channelSaveProjectId, setChannelSaveProjectId] = useState<number | null>(null);
  const [initialSelectedDeviceId] = useState<number | null>(() => {
    const saved = Number(window.localStorage.getItem('lynx.workbench.selectedDeviceId'));
    return Number.isInteger(saved) && saved > 0 ? saved : null;
  });
  const selectedDeviceIdRef = useRef(initialSelectedDeviceId);
  const overviewDeviceIdRef = useRef<number | null>(device.id);
  const [monitorSamples, setMonitorSamples] = useState<Record<number, MonitorSample[]>>({});
  const overviewRequests = useRef(new Set<number>());
  overviewDeviceIdRef.current = device.id;
  const scanningRef = useRef(false);
  const isActiveRef = useRef(isActive);
  isActiveRef.current = isActive;
  const [scanningInterfaces, setScanningInterfaces] = useState(false);
  const [interfaceScanFeedback, setInterfaceScanFeedback] = useState('');
  const creatingBoardRef = useRef(false);
  const configInitializedRef = useRef(false);
  const flashRunningRef = useRef(false);
  flashRunningRef.current = taskHistory.some(
    (task) => task.status === 'running' && isFlashTask(task)
  );
  const settingsOpenRef = useRef(settingsOpen);
  settingsOpenRef.current = settingsOpen;
  const flasherConfigs = useMemo(() => {
    return [
      ...devices.map((item) => ({
        kind: item.flasherKind,
        path: item.flasherPath,
      })),
      { kind: device.flasherKind, path: device.flasherPath },
    ].filter((item) => item.kind);
  }, [device.flasherKind, device.flasherPath, devices]);
  const flasherConfigsRef = useRef(flasherConfigs);
  flasherConfigsRef.current = flasherConfigs;
  const selectedEditorUsbDevices = useMemo(
    () => editorUsbDevices.filter((item) => item.flasherKind === device.flasherKind),
    [device.flasherKind, editorUsbDevices]
  );
  const detectedUsbSummary = useMemo(
    () =>
      CHIP_VENDORS.map((platform) => ({
        protocol: platform.protocol,
        count: editorUsbDevices.filter((item) => item.flasherKind === platform.flasher).length,
      }))
        .filter((item) => item.count > 0)
        .map((item) => `${item.protocol} ${item.count}`)
        .join('、'),
    [editorUsbDevices]
  );

  const openSettings = () => {
    settingsSessionRef.current += 1;
    setLocalMcpStatus(null);
    setLocalMcpError('');
    setEditorSerialPorts(serialPorts);
    setEditorAdbDevices(adbDevices);
    setEditorUsbDevices(usbDevices);
    setEditorCh347Devices(ch347Devices);
    setEditorCameraDevices([]);
    setSerialLoginPassword('');
    setDeleteSerialPassword(false);
    setSettingsOpen(true);
  };

  useEffect(() => {
    if (!settingsOpen || !device.id) {
      setSerialPasswordConfigured(false);
      return;
    }
    let active = true;
    void invokeCommand('workbench_serial_login_password_status', { deviceId: device.id })
      .then((configured) => {
        if (active) setSerialPasswordConfigured(configured);
      })
      .catch(() => {
        if (active) setSerialPasswordConfigured(false);
      });
    return () => {
      active = false;
    };
  }, [device.id, settingsOpen]);

  const refreshConfig = useCallback(async (timeoutMs?: number) => {
    const [nextDevices, nextHosts, nextProjects] = await Promise.all([
      invokeCommand('workbench_list_devices', undefined, { timeoutMs }),
      invokeCommand('workbench_list_hosts', undefined, { timeoutMs }),
      invokeCommand('workbench_list_projects', undefined, { timeoutMs }),
    ]);
    setDevices(nextDevices);
    setHosts(nextHosts);
    setProjects(nextProjects);
    const firstLoad = !configInitializedRef.current;
    configInitializedRef.current = true;
    setDevice((current) => {
      if (creatingBoardRef.current) return current;
      if (current.id) {
        return nextDevices.find((item) => item.id === current.id) ?? current;
      }
      return firstLoad
        ? (nextDevices.find((item) => item.id === selectedDeviceIdRef.current) ??
            nextDevices[0] ??
            current)
        : current;
    });
    setHost((current) => {
      if (current.id) return nextHosts.find((item) => item.id === current.id) ?? current;
      if (current.name || current.address || current.username) return current;
      return firstLoad ? (nextHosts[0] ?? current) : current;
    });
  }, []);

  useEffect(() => {
    if (!device.id) return;
    selectedDeviceIdRef.current = device.id;
    window.localStorage.setItem('lynx.workbench.selectedDeviceId', String(device.id));
  }, [device.id]);

  const scanInterfaces = useCallback(async (updateEditor = false) => {
    if (!isActiveRef.current || scanningRef.current) return;
    // Keep scanners sequential: they share the USB discovery plane with flashers.
    if (flashRunningRef.current) {
      if (updateEditor) setInterfaceScanFeedback('正在烧录，接口扫描暂停；任务结束后可重新扫描。');
      return;
    }
    scanningRef.current = true;
    setScanningInterfaces(true);
    setInterfaceScanFeedback('正在扫描当前接口…');
    const failures: string[] = [];
    const updateSettings = () => updateEditor || settingsOpenRef.current;
    async function scan<T>(name: string, load: () => Promise<T>, publish: (value: T) => void) {
      if (!isActiveRef.current || flashRunningRef.current) return;
      try {
        const value = await load();
        if (isActiveRef.current) publish(value);
      } catch (error) {
        failures.push(`${name}：${formatErrorMessage(error)}`);
      }
    }
    try {
      await scan('串口', listSerialPorts, (ports) => {
        setSerialPorts((current) => stableSnapshot(current, ports));
        if (updateSettings()) setEditorSerialPorts((current) => stableSnapshot(current, ports));
      });
      await scan('ADB', listAdbDevices, (devices) => {
        setAdbDevices((current) => stableSnapshot(current, devices));
        if (updateSettings()) setEditorAdbDevices((current) => stableSnapshot(current, devices));
      });
      await scan(
        'USB 烧录接口',
        () =>
          listUsbDevices(
            flasherConfigsRef.current,
            () => isActiveRef.current && !flashRunningRef.current
          ),
        (result) => {
          setUsbDevices((current) => stableSnapshot(current, result.devices));
          if (updateSettings())
            setEditorUsbDevices((current) => stableSnapshot(current, result.devices));
          failures.push(...result.failures);
        }
      );
      await scan('电源控制器', listCh347Devices, (devices) => {
        setCh347Devices((current) => stableSnapshot(current, devices));
        if (updateSettings()) setEditorCh347Devices((current) => stableSnapshot(current, devices));
      });
      if (updateSettings()) {
        await scan(
          '摄像头',
          () => invokeCommand('workbench_list_cameras'),
          (cameras) => {
            setEditorCameraDevices((current) => stableSnapshot(current, cameras));
          }
        );
      }
      if (isActiveRef.current) {
        setInterfaceScanFeedback(
          flashRunningRef.current
            ? '正在烧录，后续接口扫描已暂停。'
            : failures.length
              ? `部分接口扫描失败，其他结果已更新：${failures.join('；')}`
              : '扫描完成。换线或换口后，请选择当前接口并保存单板。'
        );
      }
    } finally {
      scanningRef.current = false;
      setScanningInterfaces(false);
    }
  }, []);

  useEffect(() => {
    if (!isActive) return;
    refreshConfig().catch((value) => setError(formatErrorMessage(value)));
  }, [isActive, refreshConfig]);

  useEffect(() => {
    if (!isActive) return;
    scanInterfaces().catch((value) => setError(formatErrorMessage(value)));
    const timer = window.setInterval(
      () => {
        scanInterfaces().catch(() => undefined);
      },
      settingsOpen ? ACTIVE_INTERFACE_SCAN_INTERVAL_MS : BACKGROUND_INTERFACE_SCAN_INTERVAL_MS
    );
    return () => window.clearInterval(timer);
  }, [isActive, scanInterfaces, settingsOpen]);

  useEffect(() => {
    if (!isActive) return;
    const refreshTasks = () =>
      invokeCommand('workbench_list_tasks', { limit: 50 })
        .then((tasks) => setTaskHistory((current) => stableSnapshot(current, tasks)))
        .catch(() => undefined);
    refreshTasks();
    const timer = window.setInterval(refreshTasks, 5000);
    return () => window.clearInterval(timer);
  }, [isActive]);

  useEffect(() => {
    if (!isActive || !projects.length) return;
    let cancelled = false;
    const ensureGateways = () =>
      Promise.all(
        projects.map((item) =>
          invokeCommand('gateway_start', {
            port: item.localGatewayPort,
            projectId: item.id,
            deviceId: item.deviceId,
          })
        )
      );
    const restore = async () => {
      await ensureGateways();
      const running = await invokeCommand('workbench_list_tunnel_statuses');
      const activeProjectIds = new Set(
        running
          .filter((item) =>
            ['connecting', 'verifying', 'connected', 'host_key_required'].includes(item.state)
          )
          .map((item) => item.projectId)
      );
      await Promise.allSettled(
        projects
          .filter((item) => item.id && !activeProjectIds.has(item.id))
          .map((item) => invokeCommand('workbench_start_tunnel', { projectId: item.id! }))
      );
      const statuses = await invokeCommand('workbench_list_tunnel_statuses');
      if (!cancelled) {
        setTunnelStatuses((current) => stableSnapshot(current, statuses));
        setRemoteMcpError('');
      }
    };
    restore().catch((value) => {
      if (!cancelled) {
        setError(formatErrorMessage(value));
        setTunnelStatuses([]);
        setRemoteMcpError(formatErrorMessage(value));
      }
    });
    const timer = window.setInterval(() => {
      invokeCommand('workbench_list_tunnel_statuses')
        .then((statuses) => {
          if (!cancelled) {
            setTunnelStatuses((current) => stableSnapshot(current, statuses));
            setRemoteMcpError('');
          }
        })
        .catch((value) => {
          if (!cancelled) {
            setTunnelStatuses([]);
            setRemoteMcpError(formatErrorMessage(value));
          }
        });
    }, TUNNEL_STATUS_INTERVAL_MS);
    const gatewayWatchdog = window.setInterval(() => {
      ensureGateways().catch(() => undefined);
    }, GATEWAY_WATCHDOG_INTERVAL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
      window.clearInterval(gatewayWatchdog);
    };
  }, [isActive, projects]);

  useEffect(() => {
    if (!isActive || !settingsOpen || !project.id || !device.id) return;
    let cancelled = false;
    const projectId = project.id;
    const deviceId = device.id;
    let sequence = 0;
    setLocalMcpStatus(null);
    setLocalMcpError('');
    const refresh = async () => {
      const request = ++sequence;
      try {
        const status = await invokeCommand('gateway_channel_status', { projectId, deviceId });
        if (!cancelled && request === sequence) {
          setLocalMcpStatus(status);
          setLocalMcpError('');
        }
      } catch (value) {
        if (!cancelled && request === sequence) {
          setLocalMcpStatus(null);
          setLocalMcpError(formatErrorMessage(value));
        }
      }
    };
    void refresh();
    const timer = window.setInterval(() => void refresh(), TUNNEL_STATUS_INTERVAL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [isActive, settingsOpen, project.id, device.id]);

  useEffect(() => {
    if (!channelSaveProjectId || savingChannel) return;
    const owner = channelSaveOwnerRef.current;
    if (
      !owner ||
      owner.session !== settingsSessionRef.current ||
      owner.deviceId !== device.id ||
      owner.projectId !== project.id
    ) {
      setChannelSaveProjectId(null);
      return;
    }
    const status = tunnelStatuses.find((item) => item.projectId === channelSaveProjectId);
    if (status?.state === 'connected') {
      setChannelSaveFeedback('保存成功，SDK 通道已连接并通过远端访问验证。');
      setChannelSaveProjectId(null);
      setSettingsOpen(false);
    } else if (status?.state === 'stopped') {
      setChannelSaveFeedback(`通道已保存，但连接失败：${status.message}`);
      setChannelSaveProjectId(null);
    }
  }, [channelSaveProjectId, tunnelStatuses, savingChannel, device.id, project.id]);

  useEffect(() => {
    if (!device.id) return;
    const channel = projects.find((item) => item.deviceId === device.id);
    setProject((current) => {
      // Refreshing host/device lists must not erase an SDK path or another
      // unsaved field that the user is currently editing for this board.
      if (current.deviceId === device.id) return current;
      if (channel) return { ...channel, mcpServerName: mcpName(channel.name) };
      return {
        ...emptyProject,
        deviceId: device.id,
        hostId: 0,
        name: `${device.name || '单板'} SDK`,
        mcpServerName: mcpName(`${device.name || '单板'} SDK`),
        remoteForwardPort: nextPort(
          projects.map((item) => item.remoteForwardPort),
          current.deviceId && !current.id ? current.remoteForwardPort : undefined,
          18765
        ),
        localGatewayPort: nextPort(
          projects.map((item) => item.localGatewayPort),
          current.deviceId && !current.id ? current.localGatewayPort : undefined,
          8765
        ),
      };
    });
    if (channel) {
      const linkedHost = hosts.find((item) => item.id === channel.hostId);
      if (linkedHost) setHost(linkedHost);
    } else {
      setHost({ ...emptyHost });
      setCredentials({});
    }
  }, [device.id, device.name, hosts, projects]);

  const boardChannels = useMemo(
    () => projects.filter((item) => item.deviceId === device.id),
    [device.id, projects]
  );

  const matchingHost = useMemo(
    () =>
      hosts.find(
        (item) =>
          item.address.trim().toLowerCase() === host.address.trim().toLowerCase() &&
          item.port === host.port &&
          item.username.trim() === host.username.trim()
      ),
    [host.address, host.port, host.username, hosts]
  );
  const effectiveHostId = host.id ?? matchingHost?.id ?? project.hostId;

  const nameConflict = useMemo(
    () =>
      projects.some(
        (item) =>
          item.id !== project.id &&
          item.hostId === effectiveHostId &&
          item.mcpServerName === project.mcpServerName
      ),
    [effectiveHostId, project.id, project.mcpServerName, projects]
  );

  const tunnelStatus = tunnelStatuses.find((item) => item.projectId === project.id);
  const remoteEndpoint = remoteMcpConnection(tunnelStatus, project.id).endpoint;
  const localConnection = localMcpConnection(
    localMcpStatus,
    project.id,
    device.id,
    project.mcpServerName
  );
  const currentLocalStatus =
    localMcpStatus?.projectId === project.id && localMcpStatus?.deviceId === device.id
      ? localMcpStatus
      : null;
  const activeMcpName = project.mcpServerName || 'lynx-board';
  const mcpClientScope = `${device.id ?? 'new'}:${project.id ?? 'new'}`;
  const mcpClient = mcpClients[mcpClientScope] ?? project.aiProvider;
  const localShell = /Windows/i.test(navigator.userAgent) ? 'powershell' : 'posix';
  const localClientConfig = buildMcpClientConfig(
    mcpClient,
    localConnection.endpoint,
    activeMcpName,
    localShell
  );
  const remoteClientConfig = buildMcpClientConfig(
    mcpClient,
    remoteEndpoint,
    activeMcpName,
    'posix'
  );
  const configOnly = localClientConfig.kind === 'config';
  const genericLocalConfig = buildMcpClientConfig(
    'custom',
    localConnection.endpoint,
    activeMcpName,
    localShell
  ).value;
  const genericRemoteConfig = buildMcpClientConfig(
    'custom',
    remoteEndpoint,
    activeMcpName,
    'posix'
  ).value;

  function updateDeviceBinding(patch: Partial<WorkbenchDevice>) {
    setDevice((current) => ({ ...current, ...patch }));
    setError('');
    setBoardSaveFeedback('');
  }

  const selectSerial = (name: string) => {
    const selected = editorSerialPorts.find((item) => item.name === name);
    updateDeviceBinding({
      serialPort: name,
      serialVid: selected?.vid ?? 0,
      serialPid: selected?.pid ?? 0,
      serialNumber: selected?.serialNumber ?? '',
      serialInterfaceNumber: selected?.interfaceNumber ?? null,
    });
  };

  const adbBoundBoard = (serial: string) => devices.find((item) => item.adbSerial === serial);
  const adbBindingOwner = (serial: string) => {
    const board = adbBoundBoard(serial);
    return board?.id !== device.id ? board : undefined;
  };
  const selectedAdbOwner = device.adbSerial ? adbBindingOwner(device.adbSerial) : undefined;

  const saveBoard = async () => {
    if (savingBoard) return;
    const settingsSession = settingsSessionRef.current;
    const rejectSave = (detail: string) => {
      setError(detail);
      setBoardSaveFeedback(`保存失败：${detail}`);
    };
    if (!device.name.trim()) {
      rejectSave('请先填写单板名称。');
      return;
    }
    if (
      device.serialAutoLogin &&
      device.serialLoginMode === 'password' &&
      !serialLoginPassword &&
      (!serialPasswordConfigured || deleteSerialPassword)
    ) {
      rejectSave('已开启串口自动登录，请输入登录密码并保存到系统密钥环。');
      return;
    }
    const vendorHint = vendorHintFromName(device.name);
    if (vendorHint && vendorHint !== device.chipVendor) {
      rejectSave(
        `单板名称识别为“${vendorLabel(vendorHint)}”，但当前厂家选择为“${vendorLabel(device.chipVendor)}”。请先修正厂家，LYNX 不会保存可能误烧的配置。`
      );
      return;
    }
    if (device.chipVendor === 'allwinner') {
      const allwinner = allwinnerFlashOptions(device.flasherArgs);
      if (allwinner.route !== 'vendor_image' && !allwinner.loaderPath.trim()) {
        rejectSave('主线烧录路线必须选择与当前板卡、DRAM 和存储完全匹配的 RAM FES Loader。');
        return;
      }
    }
    try {
      const powerProfile = JSON.parse(device.powerProfileJson) as {
        type?: string;
        roleMode?: string;
      };
      if (powerProfile.type === 'ch347_gpio') {
        const channels = [device.powerChannel, device.bootChannel, device.resetChannel].filter(
          (channel) => channel !== ''
        );
        if (powerProfile.roleMode === 'independent' && new Set(channels).size !== channels.length) {
          rejectSave('Lynx Power 的电源、Boot、Reset 必须使用不同 GPIO，已拒绝保存。');
          return;
        }
        if (
          powerProfile.roleMode === 'dualPower' &&
          (!device.bootChannel || device.powerChannel === device.bootChannel)
        ) {
          rejectSave('双路 USB 协同供电必须选择两个不同 GPIO，已拒绝保存。');
          return;
        }
      }
    } catch {
      // The backend reports full profile errors for legacy/custom controller JSON.
    }
    const bindingConflict = devices.find(
      (item) =>
        item.id !== device.id &&
        ((sameSerialBinding(device, item) &&
          (device.serialPort || device.serialNumber || device.serialVid)) ||
          (device.usbBinding && item.usbBinding === device.usbBinding) ||
          (device.adbSerial && item.adbSerial === device.adbSerial))
    );
    if (bindingConflict) {
      const detail =
        device.adbSerial && bindingConflict.adbSerial === device.adbSerial
          ? `ADB ${device.adbSerial} 已绑定到单板“${bindingConflict.name}”，不能重复绑定。`
          : `接口已被单板“${bindingConflict.name}”占用。多设备开发时，每块板必须绑定独立串口、烧录接口和 ADB。`;
      rejectSave(detail);
      return;
    }
    const powerClaim = ch347ChannelClaim(device);
    const powerConflict = powerClaim
      ? devices.find((item) => {
          if (item.id === device.id) return false;
          const other = ch347ChannelClaim(item);
          return (
            other?.controller === powerClaim.controller &&
            powerClaim.channels.some((channel) => other.channels.includes(channel))
          );
        })
      : undefined;
    if (powerClaim && powerConflict) {
      const other = ch347ChannelClaim(powerConflict);
      const channel = powerClaim.channels.find((value) => other?.channels.includes(value));
      rejectSave(`Lynx Power GPIO${channel} 已绑定到单板“${powerConflict.name}”，请选择独立通道。`);
      return;
    }
    const linkedProjectIds = new Set(
      projects.filter((item) => item.deviceId === device.id).map((item) => item.id)
    );
    const runningTask = taskHistory.find(
      (task) =>
        task.status === 'running' &&
        !['build', 'transfer', 'ai'].includes(task.kind) &&
        (task.deviceId === device.id ||
          (task.projectId != null && linkedProjectIds.has(task.projectId)))
    );
    if (powerBusy || runningTask) {
      const operation = powerBusy
        ? '电源操作'
        : `${runningTask?.kind || '设备任务'}（${runningTask?.phase || '运行中'}）`;
      setError(`当前单板正在执行${operation}，暂时不能修改接口配置。请等待任务结束后重试。`);
      setBoardSaveFeedback('当前设备使用中，未保存');
      return;
    }
    setSavingBoard(true);
    setBoardSaveFeedback('正在保存单板配置…');
    setError('');
    try {
      const isNewBoard = !device.id;
      let id: number;
      try {
        id = await invokeCommand(
          'workbench_save_device',
          { device },
          { timeoutMs: BOARD_SAVE_TIMEOUT_MS }
        );
      } catch (value) {
        const detail = ipcFailureDetail(value);
        setError(`保存失败：${detail}`);
        setBoardSaveFeedback(`保存失败：${detail}`);
        return;
      }
      const saved = { ...device, id };
      creatingBoardRef.current = false;
      setDevice(saved);
      let serialCredentialFailure = '';
      if (serialLoginPassword || deleteSerialPassword) {
        try {
          await invokeCommand('workbench_save_serial_login_password', {
            deviceId: id,
            password: deleteSerialPassword ? null : serialLoginPassword,
          });
          setSerialPasswordConfigured(!deleteSerialPassword);
          setSerialLoginPassword('');
          setDeleteSerialPassword(false);
        } catch (value) {
          serialCredentialFailure = ipcFailureDetail(value);
        }
      }
      if (isNewBoard) {
        const channelName = `${saved.name} SDK`;
        setProject((current) => ({
          ...current,
          id: null,
          deviceId: id,
          hostId: current.hostId,
          name: channelName,
          mcpServerName: mcpName(channelName),
        }));
      }
      const linkedProjects = projects.filter((item) => item.deviceId === id);
      let gatewayFailure = '';
      if (linkedProjects.length) {
        setBoardSaveFeedback('单板已保存，正在重载 Gateway…');
        const gatewayResults = await Promise.allSettled(
          linkedProjects.map((item) =>
            invokeCommand(
              'gateway_start',
              {
                port: item.localGatewayPort,
                projectId: item.id,
                deviceId: id,
              },
              { timeoutMs: GATEWAY_RELOAD_TIMEOUT_MS }
            )
          )
        );
        const failedGateways = gatewayResults.flatMap((result) =>
          result.status === 'rejected' ? [ipcFailureDetail(result.reason)] : []
        );
        if (failedGateways.length) {
          gatewayFailure = `${failedGateways.length}/${linkedProjects.length} 个通道失败：${failedGateways.join('；')}`;
        }
      }

      let refreshFailure = '';
      try {
        await refreshConfig(CONFIG_REFRESH_TIMEOUT_MS);
      } catch (value) {
        refreshFailure = ipcFailureDetail(value);
      }

      if (gatewayFailure) {
        const detail = `单板已保存，但 Gateway 重载失败：${gatewayFailure}`;
        setMessage(detail);
        setError(detail);
        setBoardSaveFeedback('单板已保存，但 Gateway 重载失败');
        return;
      }
      if (serialCredentialFailure) {
        const detail = `单板已保存，但串口登录密码未能写入系统密钥环：${serialCredentialFailure}`;
        setMessage(detail);
        setError(detail);
        setBoardSaveFeedback('单板已保存，但串口密码保存失败');
        return;
      }
      if (refreshFailure) {
        const detail = `单板已保存，但配置刷新失败：${refreshFailure}`;
        setMessage(detail);
        setError(detail);
        setBoardSaveFeedback('单板已保存，但配置刷新失败');
        return;
      }

      setMessage(
        linkedProjects.length
          ? `单板“${saved.name}”已保存，Gateway 已重载。`
          : `单板“${saved.name}”已保存。`
      );
      setBoardSaveFeedback('保存成功');
      if (settingsSessionRef.current === settingsSession) setSettingsOpen(false);
    } finally {
      setSavingBoard(false);
    }
  };

  const testHost = async () => {
    if (testingHostRef.current || savingChannel) return;
    testingHostRef.current = true;
    setTestingHost(true);
    setError('');
    try {
      const hostForTest = matchingHost
        ? {
            ...matchingHost,
            name: host.name,
            address: host.address,
            port: host.port,
            username: host.username,
          }
        : host;
      const verified = await testAndConfirmSshHost(hostForTest, {
        test: (candidate) =>
          invokeCommand('workbench_test_ssh', {
            host: candidate,
            credentials,
            allowUntrusted: false,
          }),
        confirm: (message) =>
          confirm(message, {
            title: '核验 SSH 主机指纹',
            kind: 'warning',
            okLabel: '已核实，信任此指纹',
            cancelLabel: '取消',
          }),
        describeError: formatErrorMessage,
        isCurrent: () =>
          hostSelectionRef.current.host === host &&
          hostSelectionRef.current.projectId === project.id &&
          hostSelectionRef.current.deviceId === device.id,
        sharedChannels: projects.filter((item) => item.hostId === hostForTest.id).length,
      });
      if (!verified) {
        setMessage('已取消主机信任，原配置保持不变。');
        return;
      }
      const { host: trustedHost, result } = verified;
      const hostId = await invokeCommand('workbench_save_host', {
        host: trustedHost,
        connectionTestSessionId: result.auditSessionId,
      });
      setHost({ ...trustedHost, id: hostId });
      setProject({ ...project, hostId });
      if (credentials.password || credentials.keyPassphrase) {
        await invokeCommand('workbench_save_credentials', { hostId, credentials });
      }
      setMessage(
        `SSH 指纹已核验并保存：${result.fingerprint}。请保存通道以使用更新后的主机配置连接。`
      );
      await refreshConfig();
    } catch (value) {
      setError(formatErrorMessage(value));
    } finally {
      testingHostRef.current = false;
      setTestingHost(false);
    }
  };

  const saveChannel = async () => {
    if (savingChannel || testingHostRef.current) return;
    if (!device.id) {
      setError('请先保存单板，再添加 SDK 通道。');
      setChannelSaveFeedback('保存失败：请先保存单板。');
      return;
    }
    if (!host.name.trim() || !host.address.trim() || !host.username.trim()) {
      setError('远端主机名称、地址和 SSH 用户不能为空。');
      setChannelSaveFeedback('保存失败：远端主机信息不完整。');
      return;
    }
    if (!project.name.trim() || !project.mcpServerName.trim()) {
      setError('通道名称和 MCP 名称不能为空。');
      setChannelSaveFeedback('保存失败：通道名称或 MCP 名称为空。');
      return;
    }
    if (!project.sdkDir.trim().startsWith('/')) {
      setError('SDK 目录必须使用远端主机绝对路径（例如 /home/ubuntu/...）。');
      setChannelSaveFeedback('保存失败：SDK 目录不是绝对路径。');
      return;
    }
    const settingsSession = settingsSessionRef.current;
    setSavingChannel(true);
    setChannelSaveFeedback('正在保存 SDK 通道…');
    setChannelSaveProjectId(null);
    setError('');
    try {
      const hostId = await invokeCommand('workbench_save_host', { host });
      if (credentials.password || credentials.keyPassphrase) {
        await invokeCommand('workbench_save_credentials', { hostId, credentials });
      }
      const savedHost = { ...host, id: hostId };
      setHost(savedHost);
      const otherProjects = projects.filter((item) => item.id !== project.id);
      const localGatewayPort = availablePort(
        project.localGatewayPort,
        otherProjects.map((item) => item.localGatewayPort),
        8765
      );
      const sameHostProjects = otherProjects.filter((item) => item.hostId === hostId);
      const remoteForwardPort = availablePort(
        project.remoteForwardPort,
        sameHostProjects.map((item) => item.remoteForwardPort),
        18765
      );
      const mcpServerName = availableMcpName(
        mcpName(project.name),
        sameHostProjects.map((item) => item.mcpServerName)
      );
      const value = {
        ...project,
        hostId,
        deviceId: device.id,
        localGatewayPort,
        remoteForwardPort,
        mcpServerName,
      };
      const id = await invokeCommand('workbench_save_project', { project: value });
      setProject({ ...value, id });
      setChannelSaveFeedback(`通道“${value.name}”保存成功，正在建立连接…`);

      await invokeCommand('gateway_start', {
        port: value.localGatewayPort,
        projectId: id,
        deviceId: device.id,
      });
      const tunnel = await invokeCommand('workbench_start_tunnel', { projectId: id });
      setTunnelStatuses((items) => [...items.filter((item) => item.projectId !== id), tunnel]);
      setMessage(`通道“${value.name}”已完整保存，LYNX 正在自动建立隧道。`);
      await refreshConfig();
      channelSaveOwnerRef.current = {
        session: settingsSession,
        deviceId: device.id,
        projectId: id,
      };
      setChannelSaveProjectId(id);
    } catch (value) {
      const detail = formatErrorMessage(value);
      setError(detail);
      setChannelSaveFeedback(`保存或连接失败：${detail}`);
      setChannelSaveProjectId(null);
    } finally {
      setSavingChannel(false);
    }
  };

  const copy = async (value: string, label = '命令') => {
    if (!value) return;
    try {
      await navigator.clipboard.writeText(value);
      setMessage(`${label}已复制。`);
    } catch {
      setMessage('复制失败，请选中文本手动复制。');
    }
  };

  const deleteBoard = async (board: WorkbenchDevice) => {
    if (!board.id) return;
    const linkedProjects = projects.filter((item) => item.deviceId === board.id);
    if (
      !(await confirm(
        `确定删除单板“${board.name}”吗？该单板的 ${linkedProjects.length} 条 SDK 通道也会删除，共享 SSH 主机和凭证会保留。`,
        { title: '删除单板', kind: 'warning' }
      ))
    ) {
      return;
    }
    setError('');
    await Promise.allSettled(
      linkedProjects.flatMap((item) => [
        item.id
          ? invokeCommand('workbench_stop_tunnel', { projectId: item.id })
          : Promise.resolve(),
        invokeCommand('gateway_stop_port', { port: item.localGatewayPort }),
      ])
    );
    await invokeCommand('workbench_delete_config', { kind: 'device', id: board.id });
    if (device.id === board.id) {
      const nextBoard = devices.find((item) => item.id !== board.id);
      setDevice(nextBoard ?? { ...emptyDevice });
      setProject({ ...emptyProject });
      setHost({ ...emptyHost });
      setCredentials({});
      setSettingsOpen(false);
    }
    setMessage(`单板“${board.name}”及其 SDK 通道已删除。`);
    await refreshConfig();
  };

  const tunnelStatusText = remoteMcpError
    ? `远端状态查询失败：${remoteMcpError}`
    : tunnelStatus?.state === 'connected'
      ? '已连接，并通过远端实际访问验证'
      : tunnelStatus?.message ||
        (tunnelStatus?.state === 'verifying'
          ? 'SSH 已连接，正在验证远端通道'
          : tunnelStatus?.state === 'connecting'
            ? '正在连接远端 SDK 主机'
            : tunnelStatus?.state === 'reconnecting'
              ? '连接暂时失败，LYNX 正在自动重试'
              : '保存配置后由 LYNX 自动连接');

  const currentSerialPort = matchingSerialPort(device, serialPorts);
  const currentSerialOnline = !!currentSerialPort;
  const sessionOnlyPort =
    !currentSerialPort && !serialPorts.some((port) => port.name === device.serialPort)
      ? device.serialPort
      : '';
  const serialSessionOpen = useBoundSerialSession(isActive, device.id, sessionOnlyPort);
  const serialAvailable = currentSerialOnline || serialSessionOpen;
  const consoleDevices = devices.map((item) => {
    const port = matchingSerialPort(item, serialPorts);
    return port ? { ...item, serialPort: port.name } : item;
  });
  const consoleDevice = device.id
    ? {
        ...device,
        serialPort: currentSerialPort?.name ?? device.serialPort,
      }
    : undefined;
  const currentUsbOnline = isUsbOnline(device, usbDevices);
  const currentAdbOnline = isAdbOnline(device, adbDevices);
  const currentFlashTask = device.id
    ? taskHistory.find((task) => task.deviceId === device.id && isFlashTask(task))
    : undefined;
  const currentDeviceTaskBusy = Boolean(
    device.id &&
    taskHistory.some((task) => task.deviceId === device.id && task.status === 'running')
  );
  const headerFlashState =
    currentFlashTask?.status === 'running'
      ? 'running'
      : currentFlashTask?.status === 'completed' || currentFlashTask?.status === 'success'
        ? 'success'
        : currentFlashTask?.status === 'failed'
          ? 'failed'
          : 'idle';
  const flashTaskStatusText =
    currentFlashTask?.status === 'running'
      ? currentFlashTask.kind === 'mainline_boot'
        ? '正在启动主线系统'
        : '正在烧录'
      : currentFlashTask?.status === 'completed' || currentFlashTask?.status === 'success'
        ? currentFlashTask.kind === 'mainline_boot'
          ? '最近主线启动成功'
          : '最近烧录成功'
        : currentFlashTask?.status === 'failed'
          ? currentFlashTask.kind === 'mainline_boot'
            ? '最近主线启动失败'
            : '最近烧录失败'
          : '当前无烧录任务';
  const activeTask = taskHistory.find(
    (task) => task.deviceId === device.id && ['queued', 'running'].includes(task.status)
  );
  const taskLabel = activeTask
    ? `${activeTask.kind} · ${activeTask.phase || activeTask.status}`
    : '任务空闲';
  useEffect(() => {
    if (isActive)
      onStatusChange?.({ target: device.id ? device.name : '未选择设备', task: taskLabel });
  }, [device.id, device.name, isActive, onStatusChange, taskLabel]);
  const flashConnectionText = !device.usbBinding ? '未绑定' : currentUsbOnline ? '在线' : '离线';

  useEffect(() => {
    if (!isActive || !device.id) {
      setPowerState('unknown');
      return;
    }
    const deviceId = device.id;
    let cancelled = false;
    setPowerState('unknown');
    const refreshPowerState = () => {
      void invokeCommand('workbench_power_last_known_state', { deviceId })
        .then((state) => {
          if (!cancelled) setPowerState(state);
        })
        .catch(() => {
          if (!cancelled) setPowerState('unknown');
        });
    };
    refreshPowerState();
    const timer = window.setInterval(refreshPowerState, TUNNEL_STATUS_INTERVAL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [device.id, isActive]);
  const refreshLinuxOverview = useCallback(async () => {
    if (!device.id) return;
    if (!currentAdbOnline && !currentSerialOnline) {
      setLinuxOverviewError('请先连接 ADB，或打开该单板的串口并登录目标 Shell。');
      return;
    }
    const id = device.id;
    if (overviewRequests.current.has(id)) return;
    overviewRequests.current.add(id);
    setLinuxOverviewLoading(true);
    try {
      const next = await invokeCommand('workbench_linux_device_overview', { deviceId: id });
      if (next.deviceId !== id) return;
      setMonitorSamples((current) => ({
        ...current,
        [id]: recordOverview(current[id] ?? [], {
          time: next.collectedAtMs,
          cpu: next.cpuUsagePct,
          memory: usedPercent(next.memoryTotalBytes, next.memoryAvailableBytes),
        }),
      }));
      if (overviewDeviceIdRef.current !== id) return;
      setLinuxOverview(next);
      setLinuxOverviewError('');
    } catch (value) {
      if (overviewDeviceIdRef.current === id) setLinuxOverviewError(linuxOverviewErrorText(value));
    } finally {
      overviewRequests.current.delete(id);
      if (overviewDeviceIdRef.current === id) setLinuxOverviewLoading(false);
    }
  }, [currentAdbOnline, currentSerialOnline, device.id]);

  useEffect(() => {
    setLinuxOverview((current) => (current?.deviceId === device.id ? current : null));
    setLinuxOverviewLoading(device.id != null && overviewRequests.current.has(device.id));
    setLinuxOverviewError('');
  }, [device.id]);

  const hasSerialSnapshot =
    linuxOverview?.deviceId === device.id && linuxOverview?.transport === 'serial';
  useEffect(() => {
    if (!isActive || activeView !== 'overview' || !device.id || hasSerialSnapshot) return;
    void refreshLinuxOverview();
    const timer = window.setInterval(() => void refreshLinuxOverview(), 10000);
    return () => window.clearInterval(timer);
  }, [activeView, device.id, isActive, hasSerialSnapshot, refreshLinuxOverview]);

  const currentLinuxOverview = linuxOverview?.deviceId === device.id ? linuxOverview : null;
  const selectedPowerType = (() => {
    try {
      return (JSON.parse(device.powerProfileJson) as { type?: string }).type === 'ch347_gpio'
        ? 'ch347_gpio'
        : device.powerProfileJson
          ? 'ascii_cdc'
          : 'none';
    } catch {
      return device.powerProfileJson ? 'ascii_cdc' : 'none';
    }
  })();
  const selectedCh347Power = ch347PowerProfile(device);
  const selectedCh347Device =
    editorCh347Devices.find(
      (item) => !!device.powerControllerSerial && item.serialNumber === device.powerControllerSerial
    ) ??
    (!device.powerControllerSerial
      ? editorCh347Devices.find((item) => item.index === selectedCh347Power.deviceIndex)
      : undefined);
  const savedCh347ControllerOffline =
    selectedPowerType === 'ch347_gpio' && !!device.powerControllerSerial && !selectedCh347Device;

  const powerAction = async (action: 'power_on' | 'power_off' | 'get_state') => {
    if (!device.id) return;
    setPowerBusy(true);
    setPowerResult('');
    try {
      const response = await invokeCommand('workbench_power_action', {
        deviceId: device.id,
        action,
      });
      setPowerResult(response);
      const reportedState = powerStateFromResponse(response);
      if (action === 'power_on') setPowerState('on');
      else if (action === 'power_off') setPowerState('off');
      else if (reportedState !== 'unknown') setPowerState(reportedState);
    } catch (value) {
      setPowerResult(ipcFailureDetail(value));
    } finally {
      setPowerBusy(false);
    }
  };

  const powerCycle = async () => {
    if (!device.id) return;
    setPowerBusy(true);
    setPowerResult('正在断电重启…');
    try {
      await invokeCommand('workbench_power_action', { deviceId: device.id, action: 'power_off' });
      await new Promise((resolve) => window.setTimeout(resolve, device.powerOffWaitMs));
      const response = await invokeCommand('workbench_power_action', {
        deviceId: device.id,
        action: 'power_on',
      });
      setPowerResult(`cycle success · ${response}`);
      setPowerState('on');
    } catch (value) {
      setPowerResult(ipcFailureDetail(value));
    } finally {
      setPowerBusy(false);
    }
  };

  const toggleHeaderPower = async () => {
    if (powerState === 'on') {
      const approved = await confirm(`确认断开“${device.name}”的 Lynx Power 电源？`, {
        title: '确认断电',
        kind: 'warning',
      });
      if (!approved) return;
      await powerAction('power_off');
      return;
    }
    await powerAction('power_on');
  };

  const createDevice = () => {
    creatingBoardRef.current = true;
    setDevice({ ...emptyDevice });
    setHost({ ...emptyHost });
    setCredentials({});
    setProject({
      ...emptyProject,
      hostId: 0,
      name: '新单板 SDK',
      mcpServerName: mcpName('新单板 SDK'),
      remoteForwardPort: nextPort(
        projects.map((item) => item.remoteForwardPort),
        project.deviceId ? project.remoteForwardPort : undefined,
        18765
      ),
      localGatewayPort: nextPort(
        projects.map((item) => item.localGatewayPort),
        project.deviceId ? project.localGatewayPort : undefined,
        8765
      ),
    });
    openSettings();
  };
  const visibleDevices = devices.filter((item) =>
    `${item.name} ${vendorLabel(item.chipVendor)} ${item.serialPort}`
      .toLowerCase()
      .includes(boardSearch.trim().toLowerCase())
  );
  const deviceOnline = serialAvailable || currentAdbOnline || currentUsbOnline;
  const localMcpReady = !!localConnection.endpoint && !localMcpError;
  const remoteSdkReady = project.deviceId === device.id && !!remoteEndpoint && !remoteMcpError;
  const sshPresence = useSshPresence(
    isActive,
    project.deviceId === device.id ? project.id : null,
    device.id
  );
  const tools: ReadonlyArray<{
    id: LinuxDeviceView;
    label: string;
    icon: LynxIconName;
    status: string;
    online: boolean;
    partial?: boolean;
  }> = [
    {
      id: 'overview',
      label: '设备概览',
      icon: 'board',
      online: deviceOnline,
      status: deviceOnline ? '检测到设备' : '设备离线',
    },
    {
      id: 'serial',
      label: '串口终端',
      icon: 'terminal',
      online: serialAvailable,
      status: serialSessionOpen
        ? '绑定串口已有打开的会话'
        : currentSerialOnline
          ? '已检测到绑定的串口'
          : '未检测到绑定的串口',
    },
    {
      id: 'sdk',
      label: 'SSH 终端',
      icon: 'terminal',
      online: sshPresence === 'online',
      status:
        sshPresence === 'online'
          ? 'SSH 会话已连接'
          : sshPresence === 'offline'
            ? '无已连接的 SSH 会话'
            : 'SSH 会话状态未知',
    },
    {
      id: 'flash',
      label: '固件烧录',
      icon: 'download',
      online: currentUsbOnline,
      status: currentUsbOnline ? '检测到烧录设备' : '未检测到烧录设备',
    },
    {
      id: 'ai',
      label: 'AI 通道',
      icon: 'agent',
      online: localMcpReady && remoteSdkReady,
      partial: localMcpReady !== remoteSdkReady,
      status: `本机 MCP${localMcpReady ? '已验证' : '未验证'}，远端 SDK${remoteSdkReady ? '已验证' : '未验证'}`,
    },
  ];
  return (
    <div className="board-workbench advanced-workbench lynx-advanced-desktop">
      <aside className="board-list workbench-card" aria-label="Linux 单板">
        <div className="device-board-rail-heading">
          <span>
            <h3>Linux 单板</h3>
            <b>{devices.length}</b>
          </span>
          <button
            type="button"
            data-testid="linux-new-board"
            aria-label="新建单板"
            onClick={createDevice}
          >
            <LynxIcon name="add" size={16} /> 新建
          </button>
        </div>
        <label className="device-board-search">
          <LynxIcon name="search" size={16} />
          <input
            aria-label="搜索单板"
            placeholder="搜索单板"
            value={boardSearch}
            onChange={(event) => setBoardSearch(event.target.value)}
          />
        </label>
        <div className="board-items">
          {visibleDevices.map((item) => {
            const online =
              (item.id === device.id && serialSessionOpen) ||
              isSerialOnline(item, serialPorts) ||
              isUsbOnline(item, usbDevices) ||
              isAdbOnline(item, adbDevices);
            const vendor = vendorLabel(item.chipVendor);
            const [specs, interfaces] = linuxBoardSpecs(item);
            return (
              <div
                key={item.id}
                className={`board-item is-${item.chipVendor} ${item.id === device.id ? 'is-active' : ''}`}
              >
                <button
                  type="button"
                  className="board-item-select device-board-card-select"
                  aria-pressed={item.id === device.id}
                  onClick={() => {
                    creatingBoardRef.current = false;
                    setNavigationError('');
                    setDevice(item);
                  }}
                >
                  <BoardThumbnail
                    imagePath={item.boardImagePath}
                    marker={vendorMark(item.chipVendor)}
                    tone={item.chipVendor}
                  />
                  <span className="device-board-copy">
                    <span className="device-board-name-row">
                      <strong title={item.name}>{item.name}</strong>
                      <span className={`board-live-row ${online ? 'is-online' : ''}`}>
                        <i /> {online ? '在线' : '离线'}
                      </span>
                    </span>
                    <small>{vendor}</small>
                    <span className="device-board-specs">
                      {specs !== vendor && <b title={specs}>{specs}</b>}
                      <em title={interfaces}>{interfaces}</em>
                    </span>
                  </span>
                </button>
              </div>
            );
          })}
          {!visibleDevices.length && (
            <p className="board-empty">
              {devices.length ? '没有匹配的单板。' : '点击“新建”添加第一块单板。'}
            </p>
          )}
        </div>
        {(onOpenRecords || onOpenSettings) && (
          <footer className="lynx-board-rail-actions">
            {onOpenRecords && (
              <button onClick={onOpenRecords}>
                <LynxIcon name="history" size={16} />
                任务记录
              </button>
            )}
            {onOpenSettings && (
              <button onClick={onOpenSettings}>
                <LynxIcon name="settings" size={16} />
                应用设置
              </button>
            )}
          </footer>
        )}
      </aside>
      <main className="board-main">
        {!settingsOpen && error && (
          <p className="adv-navigation-error" role="alert">
            {error}
          </p>
        )}
        {navigationError && (
          <p className="adv-navigation-error" role="alert">
            {navigationError}
          </p>
        )}
        {device.id ? (
          <>
            <header className="device-workbench-header workbench-card">
              <div className="device-workbench-identity">
                <span className="device-header-icon">
                  <LynxIcon name="board" filled size={20} />
                </span>
                <div>
                  <span className="device-workbench-kicker">LINUX / SOC BOARD</span>
                  <h2>{device.name}</h2>
                  <p>
                    {vendorLabel(device.chipVendor)} ·{' '}
                    {device.serialPort ? `串口绑定 ${device.serialPort}` : '串口未绑定'} ·{' '}
                    {formatUsbBinding(device.usbBinding)}
                  </p>
                </div>
              </div>
              <div className="device-header-actions">
                <button
                  type="button"
                  data-testid="linux-header-power"
                  className={`device-header-power is-${powerState} ${powerBusy ? 'is-busy' : ''}`}
                  disabled={powerBusy || currentDeviceTaskBusy || selectedPowerType === 'none'}
                  title={
                    selectedPowerType === 'none'
                      ? '尚未配置 Lynx Power'
                      : currentDeviceTaskBusy
                        ? '当前单板有硬件任务运行，电源操作已锁定'
                        : powerBusy
                          ? '电源切换中'
                          : powerState === 'on'
                            ? '上次成功命令：已上电；点击断电'
                            : powerState === 'off'
                              ? '上次成功命令：已断电；点击上电'
                              : '电源状态未知；点击上电'
                  }
                  aria-label={
                    powerState === 'on' ? 'Lynx Power 已上电，点击断电' : 'Lynx Power 点击上电'
                  }
                  onClick={() => void toggleHeaderPower()}
                >
                  <LynxIcon name="plug" filled={powerState === 'on'} size={16} />
                </button>
                <span
                  data-testid="linux-header-device-state"
                  className={deviceOnline ? 'is-online' : ''}
                >
                  <i /> {deviceOnline ? '设备在线' : '设备离线'}
                </span>
                <span
                  data-testid="linux-header-flash-state"
                  className={`device-header-flash-state is-${headerFlashState}`}
                  title={
                    currentFlashTask
                      ? `${currentFlashTask.taskId} · ${flashTaskStatusText}`
                      : '当前无烧录任务'
                  }
                >
                  <i />{' '}
                  {headerFlashState === 'running'
                    ? '正在烧录'
                    : headerFlashState === 'idle'
                      ? '无烧录任务'
                      : flashTaskStatusText}
                </span>
                <button
                  type="button"
                  data-testid="linux-header-new-ssh"
                  disabled={!project.id || !host.id}
                  title={
                    project.id && host.id ? `在 ${project.name} 新建 SSH 终端` : '请先保存 SDK 通道'
                  }
                  onClick={() => {
                    setActiveView('sdk');
                    setSshOpenRequestId((current) => current + 1);
                  }}
                >
                  <LynxIcon name="terminal" size={16} /> SSH 终端
                </button>
                <button
                  type="button"
                  data-testid="linux-open-terminal-workspace"
                  onClick={() => {
                    if (activeView !== 'workspace') {
                      viewBeforeWorkspaceRef.current = activeView;
                      setWorkspacePrimarySshOnline(false);
                      setActiveView('workspace');
                    }
                  }}
                >
                  <LynxIcon name="fullscreen" size={16} /> 全屏
                </button>
                <button data-testid="linux-header-device-settings" onClick={openSettings}>
                  设备设置
                </button>
              </div>
            </header>

            <nav className="device-workbench-tabs" aria-label="Linux 单板功能">
              {tools.map(({ id, label, icon, online, partial, status }) => (
                <button
                  type="button"
                  key={id}
                  data-testid={`linux-tab-${id}`}
                  className={activeView === id ? 'is-active' : ''}
                  aria-current={activeView === id ? 'page' : undefined}
                  aria-description={status}
                  title={`${label} · ${status}`}
                  onClick={() => setActiveView(id)}
                >
                  <span className="device-tab-icon">
                    <LynxIcon name={icon} filled={activeView === id} size={20} />
                    <i className={online ? 'is-online' : partial ? 'is-partial' : ''} />
                  </span>
                  <span>{label}</span>
                </button>
              ))}
            </nav>
            <div className="lynx-device-tool-content">
              {activeView === 'overview' && (
                <div className="lynx-device-overview">
                  <AdvancedOverview
                    device={device}
                    snapshot={currentLinuxOverview}
                    samples={monitorSamples[device.id] ?? []}
                    history={taskHistory}
                    error={linuxOverviewError}
                    loading={linuxOverviewLoading}
                    serial={currentSerialPort?.name || device.serialPort}
                    sdk={host.address}
                    tunnel={tunnelStatusText}
                    usb={currentUsbOnline}
                    adb={currentAdbOnline}
                    serialOnline={serialAvailable}
                    onRefresh={() => void refreshLinuxOverview()}
                    onOpenRecords={onOpenRecords}
                  />
                  <AdbFileTransfer
                    key={device.id}
                    device={device}
                    isActive={isActive}
                    online={currentAdbOnline}
                    onOpenSettings={openSettings}
                    onDetect={() => void scanInterfaces()}
                  />
                </div>
              )}
              <div className="layout adv-hardware-tools">
                {(['serial', 'flash', 'workspace'] as LinuxDeviceView[]).includes(activeView) && (
                  <div className="device-console-view is-visible">
                    <BoardConsole
                      applicationCandidateId={
                        firmwareHandoff?.deviceId === device.id
                          ? firmwareHandoff.candidateId
                          : undefined
                      }
                      view={
                        activeView === 'workspace'
                          ? 'workspace'
                          : activeView === 'flash'
                            ? 'flash'
                            : 'serial'
                      }
                      devices={consoleDevices}
                      selectedDevice={consoleDevice}
                      isActive={
                        isActive &&
                        (['serial', 'flash', 'workspace'] as LinuxDeviceView[]).includes(activeView)
                      }
                      taskHistory={taskHistory}
                      serialDetected={currentSerialOnline}
                      adbDetected={currentAdbOnline}
                      flashDetected={currentUsbOnline}
                      flashBound={!!device.usbBinding}
                      savedSerialPort={device.serialPort}
                      flashProtocol={flashProtocolLabel(device.chipVendor)}
                      onOpenSettings={openSettings}
                      aiChannel={
                        <DeviceAiChannel
                          kind="linux"
                          boardId={device.id}
                          board={device}
                          project={project.id ? project : undefined}
                          host={host.id ? host : undefined}
                          tunnel={tunnelStatus}
                          onOpenSettings={openSettings}
                        />
                      }
                      workspace={{
                        projectName: project.id ? project.name : '',
                        hostLabel: host.id ? `${host.username}@${host.address}` : '',
                        primaryTerminalOnline: workspacePrimarySshOnline,
                        overview: currentLinuxOverview ?? undefined,
                        onRequestExit: () => setActiveView(viewBeforeWorkspaceRef.current),
                        primaryTerminal: (
                          <SdkTerminal
                            isActive={isActive && activeView === 'workspace'}
                            board={device}
                            project={project.id ? project : undefined}
                            host={host.id ? host : undefined}
                            tunnel={tunnelStatus}
                            onOpenSettings={openSettings}
                            variant="workspace"
                            sessionIndex={0}
                            workspaceRole="primary"
                            onWorkspaceOnlineChange={handlePrimaryWorkspaceOnline}
                          />
                        ),
                      }}
                    />
                  </div>
                )}

                {activeView === 'sdk' && (
                  <SdkTerminal
                    isActive={isActive}
                    board={device}
                    project={project.id ? project : undefined}
                    host={host.id ? host : undefined}
                    tunnel={tunnelStatus}
                    onOpenSettings={openSettings}
                    openRequestId={sshOpenRequestId}
                    onOpenRequestHandled={handleSshOpenRequest}
                  />
                )}

                {activeView === 'ai' && (
                  <DeviceAiChannel
                    kind="linux"
                    boardId={device.id}
                    board={device}
                    project={project.id ? project : undefined}
                    host={host.id ? host : undefined}
                    tunnel={tunnelStatus}
                    onOpenSettings={openSettings}
                  />
                )}
              </div>
            </div>
          </>
        ) : (
          <section className="adv-no-target">
            <LynxIcon name="board" size={24} />
            <h2>{devices.length ? '请选择设备' : '添加开发板'}</h2>
            <p>
              {devices.length
                ? '从左侧选择设备以打开工具。'
                : '先保存开发板型号和接口，再使用串口、SSH、固件烧录和文件传输。'}
            </p>
            <button className="adv-primary" onClick={createDevice}>
              添加设备
            </button>
          </section>
        )}

        {settingsOpen && (
          <div
            data-testid="linux-settings-backdrop"
            className="workbench-settings-backdrop"
            role="presentation"
            onMouseDown={(event) => {
              if (event.target === event.currentTarget) setSettingsOpen(false);
            }}
          >
            <div
              data-testid="linux-settings-dialog"
              className="workbench-settings-dialog"
              role="dialog"
              aria-modal="true"
            >
              <header className="workbench-settings-header">
                <div>
                  <small>BOARD & MCP AGENT SETTINGS</small>
                  <h2>{device.id ? device.name : '新建单板'}</h2>
                  <span>配置保存一次后，LYNX 会自动恢复接口、Gateway 与 SDK 隧道。</span>
                </div>
                <button onClick={() => setSettingsOpen(false)}>关闭</button>
              </header>
              <div className="workbench-settings-body">
                <section className="workbench-card board-editor">
                  <div className="board-section-title">
                    <div>
                      <small>HARDWARE CHANNELS</small>
                      <h3>{device.id ? device.name : '新建单板'}</h3>
                    </div>
                    <button disabled={scanningInterfaces} onClick={() => scanInterfaces(true)}>
                      {scanningInterfaces ? '正在扫描…' : '重新扫描接口'}
                    </button>
                  </div>
                  {interfaceScanFeedback && (
                    <output className="channel-save-feedback interface-scan-feedback" role="status">
                      {interfaceScanFeedback}
                    </output>
                  )}
                  {device.id && (
                    <div className="board-live-status">
                      <div className={serialAvailable ? 'is-online' : ''}>
                        <small>调试串口</small>
                        <strong>{serialAvailable ? '在线' : '离线'}</strong>
                        <span>{device.serialPort || '未绑定'}</span>
                      </div>
                      <div className={currentUsbOnline ? 'is-online' : ''}>
                        <small>固件烧录</small>
                        <strong>{flashConnectionText}</strong>
                        <span>
                          {flashProtocolLabel(device.chipVendor)} ·{' '}
                          {formatUsbBinding(device.usbBinding)} · {flashTaskStatusText}
                        </span>
                      </div>
                      <div className={currentAdbOnline ? 'is-online' : ''}>
                        <small>ADB</small>
                        <strong>{currentAdbOnline ? '在线' : '离线'}</strong>
                        <span>{device.adbSerial || '未绑定'}</span>
                      </div>
                    </div>
                  )}
                  <div className="board-form-grid">
                    <label>
                      <span>芯片厂家</span>
                      <select
                        value={device.chipVendor}
                        onChange={(event) => {
                          const chipVendor = event.target.value as WorkbenchDevice['chipVendor'];
                          const vendor = CHIP_VENDORS.find((item) => item.value === chipVendor);
                          setDevice({
                            ...device,
                            chipVendor,
                            flasherKind: vendor?.flasher ?? '',
                            flasherPath: '',
                            flasherArgs:
                              chipVendor === 'kendryte'
                                ? k230Args('emmc')
                                : chipVendor === 'st'
                                  ? stm32DfuArgs('0', '', true)
                                  : '',
                            usbBinding: '',
                            serialConsoleType:
                              chipVendor === 'kendryte'
                                ? 'rt_smart'
                                : device.serialConsoleType === 'rt_smart'
                                  ? 'auto'
                                  : device.serialConsoleType,
                            serialAutoLogin:
                              chipVendor === 'kendryte' ? false : device.serialAutoLogin,
                          });
                          setUsbDevices([]);
                        }}
                      >
                        {CHIP_VENDORS.map((item) => (
                          <option key={item.value} value={item.value}>
                            {item.label}
                          </option>
                        ))}
                      </select>
                    </label>
                    <label>
                      <span>单板名称</span>
                      <input
                        data-testid="linux-board-name"
                        placeholder="例如：T153-实验台-A"
                        value={device.name}
                        onChange={(event) => setDevice({ ...device, name: event.target.value })}
                      />
                    </label>
                    <div className="board-image-field is-wide">
                      <span>主板图片</span>
                      <div>
                        <button
                          type="button"
                          onClick={async () => {
                            const selected = await open({
                              multiple: false,
                              directory: false,
                              filters: [
                                {
                                  name: '主板图片',
                                  extensions: ['png', 'jpg', 'jpeg', 'webp'],
                                },
                              ],
                            });
                            if (typeof selected === 'string') {
                              setDevice({ ...device, boardImagePath: selected });
                            }
                          }}
                        >
                          选择图片
                        </button>
                        <span>
                          {device.boardImagePath.split(/[/\\]/).pop() ||
                            '未设置，将显示 PCB 示意占位'}
                        </span>
                        {device.boardImagePath && (
                          <button
                            type="button"
                            onClick={() => setDevice({ ...device, boardImagePath: '' })}
                          >
                            移除
                          </button>
                        )}
                      </div>
                    </div>
                    <label>
                      <span>调试串口</span>
                      <select
                        value={device.serialPort}
                        onChange={(event) => selectSerial(event.target.value)}
                      >
                        <option value="">不绑定</option>
                        {editorSerialPorts.map((item) => (
                          <option key={item.name} value={item.name}>
                            {item.name} {item.description ? `· ${item.description}` : ''}
                            {item.serialNumber ? ` · SN ${item.serialNumber}` : ''}
                            {item.interfaceNumber != null
                              ? ` · IF ${item.interfaceNumber.toString(16).padStart(2, '0')}`
                              : ''}
                          </option>
                        ))}
                      </select>
                      <em className="linux-serial-node-hint">
                        当前选择：{device.serialPort || '未绑定'}
                        {device.serialInterfaceNumber != null
                          ? ` · USB 接口 ${device.serialInterfaceNumber}`
                          : ' · USB 接口号未提供'}
                        <br />
                        节点排列顺序不代表模块上的第几路；请核对模块接线，保存后在串口终端查看实际节点与接收字节。
                      </em>
                    </label>
                    <label>
                      <span>波特率</span>
                      <select
                        value={device.baudRate}
                        onChange={(event) =>
                          setDevice({ ...device, baudRate: Number(event.target.value) })
                        }
                      >
                        {[9600, 38400, 57600, 115200, 921600, 1500000].map((rate) => (
                          <option key={rate} value={rate}>
                            {rate}
                          </option>
                        ))}
                      </select>
                    </label>
                    <label>
                      <span>串口控制台类型</span>
                      <select
                        value={device.serialConsoleType}
                        onChange={(event) => {
                          const serialConsoleType = event.target
                            .value as WorkbenchDevice['serialConsoleType'];
                          setDevice({
                            ...device,
                            serialConsoleType,
                            serialAutoLogin:
                              serialConsoleType === 'rt_smart' ? false : device.serialAutoLogin,
                          });
                        }}
                      >
                        <option value="auto">自动识别（未知时不发送命令）</option>
                        <option value="linux">Linux Shell</option>
                        <option value="rt_smart">RT-Smart / msh</option>
                      </select>
                      <em>K230 默认 RT-Smart；只有明确选择 Linux Shell 才允许 Linux 遥测。</em>
                    </label>
                    <label>
                      <span>串口登录方式</span>
                      <select
                        value={device.serialLoginMode}
                        disabled={device.serialConsoleType === 'rt_smart'}
                        onChange={(event) => {
                          const serialLoginMode = event.target
                            .value as WorkbenchDevice['serialLoginMode'];
                          setDevice({
                            ...device,
                            serialLoginMode,
                            serialAutoLogin:
                              serialLoginMode === 'none' ? false : device.serialAutoLogin,
                          });
                        }}
                      >
                        <option value="none">无需登录 / 手动登录</option>
                        <option value="username">只发送用户名</option>
                        <option value="password">用户名和密码</option>
                      </select>
                    </label>
                    {device.serialLoginMode !== 'none' &&
                      device.serialConsoleType !== 'rt_smart' && (
                        <>
                          <label>
                            <span>登录用户名</span>
                            <input
                              value={device.serialLoginUsername}
                              maxLength={64}
                              onChange={(event) =>
                                setDevice({
                                  ...device,
                                  serialLoginUsername: event.target.value,
                                })
                              }
                            />
                          </label>
                          <label>
                            <span>登录提示符</span>
                            <input
                              value={device.serialLoginPrompt}
                              maxLength={64}
                              onChange={(event) =>
                                setDevice({ ...device, serialLoginPrompt: event.target.value })
                              }
                            />
                          </label>
                          <label>
                            <span>Shell 提示符</span>
                            <input
                              value={device.serialShellPrompt}
                              maxLength={64}
                              onChange={(event) =>
                                setDevice({ ...device, serialShellPrompt: event.target.value })
                              }
                            />
                          </label>
                          {device.serialLoginMode === 'password' && (
                            <>
                              <label>
                                <span>密码提示符</span>
                                <input
                                  value={device.serialPasswordPrompt}
                                  maxLength={64}
                                  onChange={(event) =>
                                    setDevice({
                                      ...device,
                                      serialPasswordPrompt: event.target.value,
                                    })
                                  }
                                />
                              </label>
                              <label>
                                <span>登录密码</span>
                                <input
                                  type="password"
                                  autoComplete="new-password"
                                  value={serialLoginPassword}
                                  disabled={deleteSerialPassword}
                                  placeholder={
                                    serialPasswordConfigured
                                      ? '已安全保存；留空保持不变'
                                      : '保存到系统密钥环'
                                  }
                                  onChange={(event) => setSerialLoginPassword(event.target.value)}
                                />
                                <em>密码不会进入单板数据库、日志、审计正文或 MCP。</em>
                              </label>
                              {serialPasswordConfigured && (
                                <label>
                                  <span>已保存密码</span>
                                  <select
                                    value={deleteSerialPassword ? 'delete' : 'keep'}
                                    onChange={(event) =>
                                      setDeleteSerialPassword(event.target.value === 'delete')
                                    }
                                  >
                                    <option value="keep">保留系统密钥环密码</option>
                                    <option value="delete">保存时删除密码</option>
                                  </select>
                                </label>
                              )}
                            </>
                          )}
                          <label>
                            <span>自动登录</span>
                            <select
                              value={device.serialAutoLogin ? 'enabled' : 'disabled'}
                              onChange={(event) =>
                                setDevice({
                                  ...device,
                                  serialAutoLogin: event.target.value === 'enabled',
                                })
                              }
                            >
                              <option value="disabled">关闭（默认）</option>
                              <option value="enabled">等待提示符后自动登录</option>
                            </select>
                            <em>只在准确匹配登录阶段后发送一次，不会循环猜测凭据。</em>
                          </label>
                        </>
                      )}
                    {device.chipVendor === 'kendryte' && (
                      <label>
                        <span>K230 目标介质</span>
                        <select
                          value={k230Medium(device.flasherArgs)}
                          onChange={(event) =>
                            setDevice({
                              ...device,
                              flasherArgs: k230Args(event.target.value),
                            })
                          }
                        >
                          <option value="emmc">EMMC（SDIO0）</option>
                          <option value="sd_card">SD Card（SDIO1）</option>
                          <option value="spi_nand">SPI NAND</option>
                          <option value="spi_nor">SPI NOR</option>
                          <option value="otp">OTP</option>
                        </select>
                        <em>LYNX 内置协议，烧录完成后自动重启</em>
                      </label>
                    )}
                    {device.chipVendor === 'allwinner' && (
                      <>
                        <label>
                          <span>Allwinner 烧录路线</span>
                          <select
                            value={allwinnerFlashOptions(device.flasherArgs).route}
                            onChange={(event) => {
                              const current = allwinnerFlashOptions(device.flasherArgs);
                              setDevice({
                                ...device,
                                flasherArgs: allwinnerFlashArgs(
                                  event.target.value as
                                    | 'vendor_image'
                                    | 'mainline_emmc'
                                    | 'mainline_nand'
                                    | 'mainline_spinor',
                                  current.loaderPath
                                ),
                              });
                            }}
                          >
                            <option value="vendor_image">厂商 IMAGEWTY 完整固件</option>
                            <option value="mainline_emmc">
                              纯主线系统 · FES eMMC（含硬件 Boot0）
                            </option>
                            <option value="mainline_nand">纯主线系统 · FES NAND 组件</option>
                            <option value="mainline_spinor">
                              SPI NOR · 完整物理镜像（待回读验收）
                            </option>
                          </select>
                          <em>
                            eMMC 路线按 MBR、user area、硬件 Boot0、回读校验执行；NAND 路线使用 FES
                            坏块与 Boot0/Boot1 组件语义。
                          </em>
                        </label>
                        {allwinnerFlashOptions(device.flasherArgs).route !== 'vendor_image' && (
                          <div className="board-image-field">
                            <span>RAM FES Loader</span>
                            <div>
                              <button
                                type="button"
                                onClick={async () => {
                                  const selected = await open({
                                    multiple: false,
                                    directory: false,
                                    filters: [
                                      {
                                        name: 'Allwinner Loader',
                                        extensions: ['bin', 'img', 'fex'],
                                      },
                                    ],
                                  });
                                  if (typeof selected !== 'string') return;
                                  const current = allwinnerFlashOptions(device.flasherArgs);
                                  setDevice({
                                    ...device,
                                    flasherArgs: allwinnerFlashArgs(current.route, selected),
                                  });
                                }}
                              >
                                选择 Loader
                              </button>
                              <span>
                                {allwinnerFlashOptions(device.flasherArgs)
                                  .loaderPath.split(/[/\\]/)
                                  .pop() || '未选择'}
                              </span>
                            </div>
                            <em>Loader 只在 RAM 中运行，不会作为目标系统写入存储。</em>
                          </div>
                        )}
                      </>
                    )}
                    {device.chipVendor === 'st' && (
                      <>
                        <label>
                          <span>DFU Alternate Setting</span>
                          <input
                            value={stm32DfuOptions(device.flasherArgs).alt}
                            placeholder="例如：0、fsbl1 或 rootfs"
                            onChange={(event) => {
                              const current = stm32DfuOptions(device.flasherArgs);
                              setDevice({
                                ...device,
                                flasherArgs: stm32DfuArgs(
                                  event.target.value,
                                  current.address,
                                  current.reset
                                ),
                              });
                            }}
                          />
                          <em>必须与 dfu-util --list 显示的 alt 编号或名称一致</em>
                        </label>
                        <label>
                          <span>DfuSe 地址（可选）</span>
                          <input
                            value={stm32DfuOptions(device.flasherArgs).address}
                            placeholder="例如：0x08000000:leave"
                            onChange={(event) => {
                              const current = stm32DfuOptions(device.flasherArgs);
                              setDevice({
                                ...device,
                                flasherArgs: stm32DfuArgs(
                                  current.alt,
                                  event.target.value,
                                  current.reset
                                ),
                              });
                            }}
                          />
                        </label>
                        <label>
                          <span>烧录完成动作</span>
                          <select
                            value={stm32DfuOptions(device.flasherArgs).reset ? 'reset' : 'none'}
                            onChange={(event) => {
                              const current = stm32DfuOptions(device.flasherArgs);
                              setDevice({
                                ...device,
                                flasherArgs: stm32DfuArgs(
                                  current.alt,
                                  current.address,
                                  event.target.value === 'reset'
                                ),
                              });
                            }}
                          >
                            <option value="reset">USB Reset</option>
                            <option value="none">保持 DFU 状态</option>
                          </select>
                        </label>
                      </>
                    )}
                    <label>
                      <span>USB / {flashProtocolLabel(device.chipVendor)} 接口</span>
                      <select
                        value={device.usbBinding}
                        onChange={(event) =>
                          updateDeviceBinding({ usbBinding: event.target.value })
                        }
                      >
                        <option value="">不绑定</option>
                        {device.usbBinding &&
                          !selectedEditorUsbDevices.some(
                            (item) => item.key === device.usbBinding
                          ) && (
                            <option value={device.usbBinding}>
                              旧绑定已离线，请重新选择当前物理 USB 口
                            </option>
                          )}
                        {selectedEditorUsbDevices.map((item) => {
                          const value = item.key;
                          return (
                            <option key={value} value={value}>
                              物理 USB 口 {item.bus}-{item.port} · {item.mode} · {item.chip} · VID:
                              {item.vid.toString(16).padStart(4, '0')} PID:
                              {item.pid.toString(16).padStart(4, '0')}
                            </option>
                          );
                        })}
                      </select>
                      <em>
                        {selectedEditorUsbDevices.length
                          ? `发现 ${selectedEditorUsbDevices.length} 个接口，请从上面的下拉框选择`
                          : `未发现在线 ${flashProtocolLabel(device.chipVendor)} 接口`}
                        {detectedUsbSummary ? `；全部协议：${detectedUsbSummary}` : ''}
                      </em>
                    </label>
                    <label>
                      <span>ADB 接口</span>
                      <select
                        value={device.adbSerial}
                        onChange={(event) => updateDeviceBinding({ adbSerial: event.target.value })}
                      >
                        <option value="">
                          {editorAdbDevices.length ? '不绑定' : '当前未检测到 ADB 设备'}
                        </option>
                        {device.adbSerial &&
                          !editorAdbDevices.some((item) => item.serial === device.adbSerial) && (
                            <option value={device.adbSerial}>已保存：{device.adbSerial}</option>
                          )}
                        {editorAdbDevices.map((item) => {
                          const boundBoard = adbBoundBoard(item.serial);
                          const occupiedByOther = !!boundBoard && boundBoard.id !== device.id;
                          return (
                            <option
                              key={item.serial}
                              value={item.serial}
                              disabled={occupiedByOther}
                            >
                              {item.serial} · {item.model || item.device || item.state}
                              {boundBoard
                                ? ` · 已绑定：${boundBoard.name}${occupiedByOther ? '' : '（当前单板）'}`
                                : ' · 未绑定'}
                            </option>
                          );
                        })}
                      </select>
                      <em>
                        {selectedAdbOwner
                          ? `选择无效：该 ADB 已绑定到单板“${selectedAdbOwner.name}”，请选择“未绑定”的设备`
                          : editorAdbDevices.length
                            ? `发现 ${editorAdbDevices.length} 个 ADB 设备，请从上面的下拉框选择`
                            : selectedEditorUsbDevices.length
                              ? `当前单板处于 ${flashProtocolLabel(device.chipVendor)} 模式，不会同时出现在 ADB；启动系统并开启 USB 调试后再扫描`
                              : '启动系统、开启 USB 调试并授权此电脑后，再点击“重新扫描接口”'}
                      </em>
                    </label>
                    <label>
                      <span>USB 摄像头（Linux MCP 拍照）</span>
                      <select
                        value={device.cameraBinding}
                        onChange={(event) =>
                          updateDeviceBinding({ cameraBinding: event.target.value })
                        }
                      >
                        <option value="">不绑定</option>
                        {device.cameraBinding &&
                          !editorCameraDevices.some(
                            (item) => item.key === device.cameraBinding
                          ) && <option value={device.cameraBinding}>已保存摄像头当前离线</option>}
                        {editorCameraDevices.map((item) => (
                          <option key={item.key} value={item.key}>
                            {item.name} · {item.description}
                          </option>
                        ))}
                      </select>
                      <em>仅供当前 Linux 开发板通道的 lynx_camera_capture 使用；照片不自动保存</em>
                    </label>
                  </div>
                  <div className="relay-power-card">
                    <div className="relay-power-heading">
                      <div>
                        <small>LYNX POWER</small>
                        <strong>Lynx Power 电源控制</strong>
                      </div>
                      <span className={selectedCh347Device ? 'is-online' : ''}>
                        {selectedPowerType === 'ch347_gpio'
                          ? selectedCh347Device
                            ? '设备已连接'
                            : savedCh347ControllerOffline
                              ? '原控制器离线'
                              : '未检测到设备'
                          : selectedPowerType === 'ascii_cdc'
                            ? 'ASCII CDC'
                            : '未配置'}
                      </span>
                    </div>
                    <div className="relay-power-config">
                      <label>
                        <span>控制器</span>
                        <select
                          value={selectedPowerType}
                          onChange={(event) => {
                            const type = event.target.value;
                            if (type === 'ch347_gpio')
                              setDevice(
                                editorCh347Devices[0]
                                  ? withCh347Device(device, editorCh347Devices[0])
                                  : withCh347Power(device, {})
                              );
                            else if (type === 'none')
                              setDevice({
                                ...device,
                                powerController: '',
                                powerControllerVid: 0,
                                powerControllerPid: 0,
                                powerControllerSerial: '',
                                powerControllerInterfaceNumber: null,
                                powerProfileJson: '',
                                powerChannel: '',
                              });
                          }}
                        >
                          <option value="none">不启用</option>
                          <option value="ch347_gpio">Lynx Power 控制器</option>
                          {selectedPowerType === 'ascii_cdc' && (
                            <option value="ascii_cdc">原有 ASCII CDC 配置</option>
                          )}
                        </select>
                      </label>
                      {selectedPowerType === 'ch347_gpio' && (
                        <>
                          <label>
                            <span>Lynx Power 设备</span>
                            <select
                              value={
                                selectedCh347Device
                                  ? `device:${selectedCh347Device.serialNumber || selectedCh347Device.index}`
                                  : device.powerControllerSerial
                                    ? `saved:${device.powerControllerSerial}`
                                    : ''
                              }
                              onChange={async (event) => {
                                const controller = editorCh347Devices.find(
                                  (item) =>
                                    `device:${item.serialNumber || item.index}` ===
                                    event.target.value
                                );
                                if (!controller) return;
                                if (
                                  device.powerControllerSerial &&
                                  controller.serialNumber !== device.powerControllerSerial &&
                                  !(await confirm(
                                    `已保存的 Lynx Power ${device.powerControllerSerial} 当前离线。确认替换为 ${controller.serialNumber || `设备 #${controller.index}`}？GPIO 和供电模式会保持不变，保存前不会执行电源动作。`,
                                    { title: '替换 Lynx Power 绑定', kind: 'warning' }
                                  ))
                                ) {
                                  return;
                                }
                                setDevice(withCh347Device(device, controller));
                              }}
                            >
                              {savedCh347ControllerOffline && (
                                <option value={`saved:${device.powerControllerSerial}`} disabled>
                                  已保存：{device.powerControllerSerial}（离线）
                                </option>
                              )}
                              {!editorCh347Devices.length && (
                                <option value="">未发现可用控制器</option>
                              )}
                              {editorCh347Devices.map((item) => (
                                <option
                                  key={`${item.index}:${item.serialNumber}`}
                                  value={`device:${item.serialNumber || item.index}`}
                                >
                                  {item.name} · {item.serialNumber || '无序列号'} · IF
                                  {item.interfaceNumber}
                                </option>
                              ))}
                            </select>
                            {savedCh347ControllerOffline && editorCh347Devices.length > 0 && (
                              <em>
                                检测到其他 Lynx Power；请选择后确认替换。LYNX
                                不会按设备索引自动换绑。
                              </em>
                            )}
                          </label>
                          <label>
                            <span>电源控制通道</span>
                            <select
                              value={selectedCh347Power.gpio}
                              onChange={(event) =>
                                setDevice(
                                  withCh347Power(device, { gpio: Number(event.target.value) })
                                )
                              }
                            >
                              {[4, 5, 6, 7].map((gpio) => (
                                <option key={gpio} value={gpio}>
                                  GPIO{gpio}
                                </option>
                              ))}
                            </select>
                          </label>
                          <label>
                            <span>继电器拓扑</span>
                            <select
                              value={selectedCh347Power.roleMode}
                              onChange={(event) =>
                                setDevice(
                                  withCh347Power(device, {
                                    roleMode:
                                      event.target.value === 'singleRelay'
                                        ? 'singleRelay'
                                        : event.target.value === 'dualPower'
                                          ? 'dualPower'
                                          : 'independent',
                                  })
                                )
                              }
                            >
                              <option value="independent">
                                三继电器（仅适用可外接 Boot / Reset 的板卡）
                              </option>
                              <option value="singleRelay">单继电器：电源 / 断电重启复用</option>
                              <option value="dualPower">双路 USB 协同供电</option>
                            </select>
                          </label>
                          {selectedCh347Power.roleMode === 'singleRelay' ? (
                            <label>
                              <span>Boot / Reset 复用</span>
                              <output>
                                GPIO{selectedCh347Power.gpio} · 只控制供电，不会代按物理 FEL
                                键；Allwinner 由专用 MCP 使用 ADB reboot efex，或电源重启后串口
                                U-Boot efex
                              </output>
                            </label>
                          ) : selectedCh347Power.roleMode === 'dualPower' ? (
                            <>
                              <label>
                                <span>数据 / 烧录 USB 辅助供电通道</span>
                                <select
                                  value={device.bootChannel}
                                  onChange={(event) =>
                                    setDevice({ ...device, bootChannel: event.target.value })
                                  }
                                >
                                  {[4, 5, 6, 7].map((gpio) => (
                                    <option key={gpio} value={String(gpio)}>
                                      GPIO{gpio}
                                    </option>
                                  ))}
                                </select>
                              </label>
                              <label>
                                <span>双路动作约束</span>
                                <output>
                                  GPIO{selectedCh347Power.gpio} + GPIO
                                  {device.bootChannel || '—'} 在一次 CH347 写入中同时上电或断电；
                                  不会单独操作辅助供电 GPIO
                                </output>
                              </label>
                            </>
                          ) : (
                            <>
                              <label>
                                <span>烧录模式（Boot）通道</span>
                                <select
                                  value={device.bootChannel}
                                  onChange={(event) =>
                                    setDevice({ ...device, bootChannel: event.target.value })
                                  }
                                >
                                  <option value="">不配置（禁用自动进烧录模式）</option>
                                  {[4, 5, 6, 7].map((gpio) => (
                                    <option key={gpio} value={String(gpio)}>
                                      GPIO{gpio}
                                    </option>
                                  ))}
                                </select>
                              </label>
                              <label>
                                <span>复位（Reset）通道</span>
                                <select
                                  value={device.resetChannel}
                                  onChange={(event) =>
                                    setDevice({ ...device, resetChannel: event.target.value })
                                  }
                                >
                                  <option value="">不配置（禁用复位脉冲）</option>
                                  {[4, 5, 6, 7].map((gpio) => (
                                    <option key={gpio} value={String(gpio)}>
                                      GPIO{gpio}
                                    </option>
                                  ))}
                                </select>
                              </label>
                            </>
                          )}
                          <label>
                            <span>有效电平</span>
                            <select
                              value={selectedCh347Power.activeHigh ? 'high' : 'low'}
                              onChange={(event) =>
                                setDevice(
                                  withCh347Power(device, {
                                    activeHigh: event.target.value === 'high',
                                  })
                                )
                              }
                            >
                              <option value="high">高电平吸合</option>
                              <option value="low">低电平吸合</option>
                            </select>
                          </label>
                        </>
                      )}
                    </div>
                    <div className="relay-power-actions">
                      <button
                        disabled={!device.id || powerBusy || selectedPowerType === 'none'}
                        onClick={() => powerAction('power_on')}
                      >
                        上电
                      </button>
                      <button
                        disabled={!device.id || powerBusy || selectedPowerType === 'none'}
                        onClick={() => powerAction('power_off')}
                      >
                        断电
                      </button>
                      <button
                        disabled={!device.id || powerBusy || selectedPowerType === 'none'}
                        onClick={powerCycle}
                      >
                        断电重启
                      </button>
                      <button
                        disabled={!device.id || powerBusy || selectedPowerType === 'none'}
                        onClick={() => powerAction('get_state')}
                      >
                        读取状态
                      </button>
                      <output>
                        {powerBusy
                          ? '执行中…'
                          : powerResult || '保存配置后可在这里测试，也可由 LYNX MCP 调用。'}
                      </output>
                    </div>
                  </div>
                  <div className="board-binding-summary">
                    <span>厂家：{vendorLabel(device.chipVendor)}</span>
                    <span>协议：{flashProtocolLabel(device.chipVendor)}</span>
                    <span>
                      烧录器：
                      {CHIP_VENDORS.find((item) => item.value === device.chipVendor)?.flasher ||
                        '—'}
                    </span>
                    <span>串口：{device.serialPort || '—'}</span>
                    <span>USB：{formatUsbBinding(device.usbBinding)}</span>
                    <span>ADB：{device.adbSerial || '—'}</span>
                  </div>
                  <div className="board-save-actions">
                    <button
                      data-testid="linux-board-save"
                      className="board-primary"
                      disabled={savingBoard}
                      onClick={saveBoard}
                    >
                      {savingBoard ? '保存中…' : '保存单板与接口'}
                    </button>
                    {boardSaveFeedback && (
                      <output
                        className={boardSaveFeedback.startsWith('保存失败') ? 'is-error' : ''}
                        data-testid="linux-board-save-feedback"
                        role={boardSaveFeedback.startsWith('保存失败') ? 'alert' : 'status'}
                      >
                        {boardSaveFeedback}
                      </output>
                    )}
                  </div>
                </section>

                <section className="workbench-card sdk-editor">
                  <fieldset className="ssh-host-settings" disabled={testingHost || savingChannel}>
                    <div className="board-section-title">
                      <div>
                        <small>SDK CHANNELS</small>
                        <h3>远端 SDK 与隧道</h3>
                      </div>
                      <button
                        disabled={!device.id}
                        onClick={() => {
                          const channelName = `${device.name || '单板'} SDK ${
                            boardChannels.length + 1
                          }`;
                          setHost({ ...emptyHost });
                          setCredentials({});
                          setProject({
                            ...emptyProject,
                            deviceId: device.id,
                            hostId: 0,
                            name: channelName,
                            mcpServerName: mcpName(channelName),
                            remoteForwardPort: nextPort(
                              projects.map((item) => item.remoteForwardPort),
                              project.remoteForwardPort,
                              18765
                            ),
                            localGatewayPort: nextPort(
                              projects.map((item) => item.localGatewayPort),
                              project.localGatewayPort,
                              8765
                            ),
                          });
                        }}
                      >
                        ＋ 添加 SDK 通道
                      </button>
                    </div>

                    {!!boardChannels.length && (
                      <div className="sdk-tabs">
                        {boardChannels.map((item) => (
                          <button
                            key={item.id}
                            className={item.id === project.id ? 'is-active' : ''}
                            onClick={() => {
                              setProject({ ...item, mcpServerName: mcpName(item.name) });
                              const linkedHost = hosts.find(
                                (candidate) => candidate.id === item.hostId
                              );
                              if (linkedHost) setHost(linkedHost);
                            }}
                          >
                            {item.name}
                          </button>
                        ))}
                      </div>
                    )}

                    <div className="board-form-grid">
                      <label>
                        <span>SDK 通道名称</span>
                        <input
                          value={project.name}
                          onChange={(event) => {
                            const name = event.target.value;
                            setProject({ ...project, name, mcpServerName: mcpName(name) });
                          }}
                        />
                      </label>
                      <label>
                        <span>远端终端默认工具</span>
                        <select
                          value={project.aiProvider}
                          onChange={(event) =>
                            setProject({
                              ...project,
                              aiProvider: event.target.value as
                                'codex' | 'claude_code' | 'deepseek_harness',
                            })
                          }
                        >
                          <option value="codex">Codex</option>
                          <option value="claude_code">Claude Code</option>
                          <option value="deepseek_harness">DeepSeek Harness</option>
                        </select>
                      </label>
                      <label>
                        <span>主机名称</span>
                        <input
                          placeholder="例如：SDK 编译服务器"
                          value={host.name}
                          onChange={(event) => setHost({ ...host, name: event.target.value })}
                        />
                      </label>
                      <label>
                        <span>远端主机 IP / 域名</span>
                        <input
                          placeholder="例如：192.168.1.100"
                          value={host.address}
                          onChange={(event) => setHost({ ...host, address: event.target.value })}
                        />
                      </label>
                      <label>
                        <span>SSH 用户</span>
                        <input
                          value={host.username}
                          onChange={(event) => setHost({ ...host, username: event.target.value })}
                        />
                      </label>
                      <label>
                        <span>SSH 端口</span>
                        <input
                          type="number"
                          value={host.port}
                          onChange={(event) =>
                            setHost({ ...host, port: Number(event.target.value) })
                          }
                        />
                      </label>
                      <label className="is-wide">
                        <span>远端 SDK 目录（绝对路径）</span>
                        <input
                          placeholder="例如：/home/ubuntu/A133-Tina5.0-v0.9"
                          value={project.sdkDir}
                          onChange={(event) =>
                            setProject({ ...project, sdkDir: event.target.value })
                          }
                        />
                      </label>
                      <label className="is-wide">
                        <span>SSH 密码</span>
                        <input
                          type="password"
                          autoComplete="new-password"
                          placeholder={
                            host.id || matchingHost
                              ? '已匹配相同 SSH 主机，将自动复用已保存凭证'
                              : '填写后安全保存；不填则自动使用系统 SSH 密钥'
                          }
                          value={credentials.password || ''}
                          onChange={(event) => {
                            const password = event.target.value;
                            setCredentials({ ...credentials, password });
                            setHost({ ...host, authType: password ? 'password' : host.authType });
                          }}
                        />
                        {matchingHost && !host.id && (
                          <em>已匹配已有 SSH 主机“{matchingHost.name}”，不会重复保存或获取凭证</em>
                        )}
                      </label>
                      <label className="is-wide">
                        <span>已信任的 SSH 主机指纹</span>
                        <output className="ssh-host-fingerprint">
                          {displaySshFingerprint((matchingHost ?? host).hostFingerprint) ||
                            '尚未信任或保存格式无效'}
                        </output>
                        <em>
                          更换服务器或服务器重装后，点击下方按钮核验新旧指纹；确认后再保存通道。
                        </em>
                      </label>
                      <label>
                        <span>远端 MCP 实际地址（自动避让）</span>
                        <output>
                          {remoteEndpoint || '连接验证通过后显示，未使用保存端口推测地址'}
                        </output>
                        <em>多人连接同一主机时，由 SSH 服务端原子分配空闲端口并自动保存</em>
                      </label>
                      <label>
                        <span>MCP 名称（根据通道名称自动生成）</span>
                        <input
                          className={nameConflict ? 'has-conflict' : ''}
                          value={project.mcpServerName}
                          readOnly
                        />
                        {nameConflict && <em>同名 MCP 已存在，保存时会自动追加序号</em>}
                      </label>
                    </div>
                    <div className="workbench-row">
                      <button onClick={testHost}>
                        {testingHost ? '正在核验…' : '核验 SSH 指纹并测试连接'}
                      </button>
                      <button
                        className="board-primary"
                        disabled={savingChannel}
                        onClick={saveChannel}
                      >
                        {savingChannel ? '正在保存并连接…' : '保存通道并自动连接'}
                      </button>
                    </div>
                    {channelSaveFeedback && (
                      <output className="channel-save-feedback">{channelSaveFeedback}</output>
                    )}
                  </fieldset>
                </section>

                <section className="workbench-card channel-map sdk-mcp-settings">
                  <div className="board-section-title">
                    <h3>SDK 通道</h3>
                  </div>
                  <div className="mcp-topology-panel">
                    <h4>连接拓扑</h4>
                    <div className="channel-topology">
                      <div className="channel-node is-local">
                        <small>
                          <i
                            className={localConnection.endpoint ? 'is-verified' : 'is-unverified'}
                          />
                          LOCAL NODE · 01
                        </small>
                        <strong>LYNX 本机服务</strong>
                        <span>单板：{device.name || '尚未选择'}</span>
                        <span
                          className={`mcp-node-status${localConnection.endpoint ? ' is-ready' : ''}`}
                        >
                          {localConnection.endpoint ? '本机访问已验证' : '本机未验证'}
                        </span>
                      </div>
                      <div className={`channel-link${remoteEndpoint ? '' : ' is-idle'}`}>
                        <span>SSH 加密隧道</span>
                        <div className="mcp-tunnel-line">
                          <LynxIcon name="lock" size={20} />
                        </div>
                        <span>LYNX 自动维护</span>
                      </div>
                      <div className="channel-node is-remote">
                        <small>
                          <i className={remoteEndpoint ? 'is-verified' : 'is-unverified'} />
                          REMOTE NODE · 02
                        </small>
                        <strong>远端 SDK 主机</strong>
                        <span>
                          {host.name || '尚未配置'} · IP：{host.address || '未填写'}
                        </span>
                        <span className={`mcp-node-status${remoteEndpoint ? ' is-ready' : ''}`}>
                          {remoteEndpoint ? '远端访问已验证' : '远端未验证'}
                        </span>
                      </div>
                    </div>
                    <div className="channel-note">
                      <LynxIcon name="info" size={16} />
                      保存远端 SDK 主机信息后，LYNX 自动建立并维护连接。
                    </div>
                  </div>
                  <div className="mcp-client-picker" role="group" aria-label="MCP 接入工具">
                    <strong>AI 工具</strong>
                    <div className="mcp-client-options">
                      {MCP_CLIENTS.map((client) => (
                        <button
                          key={client.id}
                          type="button"
                          data-testid={`linux-mcp-client-${client.id}`}
                          aria-pressed={mcpClient === client.id}
                          onClick={() =>
                            setMcpClients((current) => ({
                              ...current,
                              [mcpClientScope]: client.id,
                            }))
                          }
                        >
                          <span className="mcp-client-radio" aria-hidden="true" />
                          {client.label}
                        </button>
                      ))}
                    </div>
                    <span className="mcp-picker-hint">切换工具，生成对应的连接配置。</span>
                  </div>
                  <div className="mcp-peer-cards">
                    {[
                      {
                        side: 'local',
                        title: localShell === 'powershell' ? 'Windows 本机连接' : '本机连接',
                        endpoint: localConnection.endpoint,
                        config: localClientConfig,
                        help:
                          localShell === 'powershell'
                            ? '在运行 LYNX 的 Windows 电脑上使用。'
                            : '在运行 LYNX 的这台电脑上使用。',
                        commandLabel: configOnly
                          ? 'MCP 连接配置'
                          : localShell === 'powershell'
                            ? 'PowerShell 启动命令'
                            : '本机启动命令',
                        detail:
                          localMcpError ||
                          (!localConnection.endpoint ? currentLocalStatus?.message : '') ||
                          '',
                      },
                      {
                        side: 'remote',
                        title: '远端 SDK 连接',
                        endpoint: remoteEndpoint,
                        config: remoteClientConfig,
                        help: '在远端 Ubuntu 的目标 SDK 目录中使用。',
                        commandLabel: configOnly ? 'MCP 连接配置' : '远端启动命令',
                        detail:
                          remoteMcpError || (!remoteEndpoint ? tunnelStatus?.message : '') || '',
                      },
                    ].map(({ side, title, endpoint, config, help, commandLabel, detail }) => (
                      <section
                        className="channel-command mcp-connection-card"
                        key={side}
                        data-testid={`linux-${side}-mcp-card`}
                      >
                        <div className="mcp-card-heading">
                          <h4>{title}</h4>
                          <span
                            className={`mcp-ready-status is-${endpoint ? 'connected' : 'stopped'}`}
                          >
                            {endpoint
                              ? 'HTTP 验证通过'
                              : side === 'remote' && tunnelStatus?.state === 'connecting'
                                ? '正在连接'
                                : side === 'remote' && tunnelStatus?.state === 'verifying'
                                  ? '正在验证'
                                  : side === 'remote' && tunnelStatus?.state === 'reconnecting'
                                    ? '正在重连'
                                    : '尚未验证'}
                          </span>
                        </div>
                        <p className="mcp-location-help">{help}</p>
                        <div className="mcp-address-field">
                          <span className="mcp-field-label">
                            {side === 'local' ? '本机 MCP 地址' : '远端 MCP 地址'}
                          </span>
                          <div className="mcp-address-row">
                            <code data-testid={`linux-${side}-mcp-address`}>
                              {endpoint || '连接验证通过后显示当前地址'}
                            </code>
                            <button
                              data-testid={`linux-copy-${side}-mcp-address`}
                              aria-label={side === 'local' ? '复制本机地址' : '复制远端地址'}
                              disabled={!endpoint}
                              onClick={() =>
                                copy(endpoint, side === 'local' ? '本机 MCP 地址' : '远端 MCP 地址')
                              }
                            >
                              复制地址
                            </button>
                          </div>
                        </div>
                        <div className="mcp-command-field">
                          <div className="mcp-command-heading">
                            <span className="mcp-field-label">{commandLabel}</span>
                            <button
                              data-testid={`linux-copy-${side}-mcp-command`}
                              disabled={!config.value}
                              aria-label={`复制${side === 'local' ? '本机' : '远端'}${configOnly ? '配置' : '命令'}`}
                              onClick={() => copy(config.value, commandLabel)}
                            >
                              {configOnly ? '复制配置' : '复制命令'}
                            </button>
                          </div>
                          <code data-testid={`linux-${side}-mcp-command`}>
                            {config.value || '服务验证通过后显示当前配置。'}
                          </code>
                        </div>
                        {!!detail && !endpoint && (
                          <details className="channel-connection-details">
                            <summary>查看连接详情</summary>
                            <code>{detail}</code>
                          </details>
                        )}
                      </section>
                    ))}
                  </div>
                  <div className="channel-note mcp-scope-warning">
                    <LynxIcon name="info" size={16} />
                    两端的 127.0.0.1 分别属于各自电脑，本机与远端地址不可混用。
                  </div>
                  <div className="mcp-config-footnote">
                    {mcpClient === 'zcode'
                      ? '在 ZCode 的 MCP 设置中导入当前配置；仅包含当前 SDK 通道，不自动修改工具配置。'
                      : mcpClient === 'custom'
                        ? '在支持 HTTP MCP 的工具中导入当前配置；连接范围仅限当前 SDK 通道。'
                        : mcpClient === 'deepseek_harness'
                          ? '通过本次会话的临时 Cordis patch 接入，退出后删除，不修改用户级 DSH 配置。'
                          : '配置仅对本次 AI 进程生效，并仅挂载当前 SDK 通道。'}
                  </div>
                  <details className="mcp-generic-config" data-testid="linux-generic-mcp-config">
                    <summary>
                      <span>其他工具可复制通用 MCP 配置。</span>
                      <span className="mcp-disclosure-action">查看通用配置</span>
                    </summary>
                    <div className="mcp-generic-columns">
                      {[
                        { side: 'local', title: '本机通用配置', value: genericLocalConfig },
                        { side: 'remote', title: '远端通用配置', value: genericRemoteConfig },
                      ].map(({ side, title, value }) => (
                        <div key={side}>
                          <div className="mcp-command-heading">
                            <strong>{title}</strong>
                            <button
                              disabled={!value}
                              data-testid={`linux-copy-${side}-generic-config`}
                              onClick={() => copy(value, title)}
                            >
                              复制配置
                            </button>
                          </div>
                          <pre>
                            <code data-testid={`linux-${side}-generic-config`}>
                              {value || '服务验证通过后显示配置。'}
                            </code>
                          </pre>
                        </div>
                      ))}
                    </div>
                  </details>
                </section>

                {device.id && (
                  <section className="workbench-card board-danger-zone">
                    <div>
                      <strong>删除单板</strong>
                      <span>同时删除该单板的 SDK 通道；共享 SSH 主机和凭证会保留。</span>
                    </div>
                    <button
                      onClick={() =>
                        deleteBoard(device).catch((value) => setError(formatErrorMessage(value)))
                      }
                    >
                      删除当前单板
                    </button>
                  </section>
                )}

                {message && <div className="board-message">{message}</div>}
                {error && <div className="workbench-error">{error}</div>}
              </div>
            </div>
          </div>
        )}
      </main>
    </div>
  );
};
