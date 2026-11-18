import type {
  LearnV2Home,
  LearnPublishedCourse,
  LearnLearningSummary,
  LearnReviewRequest,
  LearnReviewResult,
  LearnReportRequest,
  LearnReportJob,
  LearnEnvironmentRequest,
  LearnEnvironmentBinding,
  LearnEnvironmentView,
  LearnEnvironmentInspection,
  LearnActionRequest,
  LearnActionRun,
  LearnActionEvidencePreview,
  LearnPracticeSyncSelection,
  LearnPracticeSyncBatch,
  LearnBoardObservation,
  LearnLearningRecord,
  LearnRecordRequest,
  LearnVerifiedPreview,
  LearnPreviewList,
  LearnPreviewReadReceipt,
  LearnCoursePin,
  LearnVerifiedCourse,
  LearnUnitContent,
} from './contracts/LearnV2';
export type * from './contracts/LearnV2';
import type { IpcCommandSpec, IpcEventSpec } from './Types';
import type { McuCommandMap, McuEventMap } from './contracts/Mcu';
import type { RecordCenterCommandMap } from './contracts/RecordCenter';
import type { SupervisorCommandMap, SupervisorEventMap } from './contracts/Supervisor';
import type { WorkbenchCommandMap, WorkbenchEventMap } from './contracts/Workbench';
export type {
  McuBoardConfig,
  McuChipSeries,
  McuChipVendor,
  McuMcpGatewayStatus,
  McuMcpInteraction,
  McuMcpTunnelStatus,
  McuJLinkConnection,
  McuOpenOcdConnection,
  McuOpenOcdToolInfo,
  McuOperationResult,
  McuProbeBackend,
  McuPyOcdConnection,
  McuStLinkConnection,
  McuStLinkProbe,
  McuTargetStatus,
  McuToolInfo,
} from './contracts/Mcu';
export type {
  RecordCenterBackup,
  RecordCenterEvent,
  RecordCenterFeedbackDraft,
  RecordCenterRestorePreview,
  RecordCenterRestoreResult,
  RecordCenterSession,
} from './contracts/RecordCenter';
export type {
  AiProviderConfig,
  SaveAiProviderRequest,
  SupervisorActionResultEvent,
  SupervisorAuditRecord,
  SupervisorAutomaticTask,
  SupervisorHumanRequest,
  SupervisorPolicy,
  SupervisorRound,
  SupervisorRun,
  SupervisorRuntimeEvent,
} from './contracts/Supervisor';
export type {
  CameraDeviceCandidate,
  GatewayInteraction,
  LocalMcpStatus,
  LinuxDeviceOverview,
  WorkbenchCredentials,
  WorkbenchDevice,
  WorkbenchFlasherDevice,
  WorkbenchHost,
  WorkbenchPlatform,
  WorkbenchProject,
  WorkbenchSshTerminalMode,
  WorkbenchSshTerminalSnapshot,
  WorkbenchTaskHistory,
  WorkbenchTunnelStatus,
  WorkbenchWorkflowStep,
} from './contracts/Workbench';

import type { GatewayInteraction } from './contracts/Workbench';

export interface SerialIdentity {
  port: string;
  vid: number;
  pid: number;
  serialNumber: string;
  interfaceNumber?: number | null;
}

export interface SerialAuditContext {
  actorType: string;
  actorId: string;
  scopeType: 'linux_board' | 'mcu_board';
  scopeId: string;
  sessionId: string;
  sessionTitle: string;
}

export interface DeltaUpdateFailure {
  code: 'PRECONDITION' | 'COPY' | 'PATCH' | 'SELF_TEST';
  message: string;
}

export interface AdbTransferSnapshot {
  taskId: string;
  deviceId: number;
  direction: 'upload' | 'download';
  fileName: string;
  transferredBytes: number;
  totalBytes: number;
  bytesPerSecond: number;
  status: 'queued' | 'running' | 'success' | 'failed' | 'cancelled';
  errorCode: string | null;
  startedAtMs: number;
  finishedAtMs: number | null;
}

export interface ResolvedSerialPort {
  name: string;
  vid: number;
  pid: number;
  manufacturer: string;
  description: string;
  serialNumber: string;
  interfaceNumber: number | null;
}

export type HardwareMonitorRunState =
  'stopped' | 'starting' | 'running' | 'paused' | 'waiting' | 'reconnecting' | 'stopping' | 'error';

export interface HardwareMonitorChannel {
  channel: number;
  address: number;
  relayGpio: number;
  status: 'waiting' | 'ok' | 'error';
  busVoltageV: number | null;
  shuntVoltageV: number | null;
  currentA: number | null;
  powerW: number | null;
  errorCode: string | null;
  error: string | null;
}

export interface HardwareMonitorAiChannel {
  status: string;
  providerCount: number;
  enabledProviderCount: number;
  readyProviderCount: number;
  activeRunCount: number;
  waitingHumanCount: number;
  lastTestStatus: string;
}

export interface HardwareMonitorSnapshot {
  state: HardwareMonitorRunState;
  deviceIndex: number | null;
  serialNumber: string;
  refreshMs: number;
  sequence: number;
  updatedAtMs: number;
  reconnectCount: number;
  relays: 'board-controlled';
  channels: HardwareMonitorChannel[];
  ai: HardwareMonitorAiChannel;
  errorCode: string | null;
  error: string | null;
}

/**
 * Type map for all IPC commands.
 *
 * Defines argument and result types for every Tauri command,
 * enabling type-safe invocation through invokeCommand.
 * Command names map to IpcCommandSpec with typed args and result.
 */
