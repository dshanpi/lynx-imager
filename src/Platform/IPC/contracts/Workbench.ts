import type { IpcCommandSpec, IpcEventSpec } from '../Types';

export interface FirmwareApplication {
  id: string;
  name: string;
  description: string;
  boardModel: string;
  system: string;
  routes: string[];
  documentationUrl: string;
  documentationScope: string;
}
export interface FirmwareApplicationCatalog {
  deviceId: number;
  boardUid: string;
  boardName: string;
  hardwareProfile: 'unverified';
  source: 'bundled_documentation';
  publicationStatus: 'github_configured';
  repository: string;
  releasesUrl: string;
  applications: FirmwareApplication[];
}
export interface FirmwareReleaseDiscovery {
  deviceId: number;
  boardUid: string;
  repository: string;
  releasesUrl: string;
  channel: 'stable';
  checkedAtMs: number;
  stableReleases: number;
  releasesWithoutIndex: number;
  indices: Array<{
    releaseId: number;
    tag: string;
    publishedAt: string;
    indexAssetId: number;
    indexSizeBytes: number;
    indexUrl: string;
    releaseUrl: string;
    validation: 'not_checked';
  }>;
  installAuthorized: false;
}
export interface FirmwareCandidateFile {
  fileName: string;
  sizeBytes: number;
  sha256: string;
}
export interface FirmwareCandidateInspection {
  deviceId: number;
  boardUid: string;
  manifest: {
    schemaVersion: number;
    applicationId: string;
    version: string;
    boardModel: string;
    chip: string;
    route: string;
    flashCapacityBytes: number;
    layout: { bootOffset: number; parametersOffset: number; applicationOffset: number };
    image: FirmwareCandidateFile;
    loader: FirmwareCandidateFile;
    loaderSource: string;
    sourceCommit: string;
  };
  imagePath: string;
  loaderPath: string;
  integrity: 'verified';
  hardwareCompatibility: 'unverified';
  loaderCompatibility: 'unverified';
  publicationStatus: 'local_candidate';
  installAuthorized: false;
}

export interface FirmwareHardwareProfile {
  model: string;
  revision: string;
  chip: string;
  capacityBytes: number;
  display: string;
  touch: string;
  operator: string;
  evidence: string;
  bindingFingerprint?: string;
  recordedAt?: number;
}
export interface FirmwareCandidate {
  id: string;
  deviceId: number;
  boardUid: string;
  manifest: FirmwareCandidateInspection['manifest'];
  manifestPath: string;
  imagePath: string;
  loaderPath: string;
  createdAt: number;
  origin: unknown;
  buildId: string | null;
  taskIds: string[];
}
export interface FirmwareConfirmation {
  id: string;
  candidateId: string;
  deviceId: number;
  expiresAt: number;
  postAction: string;
  used: boolean;
  profile: FirmwareHardwareProfile;
  fingerprint: string;
}
export interface FirmwareAcceptanceItem {
  item: string;
  result: 'pass' | 'fail' | 'not_verified';
  evidence: string;
}
export interface FirmwareAcceptance {
  id: string;
  candidateId: string;
  taskId: string;
  imageSha256: string;
  operator: string;
  items: FirmwareAcceptanceItem[];
  recordedAt: number;
  profile: FirmwareHardwareProfile;
  source: string;
}
export interface FirmwareBuildRecord {
  logTail?: string;
  id: string;
  projectId: number;
  hostId: number;
  sdkDir: string;
  status: string;
  error?: string;
  recipe: string;
  createdAt: number;
  receipt: {
    status: string;
    stages: Array<{ name: string; status: string }>;
    artifacts: FirmwareCandidateFile[];
    inputs: Record<string, unknown>;
    toolchain: { version: string; sha256: string };
    hardwareTest: string;
    error?: string;
  } | null;
}
export interface FirmwarePublishRequest {
  requestId: string;
  candidateId: string;
  version: string;
  channel: 'test' | 'stable';
  notes: string;
  distributionLicense: string;
  rightsConfirmed: boolean;
}
export interface FirmwarePublication {
  request?: FirmwarePublishRequest;
  id: string;
  candidateId: string;
  version: string;
  channel: string;
  status: string;
  createdAt: number;
  releaseId: number | null;
  url?: string;
  error?: string;
  imageSha256: string;
}
export interface FirmwareWorkspace {
  deviceId: number;
  boardUid: string;
  profile: FirmwareHardwareProfile | null;
  candidates: FirmwareCandidate[];
  tasks: WorkbenchTaskHistory[];
  acceptance: FirmwareAcceptance[];
  builds: FirmwareBuildRecord[];
  publications: FirmwarePublication[];
}
export interface FirmwareCatalogVersion {
  releaseId: number;
  tag: string;
  indexAssetId: number;
  indexSha256: string;
  imageUrl: string;
  channel: 'test' | 'stable';
  entry: {
    manifest: FirmwareCandidateInspection['manifest'];
    status: string;
    withdrawalReason: string;
    notes: string;
    documentationUrl: string;
    distributionLicense: string;
    hardware: Record<string, unknown>;
    recipe: Record<string, unknown>;
    validation: Record<string, unknown>;
  };
}
export interface FirmwareDownloadRequest {
  releaseId: number;
  applicationId: string;
  version: string;
  channel: string;
  loaderPath: string;
  indexSha256?: string;
}
export type StagedFirmware = Pick<FirmwareCandidate, 'manifest' | 'imagePath' | 'loaderPath'> & {
  releaseId: number;
  indexSha256: string;
};
export interface FirmwareRemoteCatalog {
  deviceId: number;
  boardUid: string;
  checkedAt: number;
  expiresAt: number;
  channel: string;
  entries: FirmwareCatalogVersion[];
}

