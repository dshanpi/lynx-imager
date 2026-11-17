export const FLASH_PHASE_LABELS: Record<string, string> = {
  idle: '等待',
  queued: '排队',
  preflight: '准备',
  reconnecting: 'FES 重连',
  query_device: '查询烧录设备',
  raw_metadata: '校验镜像元数据',
  raw_prepare: '准备完整磁盘镜像',
  raw_decompress: '解压系统镜像',
  rk_loader: '下载 RK RAM Loader',
  rk_raw_preflight: '检查 eMMC 与校验能力',
  rk_reconnecting: '等待 RK Loader 重连',
  raw_bootstrap_vendor_fes: '启动 RAM FES Loader',
  raw_scan_target: '查找 FEL / FES 设备',
  raw_open_usb: '打开烧录 USB',
  raw_fes_handshake: '连接 FES 服务',
  raw_fes_ready: 'FES 已就绪',
  raw_query_secure: '检查启动安全模式',
  raw_query_storage: '检测实际存储类型',
  spinor_preflight_blocked: '预检拒绝 · 未开始擦写',
  spinor_probe_capacity: '检测 SPI NOR 容量',
  raw_spinor_physical_backup: '备份 SPI NOR 并核对物理地址',
  spinor_write_complete: 'SPI NOR 写入完成',
  spinor_physical_verify: 'SPI NOR 全片物理回读校验',
  spinor_awaiting_boot: '物理校验通过 · 等待启动验收',
  spinor_mbr_write: '写入单份 16 KiB MBR',
  spinor_awaiting_acceptance: '流程结束 · 等待物理与启动验收',
  raw_probe_capacity: '检测 eMMC 容量',
  raw_mbr_write: '写入四副本 MBR',
  raw_mbr_verify: '校验 MBR',
  emmc_boot0_preflight: '校验主线 SPL',
  emmc_boot0_write: '写入 eMMC 硬件 Boot0',
  emmc_boot0_verify: '确认 eMMC Boot0 状态',
  opening_device: '打开 FEL 设备',
  device_opened: '连接 FEL 设备',
  writing_artifacts: '写入主线内存镜像',
  executing_entry: '启动主线入口',
  waiting_for_installer: '等待板端安装器',
  installer_started: '启动 NAND 安装器',
  installer_verify_payload: '校验烧录载荷',
  installer_write_spl: '写入 SPL',
  installer_write_uboot: '写入 U-Boot',
  installer_write_boot: '写入启动 FIT',
  installer_format_ubi: '格式化 UBI',
  installer_verify_rootfs: '验证根文件系统',
  installer_complete: 'NAND 烧录完成',
  installer_failed: 'NAND 烧录失败',
  erase: '擦除',
  write: '写入',
  mbr_write: '写入分区表（MBR）',
  verify: '校验',
  post_flash_action: '烧后动作',
  complete: '完成',
};

const OPENIX_STAGE_PHASES: Record<string, string> = {
  Initializing: 'preflight',
  'DRAM Init': 'raw_bootstrap_vendor_fes',
  'U-Boot Download': 'raw_bootstrap_vendor_fes',
  Reconnecting: 'reconnecting',
  'Downloading Loader': 'rk_loader',
  'Checking Raw eMMC': 'rk_raw_preflight',
  'RK Loader Reconnecting': 'rk_reconnecting',
  'Query Device': 'query_device',
  Erasing: 'erase',
  'Writing MBR': 'mbr_write',
  'Flashing Partitions': 'write',
  'Writing Boot': 'write',
  'Setting Mode': 'post_flash_action',
};

const TERMINAL_TASK_STATES = new Set(['failed', 'cancelled', 'success', 'completed']);

function parseJsonLine(line: string): Record<string, unknown> | null {
  try {
    const value = JSON.parse(line) as unknown;
    return value && typeof value === 'object' && !Array.isArray(value)
      ? (value as Record<string, unknown>)
      : null;
  } catch {
    return null;
  }
}