export interface IpcCommandMap
  extends RecordCenterCommandMap, SupervisorCommandMap, WorkbenchCommandMap, McuCommandMap {
  hardware_safety_status: IpcCommandSpec<
    undefined,
    {
      busy: boolean;
      activities: string[];
    }
  >;
  hardware_monitor_start: IpcCommandSpec<
    { request: { deviceIndex: number; serialNumber?: string } },
    HardwareMonitorSnapshot
  >;
  hardware_monitor_stop: IpcCommandSpec<undefined, HardwareMonitorSnapshot>;
  hardware_monitor_status: IpcCommandSpec<undefined, HardwareMonitorSnapshot>;
  gateway_start: IpcCommandSpec<
    { port?: number; projectId?: number | null; deviceId?: number | null },
    { running: boolean; port: number; endpoint: string }
  >;
  gateway_stop: IpcCommandSpec<undefined, { running: boolean; port: number; endpoint: string }>;
  gateway_stop_port: IpcCommandSpec<
    { port: number },
    { running: boolean; port: number; endpoint: string }
  >;
  gateway_channel_status: IpcCommandSpec<
    { projectId: number; deviceId: number },
    import('./contracts/Workbench').LocalMcpStatus
  >;
  gateway_status: IpcCommandSpec<undefined, { running: boolean; port: number; endpoint: string }>;
  gateway_network_info: IpcCommandSpec<
    undefined,
    { hostName: string; localIp: string; loopback: string }
  >;
  gateway_list_interactions: IpcCommandSpec<{ limit?: number }, GatewayInteraction[]>;
  gateway_clear_interactions: IpcCommandSpec;

  serial_resolve_port: IpcCommandSpec<{ identity: SerialIdentity }, ResolvedSerialPort>;
  serial_release_matching: IpcCommandSpec<{ identity: SerialIdentity }, string | null>;
  serial_wait_and_open: IpcCommandSpec<
    {
      identity: SerialIdentity;
      baudRate: number;
      dataBits: number;
      stopBits: number;
      parity: string;
      flowControl: string;
      timeoutMs: number;
      auditContext?: SerialAuditContext;
    },
    ResolvedSerialPort
  >;

  /** Open DevTools window for debugging */
  open_devtools: IpcCommandSpec;
  get_supported_archs: IpcCommandSpec<undefined, Array<[string, string]>>;
  /** Get system proxy settings as string */
  get_system_proxy: IpcCommandSpec<undefined, string>;
  consume_update_delta_failure: IpcCommandSpec<undefined, DeltaUpdateFailure | null>;
  /** Get proxy configuration as string */
  get_proxy_config: IpcCommandSpec<undefined, string>;

  /** Check ADB server status and version */
  adb_check_server: IpcCommandSpec<
    undefined,
    {
      /** Whether ADB server is running */
      running: boolean;
      /** ADB version info if running */
      version?: {
        /** Full version string */
        version: string;
        /** Major version number */
        major: number;
        /** Minor version number */
        minor: number;
        /** Patch version number */
        patch: number;
      };
      /** ADB server port */
      port: number;
    }
  >;

  /** List connected ADB devices */
  adb_list_devices: IpcCommandSpec<
    undefined,
    Array<{
      /** Device serial number */
      serial: string;
      /** Device state (device, offline, unauthorized) */
      state: string;
      /** Device model name */
      model?: string;
      /** Product name */
      product?: string;
      /** Device identifier */
      device?: string;
      /** Transport ID for connection */
      transport_id?: number;
    }>
  >;

  /** Select an ADB device for subsequent operations */
  adb_select_device: IpcCommandSpec<{ serial: string }>;

  /** Get the currently selected device serial */
  adb_get_selected_device: IpcCommandSpec<undefined, string | null>;

  /** Clear device selection */
  adb_clear_selected_device: IpcCommandSpec;

  /** Execute shell command on ADB device */
  adb_shell_command: IpcCommandSpec<{ serial: string | null; command: string }, string>;

  adb_transfer_start: IpcCommandSpec<
    {
      request: {
        deviceId: number;
        direction: 'upload' | 'download';
        localPath: string;
        remotePath: string;
        overwrite: boolean;
      };
    },
    AdbTransferSnapshot
  >;
  adb_transfer_cancel: IpcCommandSpec<{ taskId: string }, AdbTransferSnapshot>;
  adb_transfer_list: IpcCommandSpec<{ deviceId: number }, AdbTransferSnapshot[]>;

  /** List directory contents on ADB device */
  adb_list_directory: IpcCommandSpec<
    { serial: string | null; path: string },
    {
      /** Directory path listed */
      path: string;
      /** Items in directory */
      items: Array<{
        /** Item name */
        name: string;
        /** Full item path */
        path: string;
        /** File size in bytes */
        size: number;
        /** Whether item is a directory */
        is_directory: boolean;
        /** Modification timestamp */
        modified_time?: number;
        /** Permission string */
        permissions?: string;
      }>;
    }
  >;

  /** Push local file to ADB device */
  adb_push_file: IpcCommandSpec<{ serial: string | null; localPath: string; remotePath: string }>;

  /** Pull file from ADB device to local path */
  adb_pull_file: IpcCommandSpec<{ serial: string | null; remotePath: string; localPath: string }>;

  /** Pull folder from ADB device to local path */
  adb_pull_folder: IpcCommandSpec<{
    serial: string | null;
    remotePath: string;
    localPath: string;
  }>;

  /** Delete file on ADB device */
  adb_delete_file: IpcCommandSpec<{ serial: string | null; path: string }, string>;

  /** Create directory on ADB device */
  adb_make_directory: IpcCommandSpec<{ serial: string | null; path: string }, string>;

  /** Rename file or directory on ADB device */
  adb_rename: IpcCommandSpec<{ serial: string | null; oldPath: string; newPath: string }, string>;

  /** Get file information on ADB device */
  adb_stat: IpcCommandSpec<
    { serial: string | null; path: string },
    {
      /** File name */
      name: string;
      /** Full file path */
      path: string;
      /** File size in bytes */
      size: number;
      /** Whether item is a directory */
      is_directory: boolean;
      /** Modification timestamp */
      modified_time?: number;
      /** Permission string */
      permissions?: string;
    }
  >;

  /** Reboot ADB device */
  adb_reboot: IpcCommandSpec<{ serial: string | null; rebootType: string }>;

  /** Switch ADB device to root mode */
  adb_root: IpcCommandSpec<{ serial: string | null }, string>;

  /** Set USB backend driver type */
  efex_set_usb_backend: IpcCommandSpec<{ backend: 'libusb' | 'winusb' }>;

  /** Get current USB backend driver type */
  efex_get_usb_backend: IpcCommandSpec<undefined, 'libusb' | 'winusb'>;

  /** Scan for connected EFEX devices */
  efex_scan_devices: IpcCommandSpec<
    undefined,
    Array<{
      /** Device identifier from USB */
      device_id: number;
      /** Chip version number */
      chip_version: number;
      /** Device mode */
      mode: 'null' | 'fel' | 'srv' | 'update_cool' | 'update_hot' | 'unknown';
      /** Human-readable mode string */
      mode_str: string;
      /** USB bus number */
      bus: number;
      /** USB port number */
      port: number;
    }>
  >;

  /** Close an EFEX device context */
  efex_close_device: IpcCommandSpec<{ deviceId: number }>;

  /** Get device mode for EFEX device */
  efex_get_device_mode: IpcCommandSpec<
    { deviceId: number },
    'null' | 'fel' | 'srv' | 'update_cool' | 'update_hot' | 'unknown'
  >;

  /** Get human-readable device mode string */
  efex_get_device_mode_str: IpcCommandSpec<{ deviceId: number }, string>;

  /** Set FEL mode operation timeout */
  efex_set_fel_timeout: IpcCommandSpec<{ timeoutSecs: number }>;

  /** Set FES mode operation timeout */
  efex_set_fes_timeout: IpcCommandSpec<{ timeoutSecs: number }>;

  /** Read memory from FEL mode device */
  efex_fel_read: IpcCommandSpec<{ deviceId: number; addr: number; len: number }, number[]>;

  /** Write memory to FEL mode device */
  efex_fel_write: IpcCommandSpec<{ deviceId: number; addr: number; data: number[] }>;

  /** Execute code at address in FEL mode device */
  efex_fel_exec: IpcCommandSpec<{ deviceId: number; addr: number }>;

  /** Initialize DRAM in FEL mode */
  efex_fel_init_dram: IpcCommandSpec<
    { deviceId: number; fexData: number[] },
    {
      /** Whether initialization succeeded */
      success: boolean;
      /** DRAM init completion flag */
      dram_init_flag: number;
      /** DRAM parameter update flag */
      dram_update_flag: number;
      /** Return address for init code */
      ret_addr: number;
      /** DRAM parameters array */
      dram_para: number[];
    }
  >;

  /** Initialize DRAM with custom parameters */
  efex_fel_init_dram_with_params: IpcCommandSpec<
    {
      deviceId: number;
      fexData: number[];
      dramInfo: { dram_init_flag: number; dram_update_flag: number; dram_para: number[] };
    },
    {
      /** Whether initialization succeeded */
      success: boolean;
      /** DRAM init completion flag */
      dram_init_flag: number;
      /** DRAM parameter update flag */
      dram_update_flag: number;
      /** Return address for init code */
      ret_addr: number;
      /** DRAM parameters array */
      dram_para: number[];
    }
  >;

  /** Query storage type in FES mode */
  efex_fes_query_storage: IpcCommandSpec<{ deviceId: number }, number>;

  /** Query secure boot status in FES mode */
  efex_fes_query_secure: IpcCommandSpec<{ deviceId: number }, number>;

  /** Probe flash storage size in FES mode */
  efex_fes_probe_flash_size: IpcCommandSpec<{ deviceId: number }, number>;

  /** Enable/disable flash access in FES mode */
  efex_fes_flash_set_onoff: IpcCommandSpec<{
    deviceId: number;
    storageType: number;
    onOff: boolean;
  }>;

  /** Get chip ID from FES mode device */
  efex_fes_get_chipid: IpcCommandSpec<{ deviceId: number }, string>;

  /** Download data to flash in FES mode */
  efex_fes_down: IpcCommandSpec<{
    deviceId: number;
    buf: number[];
    addr: number;
    dataType: number;
  }>;

  /** Read data from flash in FES mode */
  efex_fes_up: IpcCommandSpec<
    { deviceId: number; len: number; addr: number; dataType: number },
    number[]
  >;

  /** Verify data value in flash */
  efex_fes_verify_value: IpcCommandSpec<
    { deviceId: number; addr: number; size: number },
    { flag: number; fes_crc: number; media_crc: number }
  >;

  /** Verify flash write status */
  efex_fes_verify_status: IpcCommandSpec<
    { deviceId: number; tag: number },
    { flag: number; fes_crc: number; media_crc: number }
  >;

  /** Verify U-Boot block in flash */
  efex_fes_verify_uboot_blk: IpcCommandSpec<
    { deviceId: number; tag: number },
    { flag: number; fes_crc: number; media_crc: number }
  >;

  /** Set tool mode and next mode for FES */
  efex_fes_tool_mode: IpcCommandSpec<{ deviceId: number; toolMode: number; nextMode: number }>;

  /** Initialize payload helper functions */
  efex_payloads_init: IpcCommandSpec<{ arch: string }>;

  /** Read 32-bit value from device memory */
  efex_payloads_readl: IpcCommandSpec<{ deviceId: number; addr: number }, number>;

  /** Write 32-bit value to device memory */
  efex_payloads_writel: IpcCommandSpec<{ deviceId: number; value: number; addr: number }>;

  /** Disassemble binary code */
  disassemble: IpcCommandSpec<
    { data: number[]; address: number; arch: string },
    {
      /** Disassembled instructions */
      instructions: Array<{
        /** Instruction address */
        address: number;
        /** Instruction size in bytes */
        size: number;
        /** Raw instruction bytes */
        bytes: number[];
        /** Instruction mnemonic */
        mnemonic: string;
        /** Operand string */
        op_str: string;
      }>;
      /** Error message if disassembly failed */
      error: string | null;
    }
  >;

  /** Start USB hot-plug monitoring */
  hotplug_start: IpcCommandSpec;

  /** Extract a chunk from a file */
  extract_file_chunked: IpcCommandSpec<
    { sourcePath: string; destPath: string; offset: number; length: number },
    { success: boolean; message: string; bytes_written: number }
  >;

  /** Extract multiple files in batch */
  extract_files_batch: IpcCommandSpec;

  /** Get file size in bytes */
  get_file_size: IpcCommandSpec<{ path: string }, number>;
  imager_file_identity: IpcCommandSpec<{ path: string }, { sizeBytes: number; sha256: string }>;
  imager_inspect_image: IpcCommandSpec<{ path: string }, import('./contracts/ImagerImage').ImageInspection>;
  imager_read_image_reference: IpcCommandSpec<{ path: string }, import('./contracts/ImagerImage').ImageReference>;
  imager_compare_application: IpcCommandSpec<{ imagePath:string; applicationPath:string; offset:number; expectedBytes:number|null }, import('./contracts/ImagerImage').AppComparison>;

  /** Start flash operation */
  flash_start: IpcCommandSpec<
    {
      deviceId: number;
      bus: number;
      port: number;
      imagePath: string;
      options: {
        /** Flash mode type */
        mode:
          | 'bootloader'
          | 'partition'
          | 'keep_data'
          | 'partition_erase'
          | 'full_erase'
          | 'erase_only';
        /** Partitions to flash (optional) */
        partitions?: string[];
        /** Whether to verify after download */
        verifyDownload: boolean;
        /** Action after flash completes */
        postFlashAction: 'reboot' | 'poweroff' | 'none';
        /** Custom MBR data (optional) */
        mbrData?: number[];
        /** Custom partition config (optional) */
        partitionConfig?: Array<{
          name: string;
          size: number;
          downloadfile: string;
          userType: number;
          keydata: boolean;
          encrypt: boolean;
          verify: boolean;
          ro: boolean;
          customFilePath?: string;
        }>;
      };
    },
    { task_id: number }
  >;

  /** Cancel active flash operation */
  flash_cancel: IpcCommandSpec<{ taskId: number }>;

  /** Confirm a flash request */
  flash_confirm: IpcCommandSpec<{ taskId: number; requestId: number; confirmed: boolean }>;

  /** Start mass production flashing */
  mass_start: IpcCommandSpec<{
    imagePath: string;
    options: {
      /** Flash mode type */
      mode:
        'bootloader' | 'partition' | 'keep_data' | 'partition_erase' | 'full_erase' | 'erase_only';
      /** Partitions to flash (optional) */
      partitions?: string[];
      /** Whether to verify after download */
      verifyDownload: boolean;
      /** Action after flash completes */
      postFlashAction: 'reboot' | 'poweroff' | 'none';
      /** Custom MBR data (optional) */
      mbrData?: number[];
      /** Custom partition config (optional) */
      partitionConfig?: Array<{
        name: string;
        size: number;
        downloadfile: string;
        userType: number;
        keydata: boolean;
        encrypt: boolean;
        verify: boolean;
        ro: boolean;
        customFilePath?: string;
      }>;
    };
    /** Maximum concurrent slots */
    maxSlots: number;
  }>;

  /** Stop mass production flashing */
  mass_stop: IpcCommandSpec;

  /** Get mass production status */
  mass_get_status: IpcCommandSpec<
    undefined,
    {
      /** Current state */
      state: 'stopped' | 'running';
      /** Slot statuses */
      slots: Array<{
        /** Slot identifier */
        id: number;
        /** Slot status */
        status: 'idle' | 'waiting' | 'flashing' | 'success' | 'failed';
        /** USB bus number */
        bus: number | null;
        /** USB port number */
        port: number | null;
        /** Task ID for this slot */
        taskId: number | null;
        /** Progress percentage */
        progress: number;
        /** Current stage name */
        stage: string;
        /** Flash speed string */
        speed: string | null;
        /** Error message */
        error: string | null;
        /** Start timestamp */
        startTime: number | null;
        /** End timestamp */
        endTime: number | null;
        /** Number of flashes completed */
        flashCount: number;
      }>;
      /** Total devices processed */
      total: number;
      /** Successful flashes */
      success: number;
      /** Failed flashes */
      failed: number;
      /** Currently in progress */
      inProgress: number;
    }
  >;

  /** Parse GPT from file */
  parse_gpt_from_file: IpcCommandSpec<
    { filePath: string; sectorSize: number },
    { success: boolean; message: string; gpt_info: unknown | null }
  >;

  /** Parse GPT from binary data */
  parse_gpt_from_data: IpcCommandSpec<
    { data: number[]; sectorSize: number },
    { success: boolean; message: string; gpt_info: unknown | null }
  >;

  /** Parse MBR from file */
  parse_mbr_from_file: IpcCommandSpec<
    { filePath: string; sectorSize: number },
    { success: boolean; message: string; mbr_info: unknown | null; partitions: unknown[] }
  >;

  /** Parse MBR from binary data */
  parse_mbr_from_data: IpcCommandSpec<
    { data: number[]; sectorSize: number },
    { success: boolean; message: string; mbr_info: unknown | null; partitions: unknown[] }
  >;

  /** Parse FDT/DTB from file */
  fdt_parse_from_file: IpcCommandSpec<
    { filePath: string },
    { success: boolean; message: string; fdt_info: unknown | null }
  >;

  /** Parse FDT/DTB from binary data */
  fdt_parse_from_data: IpcCommandSpec<
    { data: number[] },
    { success: boolean; message: string; fdt_info: unknown | null }
  >;

  /** Get FDT node by path */
  fdt_get_node: IpcCommandSpec<
    { data: number[]; nodePath: string },
    { success: boolean; message: string; node: unknown | null }
  >;

  /** Get FDT property by path and name */
  fdt_get_property: IpcCommandSpec<
    { data: number[]; nodePath: string; propertyName: string },
    { success: boolean; message: string; property: unknown | null }
  >;

  /** List FDT node children */
  fdt_list_node_children: IpcCommandSpec<
    { data: number[]; nodePath: string },
    { success: boolean; message: string; children: string[] }
  >;

  /** Find FDT nodes by compatible string */
  fdt_find_compatible: IpcCommandSpec<
    { data: number[]; compatibleString: string },
    { success: boolean; message: string; children: string[] }
  >;

  /** Generate DTS source from FDT binary */
  fdt_generate_dts: IpcCommandSpec<
    { data: number[] },
    { success: boolean; message: string; dts: string | null }
  >;

  /** Parse firmware image file */
  firmware_parse_image: IpcCommandSpec<
    { filePath: string },
    {
      image_info: unknown | null;
      file_headers: unknown[];
      is_encrypted: boolean;
      last_error: string | null;
    }
  >;

  /** Read firmware entry by filename */
  firmware_read_entry_by_filename: IpcCommandSpec<
    { filePath: string; filename: string },
    number[] | null
  >;

  /** Read firmware entry by maintype and subtype */
  firmware_read_entry_by_maintype_subtype: IpcCommandSpec<
    { filePath: string; maintype: string; subtype: string },
    number[] | null
  >;

  /** Read firmware entry range by filename */
  firmware_read_entry_range_by_filename: IpcCommandSpec<
    { filePath: string; filename: string; start: number; length: number },
    number[] | null
  >;

  /** Read firmware entry range by maintype and subtype */
  firmware_read_entry_range_by_maintype_subtype: IpcCommandSpec<
    { filePath: string; maintype: string; subtype: string; start: number; length: number },
    number[] | null
  >;

  /** Parse partition config binary */
  firmware_parse_partition_config: IpcCommandSpec<{ data: number[] }, unknown>;

  /** Serialize partition config to binary */
  firmware_serialize_partition_config: IpcCommandSpec<{ config: unknown }, number[]>;

  /** Parse Boot0 header */
  firmware_parse_boot0: IpcCommandSpec<{ data: number[] }, unknown>;

  /** Serialize Boot0 header to binary */
  firmware_serialize_boot0: IpcCommandSpec<{ header: unknown }, number[]>;

  /** Parse DRAM parameters */
  firmware_parse_dram_params: IpcCommandSpec<{ data: number[] }, unknown>;

  /** Serialize DRAM parameters to binary */
  firmware_serialize_dram_params: IpcCommandSpec<{ info: unknown }, number[]>;

  /** Parse U-Boot binary */
  firmware_parse_uboot: IpcCommandSpec<{ data: number[] }, unknown>;

  /** Get U-Boot work mode */
  firmware_get_uboot_work_mode: IpcCommandSpec<{ data: number[] }, number>;

  /** Get U-Boot storage type */
  firmware_get_uboot_storage_type: IpcCommandSpec<{ data: number[] }, number>;

  /** Set U-Boot work mode */
  firmware_set_uboot_work_mode: IpcCommandSpec<{ data: number[]; mode: number }, number[]>;

  /** Set U-Boot storage type */
  firmware_set_uboot_storage_type: IpcCommandSpec<
    { data: number[]; storageType: number },
    number[]
  >;

  /** Parse sys_config binary */
  firmware_parse_sys_config: IpcCommandSpec<{ data: number[] }, unknown>;

  /** Parse sunxi MBR binary */
  firmware_parse_sunxi_mbr: IpcCommandSpec<{ data: number[] }, unknown>;

  /** Validate sunxi MBR */
  firmware_is_valid_sunxi_mbr: IpcCommandSpec<{ data: number[] }, boolean>;

  /** Convert sunxi MBR to info */
  firmware_sunxi_mbr_to_info: IpcCommandSpec<{ mbr: unknown }, unknown>;

  /** Create empty MBR */
  firmware_mbr_create_empty: IpcCommandSpec<undefined, unknown>;

  /** Add partition to MBR */
  firmware_mbr_add_partition: IpcCommandSpec<
    { mbr: unknown; partition: unknown; beforeIndex?: number },
    unknown
  >;

  /** Add partition to MBR with raw parameters */
  firmware_mbr_add_partition_raw: IpcCommandSpec<
    { mbr: unknown; partition: unknown; beforeIndex?: number },
    unknown
  >;

  /** Update partition in MBR */
  firmware_mbr_update_partition: IpcCommandSpec<
    { mbr: unknown; index: number; partition: unknown },
    unknown
  >;

  /** Remove partition from MBR */
  firmware_mbr_remove_partition: IpcCommandSpec<{ mbr: unknown; index: number }, unknown>;

  /** Move partition in MBR */
  firmware_mbr_move_partition: IpcCommandSpec<
    { mbr: unknown; fromIndex: number; toIndex: number },
    unknown
  >;

  /** Clear all partitions from MBR */
  firmware_mbr_clear_partitions: IpcCommandSpec<{ mbr: unknown }, unknown>;

  /** Set MBR copy count */
  firmware_mbr_set_copy: IpcCommandSpec<{ mbr: unknown; copy: number }, unknown>;

  /** Set MBR version */
  firmware_mbr_set_version: IpcCommandSpec<{ mbr: unknown; version: number }, unknown>;

  /** Set MBR index */
  firmware_mbr_set_index: IpcCommandSpec<{ mbr: unknown; index: number }, unknown>;

  /** Update MBR timestamp */
  firmware_mbr_update_stamp: IpcCommandSpec<{ mbr: unknown }, unknown>;

  /** Serialize MBR to binary */
  firmware_mbr_serialize: IpcCommandSpec<{ mbr: unknown }, number[]>;

  /** Serialize MBR with copies to binary */
  firmware_mbr_serialize_with_copies: IpcCommandSpec<
    { mbr: unknown; copyCount?: number },
    number[]
  >;

  /** Parse boot package (TOC1) binary */
  firmware_parse_boot_package: IpcCommandSpec<
    { data: number[] },
    {
      /** Package name */
      name: string;
      /** Magic number */
      magic: number;
      /** Checksum */
      add_sum: number;
      /** Serial number */
      serial_num: number;
      /** Status field */
      status: number;
      /** Number of items */
      items_nr: number;
      /** Valid length */
      valid_len: number;
      /** Main version */
      version_main: number;
      /** Sub version */
      version_sub: number;
      /** Package items */
      items: Array<{
        /** Item name */
        name: string;
        /** Data offset in package */
        data_offset: number;
        /** Data length */
        data_len: number;
        /** Encryption flag */
        encrypt: number;
        /** Item type code */
        item_type: number;
        /** Run address */
        run_addr: number;
        /** Item index */
        index: number;
      }>;
    }
  >;

  /** Validate boot package */
  firmware_is_valid_boot_package: IpcCommandSpec<{ data: number[] }, boolean>;

  /** Get boot package item data by name */
  firmware_get_boot_package_item_data: IpcCommandSpec<
    { data: number[]; itemName: string },
    number[] | null
  >;

  /** Get boot package item data by index */
  firmware_get_boot_package_item_data_by_index: IpcCommandSpec<
    { data: number[]; index: number },
    number[] | null
  >;

  /** Get item type name from code */
  firmware_get_item_type_name: IpcCommandSpec<{ itemType: number }, string>;

  /** Merge firmware for SPI NOR flash */
  spinor_merge_firmware: IpcCommandSpec<
    { config: unknown },
    { success: boolean; message: string; output_size: number }
  >;

  /** Merge firmware for eMMC/UFS */
  emmc_ufs_merge_firmware: IpcCommandSpec<
    { config: unknown },
    { success: boolean; message: string; output_size: number }
  >;

  // ─── Serial port commands ──────────────────────────────

  /** List available serial ports */
  serial_list_ports: IpcCommandSpec<
    undefined,
    Array<{
      name: string;
      vid: number;
      pid: number;
      manufacturer: string;
      description: string;
      serialNumber: string;
      interfaceNumber: number | null;
    }>
  >;

  /** Open a serial port */
  serial_open: IpcCommandSpec<{
    port: string;
    baudRate: number;
    dataBits: number;
    stopBits: number;
    parity: string;
    flowControl: string;
    auditContext?: SerialAuditContext;
  }>;

  /** Close a serial port */
  serial_close: IpcCommandSpec<{ port: string }>;

  /** Write data to an open serial port */
  serial_write: IpcCommandSpec<{ port: string; data: number[] }>;

  /** Check if a port is currently open */
  serial_is_open: IpcCommandSpec<{ port: string }, boolean>;

  /** Inspect the non-consuming authoritative capture for an open serial session. */
  serial_capture_status: IpcCommandSpec<
    { port: string },
    {
      port: string;
      sessionId: string;
      nextOffset: number;
      captureError: string | null;
      emitErrors: number;
      connected: boolean;
    }
  >;

  /** Replay a bounded byte range without draining the MCP/REST compatibility buffer. */
  serial_capture_read: IpcCommandSpec<
    { port: string; sessionId: string; offset: number; maxBytes: number },
    {
      sessionId: string;
      startOffset: number;
      nextOffset: number;
      captureEndOffset: number;
      data: number[];
    }
  >;

  /** Export the complete raw capture for the current open session. */
  serial_capture_export: IpcCommandSpec<
    { port: string; sessionId: string; target: string },
    number
  >;

  // ─── TCP commands ──────────────────────────────────

  /** Connect to a TCP server */
  tcp_connect: IpcCommandSpec<{ id: string; host: string; port: number }>;
  /** Send data over an open TCP connection */
  tcp_send: IpcCommandSpec<{ id: string; data: number[] }>;
  /** Start reading from a TCP connection (emits tcp-data-received events) */
  tcp_start_read: IpcCommandSpec<{ id: string }>;
  /** Close a TCP connection */
  tcp_close: IpcCommandSpec<{ id: string }>;

  // ─── CH347 protocol commands ──────────────────────────

  ch347_list_devices: IpcCommandSpec<
    undefined,
    Array<{
      index: number;
      name: string;
      vid: number;
      pid: number;
      serialNumber: string;
      interfaceNumber: number;
    }>
  >;
  ch347_open: IpcCommandSpec<{ index: number }>;
  ch347_close: IpcCommandSpec<{ index: number }>;
  ch347_i2c_transfer: IpcCommandSpec<
    {
      index: number;
      writeData: number[];
      readLen: number;
      speedKhz?: number;
      sclStretch?: boolean;
      delayMs?: number;
    },
    number[]
  >;
  ch347_i2c_scan: IpcCommandSpec<
    { index: number; speedKhz?: number; sclStretch?: boolean; delayMs?: number },
    number[]
  >;
  ch347_spi_init: IpcCommandSpec<{
    index: number;
    mode?: number;
    speedMhz?: number;
    cs?: number;
    dataBits?: number;
    byteOrder?: number;
    writeReadInterval?: number;
    outDefaultData?: number;
    cs1Polarity?: number;
    cs2Polarity?: number;
    isAutoDeactiveCs?: number;
    activeDelay?: number;
    delayDeactive?: number;
  }>;
  ch347_spi_get_config: IpcCommandSpec<
    { index: number },
    {
      mode: number;
      clock: number;
      byteOrder: number;
      writeReadInterval: number;
      outDefaultData: number;
      chipSelect: number;
      cs1Polarity: number;
      cs2Polarity: number;
      isAutoDeactiveCS: number;
      activeDelay: number;
      delayDeactive: number;
    }
  >;
  ch347_spi_set_frequency: IpcCommandSpec<{ index: number; frequencyHz: number }>;
  ch347_spi_set_data_bits: IpcCommandSpec<{ index: number; dataBits: number }>;
  ch347_spi_change_cs: IpcCommandSpec<{ index: number; status: number }>;
  ch347_gpio_set: IpcCommandSpec<{
    index: number;
    enable: number;
    dirOut: number;
    dataOut: number;
  }>;
  ch347_spi_set_chip_select: IpcCommandSpec<{
    index: number;
    enableSelect: number;
    chipSelect: number;
    isAutoDeactiveCs: number;
    activeDelay: number;
    delayDeactive: number;
  }>;
  ch347_spi_write: IpcCommandSpec<{
    index: number;
    txData: number[];
    cs?: number;
    mode?: number;
    speedMhz?: number;
    dataBits?: number;
  }>;
  ch347_spi_fill: IpcCommandSpec<{
    index: number;
    color: number[];
    pixelCount: number;
    cs?: number;
  }>;
  ch347_spi_write_buffer: IpcCommandSpec<{
    index: number;
    data: number[];
    cs?: number;
  }>;
  ch347_spi_read: IpcCommandSpec<
    {
      index: number;
      readLen: number;
      cs?: number;
      mode?: number;
      speedMhz?: number;
      dataBits?: number;
    },
    number[]
  >;
  ch347_spi_transfer: IpcCommandSpec<
    {
      index: number;
      txData: number[];
      cs?: number;
      mode?: number;
      speedMhz?: number;
      dataBits?: number;
    },
    number[]
  >;
  ch347_spi_stream4: IpcCommandSpec<
    {
      index: number;
      txData: number[];
      cs?: number;
      mode?: number;
      speedMhz?: number;
      dataBits?: number;
    },
    number[]
  >;

  // ─── 100ask.net OAuth2 登录 ───
  /** 启动登录：起本机回调服务 + 打开浏览器授权；凭据内置无需参数。登录结果经 auth-login-result 事件回传 */
  auth_login_start: IpcCommandSpec<
    undefined,
    {
      /** 回调服务监听端口 */
      port: number;
      /** 回调地址 */
      redirectUri: string;
      /** 授权地址（打开浏览器的 URL） */
      authorizeUrl: string;
    }
  >;
  /** 查询当前登录用户（未登录返回 null） */
  auth_get_user: IpcCommandSpec<undefined, AuthUserInfo | null>;
  /** 读取当前账号有权查看的官网课程；Access Token 只在 Rust 层使用 */
  learning_list_courses: IpcCommandSpec<undefined, CloudLearningCourse[]>;
  /** 读取当前账号的学习进度；Access Token 只在 Rust 层使用 */
  learning_list_progress: IpcCommandSpec<undefined, CloudLearningProgress[]>;
  /** 读取当前账号已经绑定的硬件和经过后端校验的 HTTPS 资料入口。 */
  learning_list_hardware: IpcCommandSpec<undefined, CloudHardwareOverview>;
  /** 把订单号或兑换码绑定到当前 Rust OAuth 会话对应的账号。 */
  learning_redeem_hardware_order: IpcCommandSpec<{ orderCode: string }, CloudHardwareRedeemResult>;
  learning_get_course: IpcCommandSpec<{ identifier: string }, CloudLearningCourse>;
  learning_get_lab: IpcCommandSpec<{ identifier: string }, CloudLearningLab>;
  /** 读取订单兑换后固定到具体发布版本的组合学习路径。 */
  learning_get_home: IpcCommandSpec<undefined, CloudLearningHome>;
  /** Capability negotiation; execution still requires a signed activity and explicit confirmation. */
  learn_get_capabilities: IpcCommandSpec<undefined, LearnCapabilitiesResult>;
  learn_get_home: IpcCommandSpec<undefined, LearnV2Home>;
  learn_list_courses: IpcCommandSpec<undefined, LearnPublishedCourse[]>;
  learn_get_preview: IpcCommandSpec<{ previewId: string }, LearnVerifiedPreview>;
  learn_list_previews: IpcCommandSpec<void, LearnPreviewList>;
  learn_get_preview_unit: IpcCommandSpec<
    { previewId: string; contentDigest: string; unitId: string },
    LearnUnitContent
  >;
  learn_check_preview_read: IpcCommandSpec<
    { previewId: string; contentDigest: string; requestId: string },
    LearnPreviewReadReceipt
  >;
  learn_get_manifest: IpcCommandSpec<LearnCoursePin, LearnVerifiedCourse>;
  learn_get_learning_summary: IpcCommandSpec<{ request: LearnCoursePin }, LearnLearningSummary>;
  learn_get_practice_review_gate: IpcCommandSpec<
    { request: LearnCoursePin & { unitId: string } },
    { pending: boolean; attemptId: string | null }
  >;
  learn_inspect_practice_freshness: IpcCommandSpec<
    { batchId: string },
    { batchId: string; status: 'current' | 'stale'; checkedAt: string; reason: string | null }
  >;
  learn_submit_review: IpcCommandSpec<{ request: LearnReviewRequest }, LearnReviewResult>;
  learn_request_report: IpcCommandSpec<{ request: LearnReportRequest }, { jobId: string }>;
  learn_get_report_job: IpcCommandSpec<{ request: LearnCoursePin; jobId: string }, LearnReportJob>;
  learn_get_environment: IpcCommandSpec<{ request: LearnEnvironmentRequest }, LearnEnvironmentView>;
  learn_bind_environment: IpcCommandSpec<
    { request: LearnEnvironmentRequest & { projectId: number; sshAlias: string } },
    LearnEnvironmentBinding
  >;
  learn_inspect_environment: IpcCommandSpec<
    { request: LearnEnvironmentRequest },
    LearnEnvironmentInspection
  >;
  learn_open_ide: IpcCommandSpec<{ request: LearnEnvironmentRequest; relativeFile: string }, void>;
  learn_prepare_action: IpcCommandSpec<{ request: LearnActionRequest }, LearnActionRun>;
  learn_start_action: IpcCommandSpec<{ runId: string; confirmation: string }, LearnActionRun>;
  learn_query_action: IpcCommandSpec<{ runId: string }, LearnActionRun>;
  learn_cancel_action: IpcCommandSpec<{ runId: string }, LearnActionRun>;
  learn_list_actions: IpcCommandSpec<{ request: LearnActionRequest }, LearnActionRun[]>;
  learn_collect_action_evidence: IpcCommandSpec<{ runId: string }, LearnActionEvidencePreview>;
  learn_prepare_practice_sync: IpcCommandSpec<
    { request: LearnPracticeSyncSelection },
    LearnPracticeSyncBatch
  >;
  learn_list_practice_sync: IpcCommandSpec<
    { request: LearnEnvironmentRequest & { unitId: string } },
    LearnPracticeSyncBatch[]
  >;
  learn_advance_practice_sync: IpcCommandSpec<{ batchId: string }, LearnPracticeSyncBatch>;
  learn_observe_board: IpcCommandSpec<{ request: LearnActionRequest }, LearnBoardObservation>;
  learn_list_board_observations: IpcCommandSpec<
    { request: LearnActionRequest },
    LearnBoardObservation[]
  >;
  learn_get_learning_record: IpcCommandSpec<LearnCoursePin, LearnLearningRecord>;
  learn_save_learning_record: IpcCommandSpec<{ request: LearnRecordRequest }, LearnLearningRecord>;
  learn_get_unit: IpcCommandSpec<LearnCoursePin & { unitId: string }, LearnUnitContent>;
  learn_ask_tutor: IpcCommandSpec<
    {
      request: LearnCoursePin & {
        unitId: string;
        question: string;
        hintLevel: string;
        engineering?: { activityId: string; sourcePaths: string[] };
      };
    },
    { answer: string; contextScope: string[]; executedAction: false }
  >;
  /** 读取单元文档快照；下载凭据不进入 WebView。 */
  learning_get_unit: IpcCommandSpec<
    { enrollmentId: string; unitKey: string },
    CloudLearningUnitDetail
  >;
  /** 服务端登记 provider_opened 后返回经 Rust 白名单校验的小鹅通地址。 */
  learning_open_video: IpcCommandSpec<
    { enrollmentId: string; unitKey: string },
    CloudLearningVideoLaunch
  >;
  /** 只向已配置 Provider 发送本节签名文档、锁定 commit 与脱敏问题；无执行能力。 */
  learning_ask_tutor: IpcCommandSpec<
    {
      request: { enrollmentId: string; unitKey: string; question: string };
    },
    { answer: string; contextScope: string[]; executedAction: false }
  >;
  learning_update_unit_progress: IpcCommandSpec<
    {
      enrollmentId: string;
      unitKey: string;
      progress: UpdateLearningUnitProgress;
    },
    CloudLearningUnitProgress
  >;
  /** Rust 内下载、验签并安装版本化代码快照，不覆盖已有目录。 */
  learning_install_unit_workspace: IpcCommandSpec<
    { enrollmentId: string; unitKey: string },
    InstalledLearningWorkspace
  >;
  /** 检查实验包是否可下载；短期 URL、签名和 Manifest 只保留在 Rust。 */
  learning_check_lab_package: IpcCommandSpec<
    { identifier: string },
    { version: string; sha256: string; expiresAt: number }
  >;
  /** 下载、校验签名并原子安装实验包；下载凭据始终留在 Rust。 */
  learning_install_lab_package: IpcCommandSpec<
    { identifier: string },
    { labKey: string; version: string; sha256: string; installedAt: number }
  >;
  /** 导入本地 Ed25519 签名课程包；Rust 验签、校验 provenance 并原子安装。 */
  learning_import_local_course: IpcCommandSpec<{ path: string }, InstalledLocalCourse>;
  learning_list_local_courses: IpcCommandSpec<undefined, InstalledLocalCourse[]>;
  learning_list_local_sessions: IpcCommandSpec<undefined, LocalLearningSession[]>;
  learning_start_session: IpcCommandSpec<
    { courseId: string; version: string; projectId: number },
    LocalLearningSession
  >;
  learning_bind_local_project: IpcCommandSpec<
    { sessionId: string; projectId: number },
    LocalLearningSession
  >;
  learning_attach_photo: IpcCommandSpec<
    { sessionId: string; stepId: string; attempt: number; pngBase64: string },
    string
  >;
  learning_read_document: IpcCommandSpec<
    { sessionId: string; stepId: string },
    { markdown: string; sourceUrl: string; sha256: string } | null
  >;
  learning_get_agent_plan: IpcCommandSpec<{ sessionId: string }, LearnAgentPlan>;
  learning_get_journey: IpcCommandSpec<{ sessionId: string }, LearnJourney>;
  learning_get_local_command_diagnostic: IpcCommandSpec<
    { request: LocalCommandDiagnosticScope },
    LocalCommandDiagnostic
  >;
  learning_list_local_artifacts: IpcCommandSpec<
    { request: LocalCommandDiagnosticScope },
    LocalArtifactList
  >;
  learning_read_local_artifact_chunk: IpcCommandSpec<
    {
      request: {
        scope: LocalCommandDiagnosticScope;
        artifactId: string;
        sha256: string;
        offset: number;
      };
    },
    LocalArtifactChunk
  >;
  learning_save_draft: IpcCommandSpec<
    { sessionId: string; stepId: string; attempt: number; answer: string; reflection: string },
    void
  >;
  learning_open_ide: IpcCommandSpec<
    { sessionId: string; sshAlias?: string; relativeFile?: string },
    void
  >;
  learning_inspect_project: IpcCommandSpec<{ sessionId: string }, LocalEnvironmentSnapshot>;
  learning_get_local_step: IpcCommandSpec<{ sessionId: string }, LocalLearningStepView>;
  learning_transition_step: IpcCommandSpec<
    { sessionId: string; nextState: LocalLearningState; answer?: string; reflection?: string },
    LocalLearningSession
  >;
  learning_run_step_action: IpcCommandSpec<
    { sessionId: string; credentials: { password?: string; keyPassphrase?: string } },
    LocalStepActionResult
  >;
  learning_prepare_hil_action: IpcCommandSpec<{ sessionId: string }, LocalHilPreparation>;
  learning_confirm_hil_action: IpcCommandSpec<
    { request: LocalHilConfirmationRequest },
    LocalHilApprovalResult
  >;
  learning_attach_hil_evidence: IpcCommandSpec<
    { request: LocalHilEvidenceRequest },
    LocalLearningEvidence
  >;
  learning_finalize_hil_step: IpcCommandSpec<{ sessionId: string }, LocalStepActionResult>;
  learning_ask_local_tutor: IpcCommandSpec<
    {
      request: {
        sessionId: string;
        question: string;
        hintLevel: 'L1' | 'L2' | 'L3' | 'L4';
        expectedStepId?: string;
        expectedAttempt?: number;
        diagnosticDigest?: string;
      };
    },
    LocalTutorAnswer
  >;
  learning_skip_optional_activity: IpcCommandSpec<
    {
      request: {
        sessionId: string;
        expectedStepId: string;
        expectedAttempt: number;
        expectedState: LocalLearningState;
      };
    },
    LocalLearningSession
  >;
  learning_revisit_activity: IpcCommandSpec<
    {
      request: {
        sessionId: string;
        expectedStepId: string;
        expectedAttempt: number;
        expectedState: LocalLearningState;
        targetStepId: string;
        expectedTargetAttempt: number;
      };
    },
    LocalLearningSession
  >;
  learning_generate_local_report: IpcCommandSpec<{ sessionId: string }, LocalLearningReport>;
  /** 读取并清除启动期间收到的、已经 Rust 白名单解析的学习 Deep Link。 */
  learning_take_pending_deep_link: IpcCommandSpec<undefined, LearningDeepLinkTarget | null>;
  /** 只解析官方课程链接，不打开外部应用、不赋予权益。 */
  learn_parse_course_link: IpcCommandSpec<{ value: string }, LearningDeepLinkTarget>;
  /** 登出，清空会话 */
  auth_logout: IpcCommandSpec;
  /** 取消进行中的登录 */
  auth_cancel_login: IpcCommandSpec;

  // ─── 100ask.net 可选状态同步（Bearer 只留在 Rust 后端）───
  cloud_sync_get_status: IpcCommandSpec<undefined, CloudSyncStatus>;
  cloud_sync_set_enabled: IpcCommandSpec<{ enabled: boolean }, CloudSyncStatus>;
  cloud_sync_sync_now: IpcCommandSpec<undefined, CloudSyncStatus>;

  // ─── 独立的登录态错误诊断上报（默认关闭，固定字段白名单）───
  diagnostics_get_status: IpcCommandSpec<undefined, DiagnosticReportingStatus>;
  diagnostics_set_enabled: IpcCommandSpec<{ enabled: boolean }, DiagnosticReportingStatus>;
  diagnostics_capture: IpcCommandSpec<
    { request: DiagnosticCaptureRequest },
    DiagnosticCaptureResult
  >;
  diagnostics_list_pending: IpcCommandSpec<undefined, DiagnosticReportPreview[]>;
  diagnostics_clear_pending: IpcCommandSpec<undefined, DiagnosticReportingStatus>;
  diagnostics_list_incidents: IpcCommandSpec<{ includeDismissed?: boolean }, DiagnosticIncident[]>;
  diagnostics_dismiss_incident: IpcCommandSpec<{ fingerprint: string; dismissed: boolean }, void>;
  diagnostics_build_ai_handoff: IpcCommandSpec<{ fingerprint: string }, string>;
  diagnostics_refresh_resolutions: IpcCommandSpec<undefined, DiagnosticIncident[]>;
  diagnostics_sync_now: IpcCommandSpec<undefined, DiagnosticReportingStatus>;
}