export interface WorkbenchHost {
  id: number | null;
  name: string;
  address: string;
  port: number;
  username: string;
  authType: 'private_key' | 'password';
  privateKeyPath: string;
  hostFingerprint: string;
  defaultWorkDir: string;
  aiProvider: 'codex' | 'claude_code' | 'deepseek_harness' | 'custom';
  aiExecutable: string;
  aiArgsJson: string;
}

export interface WorkbenchProject {
  id: number | null;
  name: string;
  hostId: number;
  deviceId: number | null;
  sdkDir: string;
  buildCommand: string;
  firmwarePath: string;
  localDownloadDir: string;
  verifySha256: boolean;
  localGatewayPort: number;
  remoteForwardPort: number;
  mcpServerName: string;
  aiProvider: 'codex' | 'claude_code' | 'deepseek_harness';
}

export interface WorkbenchDevice {
  id: number | null;
  boardUid: string;
  name: string;
  boardImagePath: string;
  chipVendor: 'allwinner' | 'rockchip' | 'nxp' | 'kendryte' | 'st';
  serialPort: string;
  serialVid: number;
  serialPid: number;
  serialNumber: string;
  usbBinding: string;
  cameraBinding: string;
  adbSerial: string;
  serialInterfaceNumber: number | null;
  baudRate: number;
  serialConsoleType: 'auto' | 'linux' | 'rt_smart';
  serialLoginMode: 'none' | 'username' | 'password';
  serialLoginUsername: string;
  serialLoginPrompt: string;
  serialPasswordPrompt: string;
  serialShellPrompt: string;
  serialAutoLogin: boolean;
  flasherKind: string;
  flasherPath: string;
  flasherArgs: string;
  powerController: string;
  powerControllerVid: number;
  powerControllerPid: number;
  powerControllerSerial: string;
  powerControllerInterfaceNumber: number | null;
  powerBaudRate: number;
  powerProfileJson: string;
  powerChannel: string;
  bootChannel: string;
  resetChannel: string;
  powerOffWaitMs: number;
  powerOnWaitMs: number;
  serialReconnectTimeoutMs: number;
  bootTimeoutMs: number;
}

