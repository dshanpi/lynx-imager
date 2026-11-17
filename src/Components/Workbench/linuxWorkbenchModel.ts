import type { WorkbenchDevice, WorkbenchHost, WorkbenchProject } from '../../Platform/IPC/Commands';
import platformCatalog from '../../../src-tauri/resources/workbench-platforms.json';

export const LINUX_CHIP_VENDORS = platformCatalog as Array<{
  value: WorkbenchDevice['chipVendor'];
  label: string;
  flasher: string;
  protocol: string;
}>;

export function linuxVendorLabel(value: WorkbenchDevice['chipVendor']) {
  return LINUX_CHIP_VENDORS.find((item) => item.value === value)?.label ?? value;
}

export function formatLinuxBytes(value: number | null) {
  if (value == null || !Number.isFinite(value) || value < 0) return '—';
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
  let size = value;
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit += 1;
  }
  return `${size >= 100 || unit === 0 ? size.toFixed(0) : size.toFixed(1)} ${units[unit]}`;
}

export function formatLinuxUptime(value: number | null) {
  if (value == null || value < 0) return '—';
  const days = Math.floor(value / 86400);
  const hours = Math.floor((value % 86400) / 3600);
  const minutes = Math.floor((value % 3600) / 60);
  return days ? `${days} 天 ${hours} 小时` : `${hours} 小时 ${minutes} 分`;
}

export function linuxUsedPercent(total: number | null, available: number | null) {
  if (total == null || available == null || total <= 0 || available > total) return null;
  return Math.max(0, Math.min(100, ((total - available) / total) * 100));
}

export function linuxVendorMark(value: WorkbenchDevice['chipVendor']) {
  return (
    (
      { rockchip: 'RK', allwinner: 'AW', nxp: 'NXP', kendryte: 'K', st: 'ST' } as Partial<
        Record<WorkbenchDevice['chipVendor'], string>
      >
    )[value] ?? 'SOC'
  );
}

export function linuxBoardSpecs(board: WorkbenchDevice): [string, string] {
  const identity = `${board.name} ${board.chipVendor}`.toLowerCase();
  if (identity.includes('rk3506')) return ['三核 Cortex-A7', '2× USB 2.0 · 2× RMII'];
  if (identity.includes('t153')) return ['四核 Cortex-A7 + E907', 'DDR3/DDR4 · 工业控制'];
  return [
    linuxVendorLabel(board.chipVendor),
    `${board.serialPort ? 'UART 已绑定' : 'UART 未绑定'} · ${formatUsbBinding(board.usbBinding)}`,
  ];
}

export function formatUsbBinding(binding: string) {
  if (!binding) return 'USB 未绑定';
  if (binding.startsWith('location:')) return '物理 USB 口已绑定';
  return `USB ${binding}（旧绑定，建议重新选择）`;
}

export function linuxVendorHintFromName(name: string): WorkbenchDevice['chipVendor'] | undefined {
  const normalized = name.trim().toLowerCase();
  const compact = normalized.replace(/[^a-z0-9]/g, '');
  if (/^rk\d/.test(compact) || normalized.includes('rockchip') || normalized.includes('瑞芯微'))
    return 'rockchip';
  if (
    normalized.includes('allwinner') ||
    normalized.includes('sunxi') ||
    normalized.includes('全志') ||
    /^(t113|t153|t507|t527|a523|d1|h3|h5|h6|h616|v853|avaota)/.test(compact)
  )
    return 'allwinner';
  if (normalized.includes('nxp') || compact.includes('imx')) return 'nxp';
  if (
    normalized.includes('kendryte') ||
    normalized.includes('canaan') ||
    normalized.includes('嘉楠') ||
    compact.includes('k230')
  )
    return 'kendryte';
  if (compact.includes('stm32mp') || normalized.includes('意法半导体')) return 'st';
  return undefined;
}

export function linuxFlashProtocolLabel(value: WorkbenchDevice['chipVendor']) {
  return LINUX_CHIP_VENDORS.find((item) => item.value === value)?.protocol ?? '厂商 USB 协议';
}

export function readK230Medium(value: string) {
  try {
    const args = JSON.parse(value) as string[];
    const index = args.indexOf('--medium');
    return index >= 0 ? (args[index + 1] ?? 'emmc') : 'emmc';
  } catch {
    return 'emmc';
  }
}

export function writeK230Args(medium: string) {
  return JSON.stringify(['--medium', medium]);
}