export interface CloudSyncStatus {
  enabled: boolean;
  instanceId: string;
  state: 'disabled' | 'ready' | 'pending' | 'synced' | 'auth_required';
  pending: boolean;
  lastAttemptAt: number | null;
  lastSuccessAt: number | null;
  lastErrorCode: string | null;
}

export interface DiagnosticReportingStatus {
  enabled: boolean;
  state:
    | 'disabled'
    | 'ready'
    | 'auth_required'
    | 'pending'
    | 'uploading'
    | 'synced'
    | 'retry_wait'
    | 'rejected_schema';
  pendingCount: number;
  lastAttemptAt: number | null;
  lastSuccessAt: number | null;
  lastErrorCode: string | null;
}

export interface DiagnosticCaptureRequest {
  source: 'frontend' | 'backend' | 'rust_panic' | 'lifecycle';
  component: string;
  severity: 'warning' | 'error' | 'fatal';
  errorCode: string;
  machineCode?: string | null;
  messageTemplate?: string | null;
  stack?: string | null;
  operationType?: string | null;
  taskPhase?: string | null;
  correlationId?: string | null;
  failureClass?: DiagnosticFailureClass;
  resolutionKey?: DiagnosticResolutionKey;
  retryable?: boolean;
  recommendedSkill?: string | null;
}