export interface CameraDeviceCandidate {
  key: string;
  name: string;
  description: string;
}

export interface LinuxDeviceOverview {
  deviceId: number;
  adbSerial: string;
  transport: 'adb' | 'serial';
  transportDetail: string;
  collectedAtMs: number;
  hostname: string;
  osName: string;
  kernel: string;
  architecture: string;
  cpuModel: string;
  cpuCores: number | null;
  cpuUsagePct: number | null;
  loadOne: number | null;
  loadFive: number | null;
  loadFifteen: number | null;
  memoryTotalBytes: number | null;
  memoryAvailableBytes: number | null;
  storageTotalBytes: number | null;
  storageAvailableBytes: number | null;
  temperatureCelsius: number | null;
  uptimeSeconds: number | null;
  ipAddresses: string[];
}

export type WorkbenchVendorRoute =
  | 'rk-update'
  | 'rk-emmc'
  | 'nxp-uuu'
  | 'nxp-configured'
  | 'st-dfuse'
  | 'st-raw'
  | 'k230-kdimage'
  | 'k230-raw';
export interface VendorFileReference {
  path: string;
  bytes: number;
  sha256: string;
}
export interface WorkbenchVendorInspection {
  dependencies: VendorFileReference[];
  route: WorkbenchVendorRoute;
  path: string;
  bytes: number;
  format: string;
  items: Array<{ label: string; detail: string; ok: boolean }>;
  blockers: string[];
  alternates: number[];
}
export interface WorkbenchFlasherDevice {
  dfuAlternates?: Array<{
    number: number;
    name: string;
    interface?: number;
    configuration?: number;
  }>;
  key: string;
  flasherKind: string;
  bus: number;
  port: string;
  vid: number;
  pid: number;
  mode: string;
  chip: string;
}

export interface WorkbenchPlatform {
  value: WorkbenchDevice['chipVendor'];
  label: string;
  flasher: string;
  protocol: string;
}

export interface WorkbenchTunnelStatus {
  projectId: number;
  state: 'connecting' | 'verifying' | 'connected' | 'reconnecting' | 'stopped';
  message: string;
  localPort: number;
  remotePort: number;
}

export type WorkbenchSshTerminalMode = 'shell' | 'codex' | 'claude_code' | 'deepseek_harness';

export interface WorkbenchSshTerminalSnapshot {
  terminalId: string;
  projectId: number;
  deviceId: number;
  mode: WorkbenchSshTerminalMode;
  title: string;
  workingDirectory: string;
  state: 'connecting' | 'connected' | 'closed' | 'failed';
  message: string;
  startedAtMs: number;
  endedAtMs: number | null;
  exitCode: number | null;
  receivedBytes: number;
  sentBytes: number;
  outputEpoch: number;
  data: number[];
}

export interface WorkbenchSshTerminalClearResult {
  outputEpoch: number;
  receivedBytes: number;
}

export interface WorkbenchCredentials {
  password?: string;
  keyPassphrase?: string;
}

export interface GatewayInteraction {
  id: number;
  timestampMs: number;
  tool: string;
  category: string;
  summary: string;
  deviceKey: string;
  projectId: number | null;
  deviceId: number | null;
}

export interface WorkbenchTaskHistory {
  taskId: string;
  kind: string;
  projectId: number | null;
  deviceId: number | null;
  status: string;
  startedAt: number;
  finishedAt: number | null;
  exitCode: number | null;
  error: string;
  phase: string;
  progressPct: number | null;
  details: {
    phase?: string;
    progressPct?: number | null;
    currentPartition?: string | null;
    completedPartitions?: string[];
    partitionProgressPct?: number | null;
    transferredBytes?: number | null;
    queuedBytes?: number | null;
    committedBytes?: number | null;
    writtenBytes?: number | null;
    totalBytes?: number | null;
    bytesPerSecond?: number | null;
    usbSpeed?: string;
    usbTheoreticalMbps?: number | null;
    storageType?: string;
    physicalVerifyState?: string;
    writeState?: string | null;
    physicalImageSha256?: string | null;
    outsideImageState?: string | null;
    bootVerifyState?: string;
    verifyState?: string;
    verifyErrorCode?: number | null;
    postFlashAction?: string;
    postFlashState?: string;
    postFlashAttempts?: number;
    postFlashError?: string | null;
    diagnosticErrorCode?: string | null;
    outputTail?: string[];
  } | null;
}

