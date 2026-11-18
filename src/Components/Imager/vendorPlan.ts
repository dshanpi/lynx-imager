import { invokeCommand } from '../../Platform/IPC';
import type {
  WorkbenchCommandMap,
  WorkbenchDevice,
  WorkbenchFlasherDevice,
  WorkbenchVendorInspection,
  WorkbenchVendorRoute,
} from '../../Platform/IPC/contracts/Workbench';

export type VendorRoute = WorkbenchVendorRoute;
export type VendorInspection = WorkbenchVendorInspection;
export interface VendorSelection {
  route: VendorRoute;
  medium: string;
  alt: string;
  address: string;
  targetKey: string;
  eraseConfirmed: boolean;
}
type Options = NonNullable<WorkbenchCommandMap['workbench_start_flash']['args']['options']>;
type Family = WorkbenchDevice['chipVendor'];
export const vendorRoutes: Partial<Record<Family, Array<{ route: VendorRoute; label: string }>>> = {
  rockchip: [
    { route: 'rk-update', label: '厂商 update.img' },
    { route: 'rk-emmc', label: '主线 eMMC 整盘' },
  ],
  nxp: [
    { route: 'nxp-uuu', label: 'UUU 脚本' },
    { route: 'nxp-configured', label: '单板已保存配置' },
  ],
  st: [
    { route: 'st-dfuse', label: 'DfuSe 容器' },
    { route: 'st-raw', label: 'RAW 二进制' },
  ],
  kendryte: [
    { route: 'k230-kdimage', label: 'KDImage 分区包' },
    { route: 'k230-raw', label: 'RAW 整盘' },
  ],
};
export function defaultVendorSelection(vendor: Family): VendorSelection {
  return {
    route: vendorRoutes[vendor]?.[0].route ?? 'rk-update',
    medium: '',
    alt: '',
    address: '',
    targetKey: '',
    eraseConfirmed: false,
  };
}
export function vendorNeedsLoader(route: VendorRoute) {
  return route === 'rk-emmc';
}
export async function inspectVendorImage(
  route: VendorRoute,
  path: string
): Promise<VendorInspection> {
  return invokeCommand('imager_inspect_vendor_image', { route, path });
}
export const k230Media = [
  ['emmc', 'eMMC'],
  ['sdcard', 'SD 卡'],
  ['spi_nand', 'SPI NAND'],
  ['spi_nor', 'SPI NOR'],
] as const;
export function vendorFlashOptions(
  s: VendorSelection,
  loaderPath: string,
  inspection?: VendorInspection | null
): Options {
  const base: Options = {
    systemType: s.route === 'rk-emmc' ? 'mainline' : 'vendor',
    vendorSetup: { route: s.route },
  };
  if (s.route === 'nxp-uuu')
    return { ...base, vendorSetup: { route: s.route, dependencies: inspection?.dependencies } };
  if (s.route === 'rk-emmc')
    return {
      ...base,
      loaderPath: loaderPath.trim(),
      mainlineRoute: 'rockchip_raw_emmc',
      verify: true,
      mode: 'full_erase',
      postFlashAction: 'reboot',
      vendorSetup: { route: s.route, targetKey: s.targetKey, eraseConfirmed: s.eraseConfirmed },
    };
  if (s.route.startsWith('k230-'))
    return { ...base, vendorSetup: { route: s.route, medium: s.medium, targetKey: s.targetKey } };
  if (s.route.startsWith('st-'))
    return {
      ...base,
      vendorSetup: {
        route: s.route,
        alt: /^\d+$/.test(s.alt) ? Number(s.alt) : undefined,
        targetKey: s.targetKey,
        address: s.route === 'st-raw' ? s.address.trim() : undefined,
      },
    };
  return base;
}
export function vendorGate(
  s: VendorSelection,
  inspection: VendorInspection | null | undefined,
  usb?: WorkbenchFlasherDevice | null,
  purpose: 'files' | 'target' = 'files',
  profile?: WorkbenchDevice | null
): string {
  if (!inspection || inspection.route !== s.route) return '请选择该路线的输入文件并完成分析';
  if (inspection.blockers.length) return inspection.blockers[0];
  if (s.route === 'st-dfuse' && inspection.alternates.length !== 1)
    return '当前基础流程仅支持写入一个明确的 DFU Alternate';
  if (purpose === 'files') return '';
  if (!usb) return '请连接并选择本次操作的 USB 目标';
  if (
    s.route.startsWith('rk-') &&
    (usb.flasherKind !== 'openrockcli' || !['loader', 'maskrom'].includes(usb.mode.toLowerCase()))
  )
    return '请选择处于 Loader 或 Maskrom 模式的 RK 设备';
  if (s.route === 'rk-emmc' && (!s.eraseConfirmed || s.targetKey !== usb.key))
    return '请确认覆盖此设备的整个 eMMC 用户区';
  if (s.route.startsWith('nxp-') && usb.flasherKind !== 'uuu')
    return '请选择 UUU 检测到的 i.MX 目标';
  if (s.route === 'nxp-configured' && profile) {
    try {
      const a: unknown = JSON.parse(profile.flasherArgs || '[]');
      if (
        !Array.isArray(a) ||
        a.length !== 2 ||
        a[0] !== '-b' ||
        ![
          'emmc',
          'sd',
          'emmc_all',
          'sd_all',
          'spl',
          'qspi',
          'nand',
          'nvme_all',
          'fspinand',
        ].includes(a[1])
      ) {
        return '请先在单板设置中保存适用于单个输入文件的 UUU 内置 -b 配方';
      }
    } catch {
      return '单板 UUU 参数格式无效';
    }
  }
  if (s.route.startsWith('k230-')) {
    if (usb.flasherKind !== 'k230') return '请选择 K230 设备';
    if (!k230Media.some(([id]) => id === s.medium) || s.targetKey !== usb.key)
      return '请选择这块单板实际使用的存储介质';
  }
  if (s.route.startsWith('st-')) {
    if (usb.flasherKind !== 'dfu-util') return '请选择 USB DFU 设备';
    if (
      !/^\d+$/.test(s.alt) ||
      s.targetKey !== usb.key ||
      usb.dfuAlternates?.filter((a) => a.number === Number(s.alt)).length !== 1
    )
      return '请从这次设备枚举的 Alternate 中选择唯一目标';
    if (s.route === 'st-dfuse' && !inspection.alternates.includes(Number(s.alt)))
      return '所选 Alternate 与容器内目标不一致';
    if (s.route === 'st-raw' && !/^0x[\da-f]{1,8}$/i.test(s.address.trim()))
      return '请按板卡文档填写 32 位十六进制目的地址';
  }
  return '';
}