export interface DiagnosticCaptureResult {
  reportId: string;
  fingerprint: string;
  queued: boolean;
}

export interface DiagnosticReportPreview {
  reportId: string;
  schemaVersion: number;
  occurredAt: number;
  lastOccurredAt: number;
  occurrenceCount: number;
  appVersion: string;
  buildCommit: string;
  platform: string;
  architecture: string;
  source: string;
  component: string;
  severity: string;
  errorCode: string;
  machineCode: string | null;
  messageTemplate: string | null;
  stackFingerprint: string | null;
  fingerprint: string;
  operationType: string | null;
  taskPhase: string | null;
  correlationId: string | null;
  failureClass: DiagnosticFailureClass;
  resolutionKey: DiagnosticResolutionKey;
  retryable: boolean;
  recommendedSkill: string | null;
}

export type DiagnosticFailureClass =
  | 'software'
  | 'hardware'
  | 'driver'
  | 'permission'
  | 'configuration'
  | 'connectivity'
  | 'resource'
  | 'data'
  | 'unknown';

export type DiagnosticResolutionKey =
  | 'retry'
  | 'reconnect_device'
  | 'close_conflicting_tool'
  | 'install_compatible_driver'
  | 'reauthenticate'
  | 'check_configuration'
  | 'check_network'
  | 'repair_local_data'
  | 'update_lynx'
  | 'contact_support';