export interface WorkbenchWorkflowStep {
  id: number;
  taskId: string;
  stepName: string;
  status: string;
  startedAt: number;
  finishedAt: number | null;
  error: string;
}

export interface WorkbenchCommandMap {
  workbench_list_hosts: IpcCommandSpec<undefined, WorkbenchHost[]>;
  workbench_save_credentials: IpcCommandSpec<{
    hostId: number;
    credentials: WorkbenchCredentials;
  }>;
  workbench_delete_credentials: IpcCommandSpec<{ hostId: number }>;
  workbench_save_host: IpcCommandSpec<
    { host: WorkbenchHost; connectionTestSessionId?: string | null },
    number
  >;
  workbench_list_projects: IpcCommandSpec<undefined, WorkbenchProject[]>;
  workbench_sync_native_skills: IpcCommandSpec<
    { projectId: number; providers: Array<'codex' | 'claude'> },
    NativeSkillSyncResult[]
  >;
  workbench_save_project: IpcCommandSpec<{ project: WorkbenchProject }, number>;
  workbench_list_devices: IpcCommandSpec<undefined, WorkbenchDevice[]>;
  workbench_list_platforms: IpcCommandSpec<undefined, WorkbenchPlatform[]>;
  workbench_firmware_save_profile: IpcCommandSpec<
    { deviceId: number; profile: FirmwareHardwareProfile; confirmed: boolean },
    FirmwareHardwareProfile
  >;
  workbench_firmware_import: IpcCommandSpec<
    { deviceId: number; manifestPath: string },
    FirmwareCandidate
  >;
  workbench_firmware_prepare: IpcCommandSpec<
    { deviceId: number; candidateId: string; postAction: string },
    FirmwareConfirmation
  >;
  workbench_firmware_start: IpcCommandSpec<
    { deviceId: number; confirmationId: string; confirmed: boolean },
    string
  >;
  workbench_firmware_acceptance: IpcCommandSpec<
    {
      deviceId: number;
      candidateId: string;
      taskId: string;
      operator: string;
      items: FirmwareAcceptanceItem[];
    },
    FirmwareAcceptance
  >;
  workbench_firmware_report: IpcCommandSpec<{ deviceId: number; candidateId: string }, string>;
  workbench_firmware_workspace: IpcCommandSpec<{ deviceId: number }, FirmwareWorkspace>;
  workbench_firmware_catalog: IpcCommandSpec<
    { deviceId: number; channel: string },
    FirmwareRemoteCatalog
  >;
  imager_firmware_catalog: IpcCommandSpec<undefined, FirmwareCatalogVersion[]>;
  imager_stage_firmware: IpcCommandSpec<{ request: FirmwareDownloadRequest }, StagedFirmware>;
  workbench_firmware_download: IpcCommandSpec<
    { deviceId: number; request: FirmwareDownloadRequest },
    FirmwareCandidate
  >;
  workbench_firmware_build_start: IpcCommandSpec<
    { deviceId: number; projectId: number; requestId: string; credentials?: WorkbenchCredentials },
    FirmwareBuildRecord
  >;
  workbench_firmware_build_status: IpcCommandSpec<
    { deviceId: number; buildId: string; credentials?: WorkbenchCredentials },
    FirmwareBuildRecord
  >;
  workbench_firmware_build_candidate: IpcCommandSpec<
    { deviceId: number; buildId: string; loaderPath: string; credentials?: WorkbenchCredentials },
    FirmwareCandidate
  >;
  workbench_firmware_publish_check: IpcCommandSpec<
    { deviceId: number; request: FirmwarePublishRequest },
    {
      ready: boolean;
      localReady: boolean;
      localError: string | null;
      permission: boolean;
      permissionError: string | null;
      actor: string | null;
    }
  >;
  workbench_firmware_publish: IpcCommandSpec<
    { deviceId: number; request: FirmwarePublishRequest; confirmed: boolean },
    FirmwarePublication
  >;
  workbench_firmware_release_action: IpcCommandSpec<
    { deviceId: number; publicationId: string; action: string; reason: string; confirmed: boolean },
    FirmwarePublication
  >;
  workbench_firmware_releases: IpcCommandSpec<{ deviceId: number }, FirmwareReleaseDiscovery>;
  workbench_firmware_applications: IpcCommandSpec<{ deviceId: number }, FirmwareApplicationCatalog>;
  workbench_inspect_firmware_candidate: IpcCommandSpec<
    { deviceId: number; manifestPath: string },
    FirmwareCandidateInspection
  >;
  workbench_list_cameras: IpcCommandSpec<undefined, CameraDeviceCandidate[]>;
  workbench_save_device: IpcCommandSpec<{ device: WorkbenchDevice }, number>;
  workbench_serial_login_password_status: IpcCommandSpec<{ deviceId: number }, boolean>;
  workbench_save_serial_login_password: IpcCommandSpec<{
    deviceId: number;
    password: string | null;
  }>;
  workbench_linux_device_overview: IpcCommandSpec<{ deviceId: number }, LinuxDeviceOverview>;
  workbench_start_tunnel: IpcCommandSpec<{ projectId: number }, WorkbenchTunnelStatus>;
  workbench_stop_tunnel: IpcCommandSpec<{ projectId: number }, WorkbenchTunnelStatus>;
  workbench_list_tunnel_statuses: IpcCommandSpec<undefined, WorkbenchTunnelStatus[]>;
  workbench_ssh_terminal_open: IpcCommandSpec<
    {
      projectId: number;
      deviceId: number;
      mode: WorkbenchSshTerminalMode;
      cols: number;
      rows: number;
    },
    WorkbenchSshTerminalSnapshot
  >;
  workbench_ssh_terminal_list: IpcCommandSpec<
    { projectId: number; deviceId: number },
    WorkbenchSshTerminalSnapshot[]
  >;
  workbench_ssh_terminal_snapshot: IpcCommandSpec<
    { terminalId: string; projectId: number; deviceId: number },
    WorkbenchSshTerminalSnapshot
  >;
  workbench_ssh_terminal_claim_input: IpcCommandSpec<{
    terminalId: string;
    projectId: number;
    deviceId: number;
    surfaceId: string;
  }>;
  workbench_ssh_terminal_write: IpcCommandSpec<{
    terminalId: string;
    projectId: number;
    deviceId: number;
    surfaceId: string;
    data: number[];
  }>;
  workbench_ssh_terminal_resize: IpcCommandSpec<{
    terminalId: string;
    projectId: number;
    deviceId: number;
    cols: number;
    rows: number;
  }>;
  workbench_ssh_terminal_clear: IpcCommandSpec<
    {
      terminalId: string;
      projectId: number;
      deviceId: number;
    },
    WorkbenchSshTerminalClearResult
  >;
  workbench_ssh_terminal_close: IpcCommandSpec<{
    terminalId: string;
    projectId: number;
    deviceId: number;
  }>;
  workbench_list_tasks: IpcCommandSpec<{ limit?: number }, WorkbenchTaskHistory[]>;
  workbench_list_workflow_steps: IpcCommandSpec<{ taskId: string }, WorkbenchWorkflowStep[]>;
  workbench_delete_config: IpcCommandSpec<{ kind: string; id: number }>;
  workbench_test_ssh: IpcCommandSpec<
    { host: WorkbenchHost; credentials: WorkbenchCredentials; allowUntrusted: boolean },
    { fingerprint: string; trusted: boolean; auditSessionId: string }
  >;
  workbench_test_ai_channel: IpcCommandSpec<
    { host: WorkbenchHost; credentials: WorkbenchCredentials },
    { provider: string; executable: string; version: string; auditSessionId: string }
  >;
  workbench_run_ai_task: IpcCommandSpec<
    { projectId: number; prompt: string; credentials: WorkbenchCredentials },
    string
  >;
  workbench_run_build: IpcCommandSpec<
    { projectId: number; credentials: WorkbenchCredentials },
    string
  >;
  workbench_download_firmware: IpcCommandSpec<
    { projectId: number; credentials: WorkbenchCredentials },
    string
  >;
  workbench_upload_file: IpcCommandSpec<
    {
      hostId: number;
      localPath: string;
      remotePath: string;
      overwrite: boolean;
      credentials: WorkbenchCredentials;
    },
    string
  >;
  workbench_cancel_task: IpcCommandSpec<{ taskId: string }>;
  workbench_test_flasher: IpcCommandSpec<{ kind: string; path: string }, string>;
  imager_inspect_vendor_image: IpcCommandSpec<
    { route: WorkbenchVendorRoute; path: string },
    WorkbenchVendorInspection
  >;
  workbench_scan_flasher_devices: IpcCommandSpec<
    { kind: string; path: string },
    WorkbenchFlasherDevice[]
  >;
  workbench_start_flash: IpcCommandSpec<
    {
      deviceId: number;
      firmwarePath: string;
      options?: {
        systemType: 'vendor' | 'mainline';
        loaderPath?: string;
        vendorSetup?: {
          route: WorkbenchVendorRoute;
          medium?: string;
          alt?: number;
          address?: string;
          targetKey?: string;
          eraseConfirmed?: boolean;
          dependencies?: VendorFileReference[];
        };
        mainlineRoute?: 'mainline_nand' | 'mainline_emmc' | 'mainline_spinor' | 'rockchip_raw_emmc';
        mode?: 'full_erase' | 'partition' | 'partition_erase';
        verify?: boolean;
        postFlashAction?: 'reboot' | 'none' | 'poweroff';
      };
    },
    string
  >;
  workbench_power_action: IpcCommandSpec<
    {
      deviceId: number;
      action:
        'power_on' | 'power_off' | 'boot_assert' | 'boot_release' | 'reset_pulse' | 'get_state';
      pulseMs?: number;
    },
    string
  >;
  workbench_power_last_known_state: IpcCommandSpec<{ deviceId: number }, 'on' | 'off' | 'unknown'>;
  workbench_start_workflow: IpcCommandSpec<
    { projectId: number; deviceId: number; credentials: WorkbenchCredentials },
    string
  >;
}