function numeric(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

export function deriveFlashPhase(storedPhase: string, outputTail: string[] = []) {
  if (
    storedPhase === 'complete' ||
    storedPhase === 'spinor_awaiting_acceptance' ||
    storedPhase === 'spinor_awaiting_boot' ||
    storedPhase === 'spinor_preflight_blocked'
  )
    return storedPhase;
  let observed = '';
  for (const line of outputTail) {
    const value = parseJsonLine(line);
    const message =
      value?.type === 'log' && typeof value.message === 'string' ? value.message : line;
    if (message.includes('RAW_SPINOR_PHYSICAL_LAYOUT_UNVERIFIED')) {
      observed = 'spinor_preflight_blocked';
    } else if (message.includes('RAW_SPINOR_PHYSICAL_VERIFY_')) {
      observed = 'spinor_physical_verify';
    } else if (value?.type === 'stage' && typeof value.stage === 'string') {
      observed = OPENIX_STAGE_PHASES[value.stage] ?? observed;
    } else if (line.includes('RAW_ERASE_START')) {
      observed = 'erase';
    } else if (line.includes('RAW_WRITE_START')) {
      observed = 'write';
    }
  }
  return observed || storedPhase || 'idle';
}

export function restoreFlashOutputTail(outputTail: string[] = [], error = '') {
  const lines = outputTail.map((line) => {
    const value = parseJsonLine(line);
    if (value?.type === 'log' && typeof value.message === 'string') {
      const level = typeof value.level === 'string' ? value.level.toUpperCase() : 'INFO';
      return `[${level}] ${value.message}`;
    }
    if (value?.type === 'stage' && typeof value.stage === 'string') {
      const status = typeof value.status === 'string' ? value.status : 'update';
      return `[STAGE] ${value.stage} · ${status}`;
    }
    if (value?.type === 'progress') {
      const percent = numeric(value.percent);
      return percent == null ? '[PROGRESS] 等待有效进度' : `[PROGRESS] ${percent.toFixed(1)}%`;
    }
    if (value?.type === 'verify') {
      return `[VERIFY] ${typeof value.status === 'string' ? value.status : 'update'}`;
    }
    if (value?.type === 'post_action') {
      return `[POST] ${typeof value.status === 'string' ? value.status : 'update'}`;
    }
    const trimmed = line.trim();
    if (/^error:\s*/i.test(trimmed)) return `[ERROR] ${trimmed.replace(/^error:\s*/i, '')}`;
    return `[OUTPUT] ${trimmed}`;
  });
  if (error && !lines.some((line) => line === `[SYSTEM] ${error}`)) {
    lines.push(`[SYSTEM] ${error}`);
  }
  return lines.filter((line) => line !== '[OUTPUT] ').slice(-600);
}

export function formatFlashPhase(phase: string, taskStatus: string) {
  const label = FLASH_PHASE_LABELS[phase] ?? (phase || '等待');
  if (phase === 'spinor_preflight_blocked') return label;
  if (taskStatus === 'failed') return `${label}失败`;
  if (taskStatus === 'cancelled') return `${label}时取消`;
  return label;
}

export function formatUsbSpeed(speed: string, theoreticalMbps?: number | null) {
  const normalized = speed.trim().toLowerCase();
  if (!normalized || normalized === 'unknown') return '待检测';
  const defaults: Record<string, number> = {
    full: 12,
    high: 480,
    super: 5000,
    'super-plus': 10000,
  };
  const labels: Record<string, string> = {
    full: 'Full Speed',
    high: 'High Speed',
    super: 'SuperSpeed',
    'super-plus': 'SuperSpeed+',
  };
  const mbps = theoreticalMbps ?? defaults[normalized];
  const label = labels[normalized] ?? speed;
  return mbps ? `${label} · ${mbps.toLocaleString('en-US')} Mbps` : label;
}

export function formatVerifyState(state: string, taskStatus: string) {
  const normalized = state.trim().toLowerCase();
  if (
    (normalized === 'pending' || normalized === 'not_run') &&
    TERMINAL_TASK_STATES.has(taskStatus)
  ) {
    return '未执行';
  }
  return (
    {
      pending: '等待校验',
      not_run: '未执行',
      running: '校验中',
      success: '通过',
      failed: '失败',
      skipped: '已跳过',
    }[normalized] ?? state
  );
}

export function formatPostFlashState(action: string, state: string, taskStatus: string) {
  const actionLabel =
    { reboot: '重启', poweroff: '关机', shutdown: '关机', none: '无操作', unknown: '未配置' }[
      action.trim().toLowerCase()
    ] ?? action;
  const normalized = state.trim().toLowerCase();
  const stateLabel =
    (normalized === 'pending' || normalized === 'not_run') && TERMINAL_TASK_STATES.has(taskStatus)
      ? '未执行'
      : ({
          pending: '等待',
          not_run: '未执行',
          running: '执行中',
          success: '完成',
          failed: '失败',
          skipped: '已跳过',
        }[normalized] ?? state);
  return `${actionLabel} · ${stateLabel}`;
}

export function formatFlashRate(bytesPerSecond: number | null, taskStatus: string) {
  if (!bytesPerSecond) return TERMINAL_TASK_STATES.has(taskStatus) ? '未开始' : '—';
  if (bytesPerSecond >= 1024 * 1024) return `${(bytesPerSecond / 1024 / 1024).toFixed(1)} MB/s`;
  if (bytesPerSecond >= 1024) return `${(bytesPerSecond / 1024).toFixed(0)} KB/s`;
  return `${bytesPerSecond.toFixed(0)} B/s`;
}

export function formatIndependentVerification(state = 'not_verified') {
  return (
    { success: '通过', failed: '失败', running: '验证中', not_verified: '未验证' }[state] ??
    '未验证'
  );
}

export function flashOutcomeLabel(
  status: string,
  storageType?: string,
  bootState?: string,
  physicalState?: string
) {
  if (status === 'success' && storageType === 'spinor') {
    if (bootState === 'success' && physicalState === 'success') return '验收通过';
    if (physicalState === 'success') return '物理校验通过 · 待启动验收';
    return '流程结束 · 尚未验收';
  }
  return { success: '烧录成功', failed: '烧录失败', running: '正在烧录' }[status] ?? status;
}

export function formatMediaWriteState(
  committed?: number | null,
  total?: number | null,
  taskStatus = 'idle'
) {
  if (typeof total === 'number' && total > 0 && committed === total) return '写入完成';
  return taskStatus === 'running' ? '等待写入确认' : '未确认完成';
}