export interface DiagnosticIncident {
  fingerprint: string;
  firstOccurredAt: number;
  lastOccurredAt: number;
  occurrenceCount: number;
  component: string;
  severity: string;
  errorCode: string;
  machineCode: string | null;
  operationType: string | null;
  taskPhase: string | null;
  failureClass: DiagnosticFailureClass;
  resolutionKey: DiagnosticResolutionKey;
  retryable: boolean;
  recommendedSkill: string | null;
  uploadState: 'pending' | 'uploaded';
  dismissed: boolean;
  resolutionStatus: string | null;
  publicWorkaround: string | null;
  targetVersion: string | null;
  releasedVersion: string | null;
  resolutionUpdatedAt: number | null;
}

/**
 * 100ask.net 登录用户信息。
 *
 * 由 Rust 后端从 OAuth2 userinfo 接口获取，camelCase 序列化后回传前端。
 */
export interface AuthUserInfo {
  /** 用户 ID */
  id: number;
  /** 用户名 */
  username: string;
  /** 显示名 */
  name: string;
  /** 邮箱 */
  email: string;
  /** 邮箱是否已验证 */
  emailVerified: boolean;
  /** 头像地址 */
  avatarUrl: string;
}

export type LocalLearningState =
  | 'reading'
  | 'checking_understanding'
  | 'practicing'
  | 'collecting_evidence'
  | 'verifying'
  | 'reflecting'
  | 'completed';

