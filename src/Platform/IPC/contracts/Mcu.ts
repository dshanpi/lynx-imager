import type { IpcCommandSpec, IpcEventSpec } from '../Types';

export interface McuToolInfo {
  available: boolean;
  executablePath: string;
  version: string;
}

export type McuProbeBackend = 'openOcd' | 'pyOcd' | 'cubeProgrammer' | 'jLink';
export type McuChipVendor = 'st' | 'renesas';
export type McuChipSeries = 'stm32f1' | 'stm32f4' | 'stm32h7' | 'ra6' | 'ra8';

export interface McuBoardConfig {
  id: string;
  name: string;
  boardImagePath: string;
  chipVendor: McuChipVendor;
  chipSeries: McuChipSeries;
  mcuModel: string;
  probeBackend: McuProbeBackend;
  probeSerial: string;
  serialPort: string;
  serialVid: number;
  serialPid: number;
  serialNumber: string;
  serialInterfaceNumber: number | null;
  baudRate: number;
  toolPath: string;
  swdFrequencyKhz: number;
  openocdInterfaceConfig: string;
  openocdTargetConfig: string;
  openocdBoardConfig: string;
  openocdFlashBank: string;
  pyocdTarget: string;
  pyocdPackPath: string;
  jlinkDevice: string;
  externalLoaderPath: string;
  ospiAddressStart: number;
  ospiSizeBytes: number;
  externalFlashModel: string;
  externalSramModel: string;
  uartInstance: string;
  uartTxPin: string;
  uartRxPin: string;
  bootSuccessText: string;
  bootFailureText: string;
  bootTimeoutMs: number;
  mcpHostAddress: string;
  mcpHostPort: number;
  mcpHostUsername: string;
  mcpAuthType: 'password' | 'private_key';
  mcpPrivateKeyPath: string;
  mcpHostFingerprint: string;
  mcpLocalPort: number;
  mcpRemotePort: number;
  mcpServerName: string;
  mcuSdkDir: string;
  mcuBuildCommand: string;
  mcuFirmwarePath: string;
  powerControllerType: '' | 'ch347_gpio';
  powerDeviceIndex: number;
  powerGpio: number;
  powerActiveHigh: boolean;
}

export interface McuOpenOcdConnection {
  executablePath?: string;
  probeSerial?: string;
  frequencyKhz: number;
  interfaceConfig: string;
  targetConfig: string;
  boardConfig: string;
}

export interface McuOpenOcdToolInfo extends McuToolInfo {
  bundled: boolean;
}

export interface McuPyOcdConnection {
  executablePath?: string;
  probeSerial?: string;
  frequencyKhz: number;
  target: string;
  packPath: string;
}

export interface McuJLinkConnection {
  executablePath?: string;
  probeSerial?: string;
  frequencyKhz: number;
  device: string;
}

export interface McuStLinkProbe {
  index: number;
  serialNumber: string;
  firmwareVersion: string;
  boardName: string;
}

export interface McuStLinkConnection {
  executablePath?: string;
  probeSerial?: string;
  probeIndex?: number;
  frequencyKhz: number;
  connectMode: 'normal' | 'underReset' | 'hotPlug';
}

export interface McuTargetStatus {
  connected: boolean;
  deviceName: string;
  deviceId: string;
  voltage: number | null;
  coreState: string;
  output: string;
}

export interface McuOperationResult {
  success: boolean;
  exitCode: number;
  stdout: string;
  stderr: string;
}

export interface McuMcpGatewayStatus {
  running: boolean;
  channelKind: 'mcu';
  boardId: string;
  port: number;
  endpoint: string;
}

export interface McuMcpTunnelStatus {
  boardId: string;
  channelKind: 'mcu';
  state: string;
  message: string;
  localPort: number;
  remotePort: number;
  endpoint: string;
}

export interface McuMcpInteraction {
  id: number;
  timestampMs: number;
  boardId: string;
  tool: string;
  category: string;
  summary: string;
}