export type AllwinnerFlashRoute =
  'vendor_image' | 'mainline_emmc' | 'mainline_nand' | 'mainline_spinor';

function readStringArgs(value: string) {
  try {
    const parsed = JSON.parse(value) as unknown;
    return Array.isArray(parsed) && parsed.every((item) => typeof item === 'string') ? parsed : [];
  } catch {
    return [];
  }
}

export function readAllwinnerFlashOptions(value: string): {
  route: AllwinnerFlashRoute;
  loaderPath: string;
} {
  const args = readStringArgs(value);
  const loaderOption = args.includes('--lynx-raw-loader') ? '--lynx-raw-loader' : '--lynx-loader';
  const loaderIndex = args.indexOf(loaderOption);
  const loaderPath = loaderIndex >= 0 ? (args[loaderIndex + 1] ?? '') : '';
  if (args.includes('--raw') && args.includes('--lynx-spinor')) {
    return { route: 'mainline_spinor', loaderPath };
  }
  if (args.includes('--raw') && args.includes('--emmc-boot0-from-image')) {
    return { route: 'mainline_emmc', loaderPath };
  }
  if (args.includes('--lynx-loader')) return { route: 'mainline_nand', loaderPath };
  return { route: 'vendor_image', loaderPath };
}

export function writeAllwinnerFlashArgs(route: AllwinnerFlashRoute, loaderPath: string) {
  const loader = loaderPath.trim();
  if (route === 'mainline_spinor') {
    return JSON.stringify([
      '--raw',
      '--lynx-raw-loader',
      loader,
      '--lynx-spinor',
      '--post-action',
      'none',
    ]);
  }
  if (route === 'mainline_emmc') {
    return JSON.stringify(['--raw', '--lynx-raw-loader', loader, '--emmc-boot0-from-image']);
  }
  if (route === 'mainline_nand') {
    return JSON.stringify([
      '--lynx-loader',
      loader,
      '--verify',
      'true',
      '--mode',
      'full_erase',
      '--post-action',
      'none',
    ]);
  }
  return '';
}

export function readStm32DfuOptions(value: string) {
  let args: string[] = [];
  try {
    args = JSON.parse(value) as string[];
  } catch {
    // Invalid persisted arguments are replaced as soon as the user edits a DFU field.
  }
  const option = (...names: string[]) => {
    const index = args.findIndex((item) => names.includes(item));
    return index >= 0 ? (args[index + 1] ?? '') : '';
  };
  return {
    alt: option('-a', '--alt') || '0',
    address: option('-s', '--dfuse-address'),
    reset: args.some((item) => item === '-R' || item === '--reset'),
  };
}

export function writeStm32DfuArgs(alt: string, address: string, reset: boolean) {
  const args = ['-a', alt.trim() || '0'];
  if (address.trim()) args.push('-s', address.trim());
  if (reset) args.push('-R');
  return JSON.stringify(args);
}

export const EMPTY_LINUX_DEVICE: WorkbenchDevice = {
  id: null,
  boardUid: '',
  name: '',
  boardImagePath: '',
  chipVendor: 'allwinner',
  serialPort: '',
  serialVid: 0,
  serialPid: 0,
  serialNumber: '',
  serialInterfaceNumber: null,
  usbBinding: '',
  cameraBinding: '',
  adbSerial: '',
  baudRate: 115200,
  serialConsoleType: 'auto',
  serialLoginMode: 'none',
  serialLoginUsername: 'root',
  serialLoginPrompt: 'login:',
  serialPasswordPrompt: 'Password:',
  serialShellPrompt: '#',
  serialAutoLogin: false,
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

export const EMPTY_WORKBENCH_HOST: WorkbenchHost = {
  id: null,
  name: '',
  address: '',
  port: 22,
  username: '',
  authType: 'private_key',
  privateKeyPath: '',
  hostFingerprint: '',
  defaultWorkDir: '',
  aiProvider: 'codex',
  aiExecutable: '',
  aiArgsJson: '',
};

export const EMPTY_WORKBENCH_PROJECT: WorkbenchProject = {
  id: null,
  name: '',
  hostId: 0,
  deviceId: null,
  sdkDir: '',
  buildCommand: '',
  firmwarePath: '',
  localDownloadDir: '',
  verifySha256: true,
  localGatewayPort: 8765,
  remoteForwardPort: 18765,
  mcpServerName: '',
  aiProvider: 'codex',
};