export interface LearnAgentPlan {
  sessionId: string;
  stepId: string;
  attempt: number;
  state: LocalLearningState;
  title: string;
  guidance: string;
  destination: 'text' | 'engineering' | 'evidence' | 'assessment' | 'records';
  requiresUserAction: boolean;
  executedAction: boolean;
}

export interface InstalledLocalCourse {
  entryRequiresProject?: boolean;
  courseId: string;
  version: string;
  title: string;
  boardProfile: string;
  signerFingerprint: string;
  packageSha256: string;
  labCount: number;
  chapters: { stepId: string; title: string }[];
  installedAt: number;
}

export interface LocalLearningSession {
  recordedClassification?: string;
  sessionId: string;
  courseId: string;
  courseVersion: string;
  projectId: number;
  deviceId: number | null;
  boardUid: string;
  coursePackageSha256: string;
  currentStep: string;
  state: LocalLearningState;
  classification: string;
  createdAt: number;
  updatedAt: number;
}

export interface LocalVerificationRule {
  id: string;
  kind: string;
  command: string;
  expectedExitCode: number;
}

export interface LocalLearningLab {
  prerequisites?: string[];
  environmentChecks?: LocalEnvironmentCheck[];
  required?: boolean;
  artifactOutputs?: LocalArtifactMeta['spec'][];
  activityKind?: 'practice' | 'reflection';
  stepId: string;
  number: number;
  path: string;
  title: string;
  learningObjective: string;
  comprehensionCheck: string;
  studentAction: string;
  riskLevel: string;
  expectedObservation: string;
  verificationRules: LocalVerificationRule[];
  requiredEvidence: Array<{ kind: string; source: string }>;
  reflectionPrompt: string;
  hilVerification: LocalHilVerification | null;
}