export interface McuCommandMap {
  mcu_ch347_power_action: IpcCommandSpec<
    {
      request: {
        boardId: string;
        deviceIndex: number;
        gpio: number;
        activeHigh: boolean;
        action: 'on' | 'off' | 'cycle';
        offTimeMs?: number;
      };
    },
    'on' | 'off'
  >;
  mcu_stlink_tool_info: IpcCommandSpec<{ request: { executablePath?: string } }, McuToolInfo>;
  mcu_stlink_list_probes: IpcCommandSpec<
    { request: { executablePath?: string } },
    McuStLinkProbe[]
  >;
  mcu_stlink_target_status: IpcCommandSpec<{ request: McuStLinkConnection }, McuTargetStatus>;
  mcu_stlink_target_action: IpcCommandSpec<
    {
      boardId: string;
      request: {
        connection: McuStLinkConnection;
        action: 'resetRun' | 'resetHalt' | 'run' | 'halt';
      };
    },
    McuOperationResult
  >;
  mcu_stlink_flash_ospi: IpcCommandSpec<
    {
      boardId: string;
      request: {
        connection: McuStLinkConnection;
        firmwarePath: string;
        externalLoaderPath: string;
        externalAddressStart: number;
        externalAddressSize: number;
        binStartAddress?: number;
        postAction: 'none' | 'resetRun' | 'resetHalt';
      };
    },
    McuOperationResult
  >;
  mcu_list_boards: IpcCommandSpec<undefined, McuBoardConfig[]>;
  mcu_save_board: IpcCommandSpec<{ board: McuBoardConfig }, McuBoardConfig>;
  mcu_delete_board: IpcCommandSpec<{ id: string }>;
  mcu_mcp_gateway_start: IpcCommandSpec<
    { request: { boardId: string; port?: number } },
    McuMcpGatewayStatus
  >;
  mcu_mcp_gateway_status: IpcCommandSpec<{ port?: number }, McuMcpGatewayStatus>;
  mcu_mcp_gateway_stop: IpcCommandSpec<{ port?: number }, McuMcpGatewayStatus>;
  mcu_mcp_list_interactions: IpcCommandSpec<{ limit?: number }, McuMcpInteraction[]>;
  mcu_mcp_clear_interactions: IpcCommandSpec<{ boardId?: string }>;
  mcu_mcp_tunnel_start: IpcCommandSpec<
    {
      request: {
        boardId: string;
        hostAddress: string;
        hostPort: number;
        hostUsername: string;
        authType: 'password' | 'private_key';
        privateKeyPath: string;
        hostFingerprint: string;
        password?: string;
        keyPassphrase?: string;
        localPort: number;
        remotePort: number;
        mcpServerName: string;
      };
    },
    McuMcpTunnelStatus
  >;
  mcu_mcp_tunnel_status: IpcCommandSpec<{ boardId: string }, McuMcpTunnelStatus | null>;
  mcu_mcp_tunnel_stop: IpcCommandSpec<{ boardId: string }, McuMcpTunnelStatus>;
  mcu_openocd_tool_info: IpcCommandSpec<
    { request: { executablePath?: string } },
    McuOpenOcdToolInfo
  >;
  mcu_openocd_target_status: IpcCommandSpec<{ request: McuOpenOcdConnection }, McuTargetStatus>;
  mcu_openocd_target_action: IpcCommandSpec<
    {
      boardId: string;
      request: {
        connection: McuOpenOcdConnection;
        action: 'resetRun' | 'resetHalt' | 'run' | 'halt';
      };
    },
    McuOperationResult
  >;
  mcu_openocd_flash_ospi: IpcCommandSpec<
    {
      boardId: string;
      request: {
        connection: McuOpenOcdConnection;
        firmwarePath: string;
        flashBank: string;
        externalAddressStart: number;
        externalAddressSize: number;
        binStartAddress?: number;
        postAction: 'none' | 'resetRun' | 'resetHalt';
      };
    },
    McuOperationResult
  >;
  mcu_pyocd_tool_info: IpcCommandSpec<{ request: { executablePath?: string } }, McuToolInfo>;
  mcu_pyocd_target_status: IpcCommandSpec<{ request: McuPyOcdConnection }, McuTargetStatus>;
  mcu_pyocd_target_action: IpcCommandSpec<
    {
      boardId: string;
      request: {
        connection: McuPyOcdConnection;
        action: 'resetRun' | 'resetHalt' | 'run' | 'halt';
      };
    },
    McuOperationResult
  >;
  mcu_pyocd_flash_internal: IpcCommandSpec<
    {
      boardId: string;
      request: {
        connection: McuPyOcdConnection;
        firmwarePath: string;
        binStartAddress?: number;
        postAction: 'none' | 'resetRun' | 'resetHalt';
      };
    },
    McuOperationResult
  >;
  mcu_jlink_tool_info: IpcCommandSpec<{ request: { executablePath?: string } }, McuToolInfo>;
  mcu_jlink_target_status: IpcCommandSpec<
    { boardId: string; request: McuJLinkConnection },
    McuTargetStatus
  >;
  mcu_jlink_target_action: IpcCommandSpec<
    {
      boardId: string;
      request: {
        connection: McuJLinkConnection;
        action: 'resetRun' | 'resetHalt' | 'run' | 'halt';
      };
    },
    McuOperationResult
  >;
  mcu_jlink_flash_internal: IpcCommandSpec<
    {
      boardId: string;
      request: {
        connection: McuJLinkConnection;
        firmwarePath: string;
        binStartAddress?: number;
        postAction: 'none' | 'resetRun' | 'resetHalt';
      };
    },
    McuOperationResult
  >;
}

export interface McuEventMap {
  'mcu-mcp-interaction': IpcEventSpec<McuMcpInteraction>;
}
