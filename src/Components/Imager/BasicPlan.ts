import type {
  WorkbenchCommandMap,
  WorkbenchDevice,
  WorkbenchFlasherDevice,
} from '../../Platform/IPC/contracts/Workbench';

type FlashOptions = NonNullable<WorkbenchCommandMap['workbench_start_flash']['args']['options']>;
export type BasicRoute = 'vendor' | NonNullable<FlashOptions['mainlineRoute']>;

export function flashOptions(
  route: BasicRoute,
  vendor: WorkbenchDevice['chipVendor'],
  loaderPath: string
): FlashOptions {
  if (route === 'vendor' && vendor !== 'allwinner') return { systemType: 'vendor' };
  if (route === 'vendor')
    return { systemType: 'vendor', mode: 'full_erase', verify: true, postFlashAction: 'reboot' };
  const options: FlashOptions = {
    systemType: 'mainline',
    mainlineRoute: route,
    loaderPath: loaderPath.trim(),
  };
  if (route === 'rockchip_raw_emmc')
    return { ...options, mode: 'full_erase', verify: true, postFlashAction: 'reboot' };
  return route === 'mainline_spinor' ? { ...options, postFlashAction: 'reboot' } : options;
}

export function uniqueUsbTarget(binding: string, kind: string, found: WorkbenchFlasherDevice[]) {
  const candidates = found.filter((device) => {
    if (device.flasherKind !== kind) return false;
    if (kind !== 'openixcli') return binding === device.key;
    if (binding.toLowerCase() === device.key.toLowerCase()) return true;
    // Older libusb bindings kept only the leaf port. Match the native scanner's unique-target rule.
    const legacy = /^location:libusb:([^:]+):([^. :]+)$/.exec(binding);
    const physical = /^location:libusb:([^:]+):([^:]+)$/.exec(device.key);
    return (
      !!legacy &&
      !!physical &&
      legacy[1] === physical[1] &&
      legacy[2] === physical[2].split('.').pop()
    );
  });
  if (candidates.length !== 1)
    throw new Error(
      candidates.length
        ? '多个物理端口符合已保存的绑定。请在「高级功能」中重新绑定当前设备。'
        : '未检测到绑定的 USB 接口。请核对物理端口、连接线和板卡的烧录模式。'
    );
  return candidates[0];
}