export interface LocalEnvironmentCheck {
  id: string;
  title: string;
  kind: 'tool' | 'file' | 'os' | 'architecture' | 'git';
  selector: string;
  operator: 'present' | 'eq' | 'sha256';
  expected: string;
  remediation: string;
}

export interface LocalEnvironmentSnapshot {
  output: string;
  checkedAt: number;
  projectId: number;
  stepId?: string;
  attempt?: number;
  fingerprint?: string;
  source?: {
    kind: 'git' | 'unborn' | 'directory' | 'unavailable';
    commit: string;
    branch: string;
    dirty: boolean | null;
    untracked: boolean | null;
    status: string;
    message: string;
  };
  host?: { os: string; architecture: string };
  tools?: Array<{
    name: string;
    status: 'OBSERVED' | 'MISSING' | 'UNVERIFIED';
    path: string;
    version: string;
    sha256: string;
    message: string;
  }>;
  checks?: Array<{
    spec: LocalEnvironmentCheck;
    observed: string;
    status: 'MATCH' | 'MISMATCH' | 'MISSING' | 'UNVERIFIED';
    message: string;
  }>;
}

export interface LocalHilImage {
  path: string;
  bytes: number;
  md5: string;
  sha256: string;
}

export interface LocalHilVerification {
  actionId: string;
  target: string;
  instructions: string;
  requiredEvidenceKinds: string[];
  expectedObservations: string[];
  minimumDurationSeconds: number | null;
  image: LocalHilImage | null;
}

export interface LocalLearningEvidence {
  recorded?: { source: string; status: string } | null;
  kind: string;
  source: string;
  status: string;
  summary: string;
  digest: string;
  createdAt: number;
}

export interface LocalLearningStepView {
  session: LocalLearningSession;
  lab: LocalLearningLab;
  stepIndex: number;
  totalSteps: number;
  evidenceCount: number;
  evidence: LocalLearningEvidence[];
}

export interface LocalStepActionResult {
  status: 'PASS' | 'FAIL' | 'NOT_VERIFIED';
  source: string;
  exitCode: number | null;
  outputDigest: string;
  summary: string;
}

export interface LocalCommandDiagnosticScope {
  sessionId: string;
  stepId: string;
  attempt: number;
  digest: string;
}

export interface LocalCommandDiagnostic extends LocalCommandDiagnosticScope {
  status: 'PASS' | 'FAIL' | 'ERROR';
  availability: 'available' | 'legacy_unavailable';
  text: string | null;
  truncated: boolean;
  exitCode: number | null;
  expectedExitCode: number | null;
  outputSha256: string | null;
  createdAt: number;
}

export interface LocalArtifactMeta {
  spec: { artifactId: string; title: string; path: string; mimeType: string; maxBytes: number };
  bytes: number;
  sha256: string;
  chunkRoot: string;
  chunkCount: number;
}
export interface LocalArtifactList {
  scope: LocalCommandDiagnosticScope;
  createdAt: number;
  items: LocalArtifactMeta[];
}
export interface LocalArtifactChunk {
  scope: LocalCommandDiagnosticScope;
  artifactId: string;
  sha256: string;
  offset: number;
  nextOffset: number;
  bytes: number;
  dataBase64: string;
}

export interface LocalTutorAnswer {
  answer: string;
  contextScope: string[];
  executedAction: false;
  fixture: boolean;
}

export interface LocalHilPreparation extends LocalHilVerification {
  deviceId: number;
  boardUid: string;
  confirmationPhrase: string;
  approvalToken: string;
  expiresAt: number;
}

export interface LocalHilConfirmationRequest {
  sessionId: string;
  approvalToken: string;
  confirmation: string;
}

export interface LocalHilApprovalResult {
  status: 'APPROVED';
  actionId: string;
  boardUid: string;
  evidenceDigest: string;
  instructions: string;
}

export interface LocalHilEvidenceRequest {
  expectedStepId: string;
  expectedAttempt: number;
  sessionId: string;
  evidenceKind: string;
  status: 'PASS' | 'FAIL';
  summary: string;
  observation: string;
  artifactSha256?: string;
  durationSeconds?: number;
}

export interface LocalLearningReport {
  prerequisiteChecks?: LocalActivityAccess[];
  recordedHilResult?: string;
  requiredSteps?: number;
  requiredCompletedSteps?: number;
  optionalSteps?: number;
  optionalCompletedSteps?: number;
  optionalSkippedSteps?: number;
  session: LocalLearningSession;
  completedSteps: number;
  totalSteps: number;
  evidenceCount: number;
  softwareResult: string;
  hilResult: string;
  hilPassedSteps: number;
  hilRequiredSteps: number;
  provenanceResult: string;
  recordedSoftwareResult?: string;
  currentWorkspace?: {
    environmentChecks?: Array<{
      stepId: string;
      attempt: number;
      digest: string;
      status: 'MATCH' | 'CHANGED' | 'UNAVAILABLE' | 'UNRECORDED';
    }>;
    sourceChecks?: Array<{
      stepId: string;
      attempt: number;
      digest: string;
      status: 'MATCH' | 'CHANGED' | 'UNAVAILABLE' | 'UNRECORDED';
    }>;
    status: 'NOT_REQUIRED' | 'NOT_CHECKED' | 'MATCH' | 'CHANGED' | 'UNAVAILABLE' | 'UNRECORDED';
    checkedAt: number | null;
    reference: { stepId: string; attempt: number; digest: string } | null;
  };
}

/** 官网课程在 LYNX 学习中心使用的最小只读投影。 */
export interface CloudLearningCourse {
  id: number;
  slug: string;
  title: string;
  version: string;
  summary: string;
  cover: string;
  track: string;
  level: string;
  entitlementRequired: boolean;
  hasEntitlement: boolean;
  lynxReady: boolean;
  chapters: CloudLearningChapter[];
  labs: CloudLearningLab[];
}

export interface CloudLearningChapter {
  id: number;
  chapterKey: string;
  title: string;
  contentType: string;
  sourceUrl: string;
  sortNo: number;
}

export interface CloudLearningBoardProfile {
  profileId: string;
  displayName: string;
  vendor: string;
  soc: string;
}

export interface CloudLearningLab {
  id: number;
  labKey: string;
  title: string;
  summary: string;
  riskLevel: string;
  minLynxVersion: string;
  sdkType: string;
  sdkVersions: string;
  durationMinutes: number;
  currentPackageVersion: string;
  entitlementRequired: boolean;
  hasEntitlement: boolean;
  packageReady: boolean;
  boardProfiles: CloudLearningBoardProfile[];
}

export interface CloudLearningProgress {
  resourceType: string;
  resourceId: string;
  state: string;
  percent: number;
  sessionId: string;
  revision: number;
  updateTime: number;
}

export interface CloudLearningUnitProgress {
  step: 'document' | 'video' | 'workspace' | 'lab';
  state: 'not_started' | 'in_progress' | 'completed' | 'blocked';
  percent: number;
  completionSource: string;
  revision: number;
  updateTime: number;
}

export interface CloudLearningUnitSummary {
  id: number;
  unitKey: string;
  title: string;
  summary: string;
  sortNo: number;
  documentId: string;
  documentUrl: string;
  documentSha256: string;
  videoProvider: string;
  hasVideo: boolean;
  workspaceRepository: string;
  workspaceCommit: string;
  workspaceSha256: string;
  hasWorkspace: boolean;
  labId: number;
  completionPolicy: string;
  progress?: CloudLearningUnitProgress | null;
  stepProgress: CloudLearningUnitProgress[];
}

export interface CloudLearningEnrollment {
  enrollmentId: string;
  bundleKey: string;
  title: string;
  summary: string;
  track: string;
  boardProfileId: string;
  minLynxVersion: string;
  version: string;
  currentUnit: string;
  currentStep: string;
  revision: number;
  deliveryState: string;
  units: CloudLearningUnitSummary[];
}

export interface CloudLearningHome {
  enrollments: CloudLearningEnrollment[];
}

export interface CloudLearningUnitDetail extends CloudLearningUnitSummary {
  documentMarkdown: string;
}

export interface CloudLearningVideoLaunch {
  url: string;
  verificationSource: 'provider_opened';
}