export interface NativeSkillSyncResult {
  provider: 'codex' | 'claude';
  root: string;
  written: string[];
  unchanged: string[];
  conflicts: string[];
}

export interface WorkbenchEventMap {
  'workbench-task-event': IpcEventSpec<{
    taskId: string;
    phase: string;
    stream: string;
    message: string;
    current: number | null;
    total: number | null;
    status: string;
  }>;
  'workbench-flash-telemetry': IpcEventSpec<{
    taskId: string;
    telemetry: NonNullable<WorkbenchTaskHistory['details']>;
  }>;
  'workbench-ssh-terminal-data': IpcEventSpec<{
    terminalId: string;
    projectId: number;
    deviceId: number;
    outputEpoch: number;
    receivedBytes: number;
    data: number[];
  }>;
  'workbench-ssh-terminal-status': IpcEventSpec<{
    terminalId: string;
    projectId: number;
    deviceId: number;
    state: WorkbenchSshTerminalSnapshot['state'];
    message: string;
    endedAtMs: number | null;
    exitCode: number | null;
    sentBytes: number;
    receivedBytes: number;
  }>;
}

export interface LocalMcpStatus {
  projectId: number;
  deviceId: number;
  running: boolean;
  verified: boolean;
  port: number | null;
  endpoint: string | null;
  message: string;
}
