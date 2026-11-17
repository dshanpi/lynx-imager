import type { McuBoardConfig, McuChipSeries, McuChipVendor } from '../../Platform/IPC';

export const MCU_CHIP_VENDORS: Array<{ value: McuChipVendor; label: string }> = [
  { value: 'st', label: 'STMicroelectronics（意法半导体）' },
  { value: 'renesas', label: 'Renesas（瑞萨电子）' },
];

export const MCU_CHIP_SERIES: Record<
  McuChipVendor,
  Array<{ value: McuChipSeries; label: string; model: string }>
> = {
  st: [
    { value: 'stm32f1', label: 'STM32 F1 系列', model: 'STM32F103C8T6' },
    { value: 'stm32f4', label: 'STM32 F4 系列', model: 'STM32F407ZGT6' },
    { value: 'stm32h7', label: 'STM32 H7 系列', model: 'STM32H750ZBT6' },
  ],
  renesas: [
    { value: 'ra6', label: 'Renesas RA6 系列', model: 'R7FA6M5BF2CBG' },
    { value: 'ra8', label: 'Renesas RA8 系列', model: 'R7FA8M1AF' },
  ],
};

export interface McuSerialIdentity {
  name: string;
  vid: number;
  pid: number;
  serialNumber: string;
}

export function mcuBoardSpecs(board: McuBoardConfig): [string, string] {
  const backend =
    board.probeBackend === 'cubeProgrammer'
      ? 'STM32Cube'
      : board.probeBackend === 'pyOcd'
        ? 'pyOCD'
        : board.probeBackend === 'jLink'
          ? 'J-Link'
          : 'OpenOCD';
  return [board.mcuModel, `${backend} · SWD ${board.swdFrequencyKhz} kHz`];
}

export function mcuBoardOnline(board: McuBoardConfig, ports: McuSerialIdentity[]) {
  return ports.some((port) => {
    if (board.serialNumber) return port.serialNumber === board.serialNumber;
    if (board.serialVid || board.serialPid)
      return (
        (!board.serialVid || port.vid === board.serialVid) &&
        (!board.serialPid || port.pid === board.serialPid)
      );
    return !!board.serialPort && port.name === board.serialPort;
  });
}

export function applyMcuChipSeries(
  board: McuBoardConfig,
  vendor: McuChipVendor,
  series: McuChipSeries
) {
  const profile = MCU_CHIP_SERIES[vendor].find((item) => item.value === series)!;
  const isRenesas = vendor === 'renesas';
  const openOcdFamily =
    series === 'stm32f1' ? 'stm32f1x' : series === 'stm32f4' ? 'stm32f4x' : 'stm32h7x';
  return {
    ...board,
    chipVendor: vendor,
    chipSeries: series,
    mcuModel: profile.model,
    probeBackend: isRenesas ? ('pyOcd' as const) : ('openOcd' as const),
    openocdInterfaceConfig: isRenesas ? 'interface/cmsis-dap.cfg' : 'interface/stlink.cfg',
    openocdTargetConfig: isRenesas ? '' : `target/${openOcdFamily}.cfg`,
    openocdFlashBank: isRenesas ? '' : `${openOcdFamily}.flash`,
    pyocdTarget: series === 'ra6' ? 'R7FA6M5BF' : series === 'ra8' ? 'R7FA8M1AF' : '',
    jlinkDevice: profile.model,
  };
}

export function normalizeMcuBoardIdentity(board: McuBoardConfig): McuBoardConfig {
  const model = board.mcuModel.trim().toUpperCase();
  const inferredVendor: McuChipVendor =
    model.startsWith('RA') || model.startsWith('R7FA') ? 'renesas' : 'st';
  const vendor = MCU_CHIP_VENDORS.some((item) => item.value === board.chipVendor)
    ? board.chipVendor
    : inferredVendor;
  const inferredSeries: McuChipSeries = model.startsWith('STM32F1')
    ? 'stm32f1'
    : model.startsWith('STM32F4')
      ? 'stm32f4'
      : model.startsWith('STM32H7')
        ? 'stm32h7'
        : model.startsWith('RA8') || model.startsWith('R7FA8')
          ? 'ra8'
          : vendor === 'renesas'
            ? 'ra6'
            : 'stm32h7';
  const series = MCU_CHIP_SERIES[vendor].some((item) => item.value === board.chipSeries)
    ? board.chipSeries
    : inferredSeries;
  return { ...board, chipVendor: vendor, chipSeries: series };
}

export function createDefaultMcuBoard(): McuBoardConfig {
  return {
    id: '',
    name: '',
    boardImagePath: '',
    chipVendor: 'st',
    chipSeries: 'stm32h7',
    mcuModel: 'STM32H750ZBT6',
    probeBackend: 'openOcd',
    probeSerial: '',
    serialPort: '',
    serialVid: 0,
    serialPid: 0,
    serialNumber: '',
    serialInterfaceNumber: null,
    baudRate: 115200,
    toolPath: '',
    swdFrequencyKhz: 1000,
    openocdInterfaceConfig: 'interface/stlink.cfg',
    openocdTargetConfig: 'target/stm32h7x.cfg',
    openocdBoardConfig: '',
    openocdFlashBank: 'stm32h7x.bank1.cpu0',
    pyocdTarget: '',
    pyocdPackPath: '',
    jlinkDevice: 'STM32H750ZBT6',
    externalLoaderPath: '',
    ospiAddressStart: 0,
    ospiSizeBytes: 0,
    externalFlashModel: '',
    externalSramModel: '',
    uartInstance: '',
    uartTxPin: '',
    uartRxPin: '',
    bootSuccessText: '',
    bootFailureText: '',
    bootTimeoutMs: 30000,
    mcpHostAddress: '',
    mcpHostPort: 22,
    mcpHostUsername: 'ubuntu',
    mcpAuthType: 'password',
    mcpPrivateKeyPath: '',
    mcpHostFingerprint: '',
    mcpLocalPort: 8767,
    mcpRemotePort: 28767,
    mcpServerName: 'lynx-mcu-board',
    mcuSdkDir: '',
    mcuBuildCommand: '',
    mcuFirmwarePath: '',
    powerControllerType: '',
    powerDeviceIndex: 0,
    powerGpio: 4,
    powerActiveHigh: true,
  };
}

export function parseMcuUnsigned(value: string) {
  const trimmed = value.trim();
  if (!/^(?:0x[0-9a-f]+|[0-9]+)$/i.test(trimmed)) return null;
  const parsed = Number.parseInt(trimmed, trimmed.toLowerCase().startsWith('0x') ? 16 : 10);
  return Number.isSafeInteger(parsed) && parsed >= 0 ? parsed : null;
}

export function mcuMcpName(value: string) {
  const slug = value
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, '-')
    .replace(/^-+|-+$/g, '');
  return `lynx-mcu-${slug.replace(/^lynx-(?:mcu-)?/, '') || 'board'}`;
}

export function formatMcuCapacity(bytes: number) {
  if (!bytes) return '未识别';
  if (bytes >= 1024 * 1024) return `${bytes / 1024 / 1024} MiB`;
  return `${bytes / 1024} KiB`;
}

export function externalFlashCapacity(model: string, configuredBytes: number) {
  if (configuredBytes) return formatMcuCapacity(configuredBytes);
  if (/GD25Q256/i.test(model)) return '32 MiB';
  return model.trim() ? '容量待识别' : '未配置';
}

export function externalSramCapacity(model: string) {
  if (/IS62WV51216/i.test(model)) return '1 MiB';
  return model.trim() ? '容量待识别' : '未配置';
}