export interface UpdateLearningUnitProgress {
  step: 'document' | 'video' | 'workspace' | 'lab';
  state: 'not_started' | 'in_progress' | 'completed' | 'blocked';
  percent: number;
  completionSource:
    '' | 'user_confirmed' | 'provider_verified' | 'provider_opened' | 'validator_verified';
  revision: number;
}

export interface InstalledLearningWorkspace {
  enrollmentId: string;
  unitKey: string;
  commit: string;
  sha256: string;
  installPath: string;
  installedAt: number;
  reused: boolean;
}

export interface CloudHardwareGuide {
  id: string;
  title: string;
  summary: string;
  kind: string;
  url: string;
}

export interface CloudHardwareItem {
  id: string;
  name: string;
  model: string;
  status: string;
  serialNumberMasked: string;
  orderNumberMasked: string;
  boundAt: number;
  guides: CloudHardwareGuide[];
}

export interface CloudHardwareOverview {
  items: CloudHardwareItem[];
}

export interface CloudHardwareRedeemResult {
  hardware: CloudHardwareItem;
  message: string;
}

export type LearningDeepLinkTarget =
  | { courseSlug: string; chapterId: string | null; labKey?: never; previewId?: never }
  | { courseSlug: string; labKey: string | null; chapterId?: never; previewId?: never }
  | { previewId: string; courseSlug?: never; labKey?: never; chapterId?: never };

/**
 * Type map for all IPC events.
 *
 * Defines payload types for every Tauri event,
 * enabling type-safe subscription through subscribeEvent.
 * Event names map to IpcEventSpec with typed payload.
 */
export interface IpcEventMap extends SupervisorEventMap, WorkbenchEventMap, McuEventMap {
  'hardware-close-blocked': IpcEventSpec<string[]>;
  'hardware-monitor-update': IpcEventSpec<HardwareMonitorSnapshot>;
  'ch347-device-changed': IpcEventSpec<void>;
  'gateway-interaction': IpcEventSpec<GatewayInteraction>;
  /** USB hot-plug event for device connection/disconnection */
  'usb-hotplug': IpcEventSpec<{
    /** Event type: arrived or left */
    event: 'arrived' | 'left';
    /** USB vendor ID */
    vendorId: number;
    /** USB product ID */
    productId: number;
    /** EFEX device ID if FEL/FES device */
    efexDeviceId: number | null;
    /** USB bus ID */
    busId: number;
    /** USB device ID */
    usbDeviceId: number;
    /** Device path string */
    devicePath: string | null;
    /** USB port number */
    port: number | null;
  }>;

  /** Firmware packer log event */
  'packer-log': IpcEventSpec<{ level: string; message: string }>;

  /** Firmware packer progress event */
  'packer-progress': IpcEventSpec<{
    /** Current stage name */
    stage: string;
    /** Current item count */
    current: number;
    /** Total item count */
    total: number;
    /** Status message */
    message: string;
  }>;

  /** Flash operation progress event */
  'flash-progress': IpcEventSpec<{
    /** Task ID */
    taskId: number;
    /** Stage identifier */
    stageId: string;
    /** Stage display label */
    stageLabel: string;
    /** Stage completion percentage */
    stagePercent: number;
    /** Overall completion percentage */
    overallPercent: number;
    /** Current partition name */
    currentPartition?: string;
    /** Completed partition names */
    completedPartitions: string[];
    /** Partition completion percentage */
    partitionPercent?: number;
    /** Bytes confirmed on media; absent while data is only in transport/queue */
    writtenBytes?: number;
    /** Bytes sent by the host transport */
    transferredBytes?: number;
    /** Bytes accepted by a device-side queue but not yet media-confirmed */
    queuedBytes?: number;
    /** Bytes confirmed on media after the backend's completion criteria */
    committedBytes?: number;
    /** Total bytes to write */
    totalBytes?: number;
    /** Whether progress is indeterminate */
    indeterminate: boolean;
  }>;

  /** Flash operation log event */
  'flash-log': IpcEventSpec<{
    /** Task ID */
    taskId: number;
    /** Log level */
    level: string;
    /** Log message */
    message: string;
    /** Timestamp */
    timestamp: number;
  }>;

  /** Flash operation state change event */
  'flash-state': IpcEventSpec<{
    /** Task ID */
    taskId: number;
    /** New status */
    status: 'started' | 'completed' | 'failed' | 'cancelled';
    /** Status message */
    message?: string;
    /** Error details if failed */
    error?: { code: number; name: string; message: string };
  }>;

  /** Flash popup request event */
  'flash-popup': IpcEventSpec<{
    /** Task ID */
    taskId: number;
    /** Popup type */
    popupType: string;
    /** Popup title */
    title: string;
    /** Popup message */
    message: string;
  }>;

  /** Flash confirmation request event */
  'flash-confirm-request': IpcEventSpec<{
    /** Task ID */
    taskId: number;
    /** Request ID for response */
    requestId: number;
    /** Confirmation kind */
    kind: string;
    /** Confirmation title */
    title: string;
    /** Confirmation message */
    message: string;
  }>;

  /** DRAM initialization info event */
  'flash-dram-info': IpcEventSpec<{
    /** Task ID */
    taskId: number;
    /** Return address */
    retAddr: number;
    /** DRAM init flag */
    dramInitFlag: number;
    /** DRAM update flag */
    dramUpdateFlag: number;
    /** DRAM parameters */
    dramPara: number[];
  }>;

  /** Mass production slot update event */
  'mass-slot-update': IpcEventSpec<{
    /** Slot ID */
    slotId: number;
    /** Slot status */
    status: 'idle' | 'waiting' | 'flashing' | 'success' | 'failed';
    /** Progress percentage */
    progress: number;
    /** Current stage */
    stage: string;
    /** Flash speed */
    speed: string | null;
    /** Error message */
    error: string | null;
    /** USB bus */
    bus: number | null;
    /** USB port */
    port: number | null;
    /** Start time */
    startTime: number | null;
    /** End time */
    endTime: number | null;
  }>;

  /** Mass production log event */
  'mass-log': IpcEventSpec<{
    /** Slot ID or null for general */
    slotId: number | null;
    /** Log level */
    level: string;
    /** Log message */
    message: string;
    /** Timestamp */
    timestamp: number;
  }>;

  /** Mass production state event */
  'mass-state': IpcEventSpec<{
    /** Current state */
    state: 'stopped' | 'running';
    /** Total count */
    total: number;
    /** Success count */
    success: number;
    /** Failed count */
    failed: number;
    /** In progress count */
    inProgress: number;
  }>;

  /** Serial port data received event */
  'serial-data-received': IpcEventSpec<{
    port: string;
    sessionId: string;
    startOffset: number;
    endOffset: number;
    data: number[];
  }>;

  'adb-transfer-progress': IpcEventSpec<AdbTransferSnapshot>;

  /** TCP data received event */
  'tcp-data-received': IpcEventSpec<{
    id: string;
    data: number[];
  }>;

  /** TCP disconnected event */
  'tcp-disconnected': IpcEventSpec<string>;

  /** 100ask.net 登录结果事件（auth_login_start 后异步回传） */
  'auth-login-result': IpcEventSpec<{
    /** 是否登录成功 */
    success: boolean;
    /** 登录成功时的用户信息 */
    user: AuthUserInfo | null;
    /** 登录失败时的错误信息 */
    error: string | null;
  }>;

  /** 操作系统唤起 LYNX 后由 Rust 严格解析并清洗的学习导航目标。 */
  'learning-deep-link': IpcEventSpec<LearningDeepLinkTarget>;
}

/** Extract argument type for a command */
export type CommandArgs<K extends keyof IpcCommandMap> = IpcCommandMap[K]['args'];

/** Extract result type for a command */
export type CommandResult<K extends keyof IpcCommandMap> = IpcCommandMap[K]['result'];

/** Extract payload type for an event */
export type EventPayload<K extends keyof IpcEventMap> = IpcEventMap[K]['payload'];

export interface LearnJourney {
  activityAccess?: LocalActivityAccess[];
  notes: Array<{
    stepId: string;
    attempt: number;
    question: string;
    answer: string;
    source: string;
    createdAt: number;
  }>;
  title: string;
  labs: LocalLearningLab[];
  attempts: Array<{
    stepId: string;
    attempt: number;
    state: string;
    answer: string;
    reflection: string;
    updatedAt: number;
  }>;
  attempt: number;
  answer: string;
  reflection: string;
  evidence: Array<LocalLearningEvidence & { stepId: string; attempt: number }>;
  savedAt: number;
}

export interface LocalActivityAccess {
  stepId: string;
  available: boolean;
  completed: boolean;
  prerequisites: Array<{
    stepId: string;
    title: string;
    status:
      | 'completed'
      | 'not_started'
      | 'incomplete'
      | 'skipped'
      | 'verification_required'
      | 'prerequisite_blocked';
  }>;
}

export interface LearnCapabilities {
  protocols: string[];
  manifestFormats: string[];
  actionCatalogVersions: string[];
  minClientRequirements: string[];
  limits: { maxEventBatchItems: number; maxEventBatchBytes: number; maxJsonBytes: number };
}
export interface LearnCapabilitiesResult {
  availability: 'available' | 'not_deployed';
  server: LearnCapabilities | null;
  clientCapabilities: string[];
  missingCapabilities: string[];
}
