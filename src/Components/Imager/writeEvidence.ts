import type {
  FirmwareAcceptance,
  WorkbenchTaskHistory,
} from '../../Platform/IPC/contracts/Workbench';

type Details = NonNullable<WorkbenchTaskHistory['details']>;
type TaskEvidence = {
  status: string;
  details: Details;
  logs: string[];
  evidenceLines?: string[];
  error: string;
};
export type WriteFailure = { title: string; impact: string; recovery: string; evidence: string };

export const positive = (state?: string | null) =>
  ['success', 'verified', 'completed', 'passed', 'executed'].includes(state ?? '');

export function retainEvidence(previous: string[], incoming: string[]) {
  const next = new Set(previous);
  for (const line of incoming) {
    if (
      /RAW_(?:WRITE_|SPINOR_(?:PHYSICAL_|FES_STEP:erase_policy|CONTRACT:|EVIDENCE:|ERASE_POLICY_DOWNLOAD_FAILED)|MBR_DOWNLOAD_START)/.test(
        line
      )
    )
      next.add(line.slice(0, 4096));
  }
  return [...next].slice(-48);
}

export function physicalPlan(lines: string[], imageBytes: number) {
  const contract = lines.find((line) => line.includes('RAW_SPINOR_CONTRACT:'));
  if (!contract || !Number.isSafeInteger(imageBytes) || imageBytes <= 0) return null;
  const field = (name: string) =>
    Number(contract.match(new RegExp('(?:[:,])' + name + '=(\\d+)'))?.[1]);
  const capacity = field('capacity');
  const alignment = field('eraseAlignment');
  const span = field('writeBytes');
  if (
    ![capacity, alignment, span].every((v) => Number.isSafeInteger(v) && v > 0) ||
    span > capacity ||
    span !== Math.ceil(imageBytes / alignment) * alignment
  )
    return null;
  return { capacity, alignment, span, blocks: span / alignment, keep: span - imageBytes };
}

export function failureEvidence(task: TaskEvidence): WriteFailure {
  const d = task.details;
  const lines = [
    ...(task.evidenceLines ?? []),
    ...task.logs,
    ...(d.outputTail ?? []),
    task.error,
    d.diagnosticErrorCode ?? '',
  ];
  const has = (marker: string) => lines.some((line) => line.includes(marker));
  const error = task.error || d.postFlashError || d.diagnosticErrorCode || '后端未提供错误码。';
  const writeConfirmed = positive(d.writeState) || has('RAW_WRITE_COMPLETE:medium=spinor');
  const writeStarted =
    writeConfirmed ||
    d.writeState === 'running' ||
    d.writeState === 'failed' ||
    has('RAW_WRITE_START') ||
    (d.writtenBytes ?? 0) > 0;
  if (has('RAW_SPINOR_PHYSICAL_VERIFY_FAILED') || d.physicalVerifyState === 'failed') {
    const mismatch = lines.find((line) =>
      /RAW_SPINOR_PHYSICAL_VERIFY_FAILED[^\n]*(mismatch|differ|不一致)/i.test(line)
    );
    return {
      title: mismatch ? '物理回读与预期不一致' : '物理回读失败',
      impact: writeConfirmed
        ? '后端已确认镜像写入完成，物理布局尚未通过校验。'
        : '物理校验失败。后端没有确认完整镜像写入成功。',
      recovery:
        '保留本次日志、写入计划和回读数据。核对镜像与 Loader 后，重新进入烧录模式并检测设备。',
      evidence: mismatch || error,
    };
  }
  if (
    !writeStarted &&
    has('RAW_SPINOR_ERASE_POLICY_DOWNLOAD_FAILED') &&
    !has('RAW_SPINOR_FES_STEP:erase_policy_download:success') &&
    !has('RAW_MBR_DOWNLOAD_START')
  ) {
    const policy = lines.find((line) => line.includes('RAW_SPINOR_ERASE_POLICY_DOWNLOAD_FAILED:'));
    const packet = lines.join('\n').match(/erase_policy_download:start,bytes=(\d+)/);
    const backup = has('RAW_SPINOR_EVIDENCE:') && has('RAW_SPINOR_CONTRACT:');
    return {
      title: '擦除策略下发失败',
      impact:
        (backup ? '物理备份已完成。' : '') + '流程停在策略下发，尚未执行策略确认、MBR 或镜像写入。',
      recovery: '先导出诊断记录，再检查 USB 连接并重新进入 FEL；重新检测后核对备份与目标设备。',
      evidence: (packet ? '策略请求 ' + packet[1] + ' B · ' : '') + (policy || error),
    };
  }
  if (!writeStarted && has('RAW_SPINOR_PHYSICAL_LAYOUT_UNVERIFIED')) {
    return {
      title: '物理布局预检未通过',
      impact: 'Loader 的物理地址映射尚未验证，后端已在擦除和写入前停止。',
      recovery: '核对目标板型和 RAM Loader，再重新检测设备。',
      evidence: error,
    };
  }
  return {
    title:
      task.status === 'cancelled'
        ? '任务已取消'
        : task.status === 'terminated'
          ? '任务已终止'
          : '烧录失败',
    impact: writeConfirmed
      ? '镜像写入已完成，请核对后续校验和结束动作。'
      : writeStarted
        ? '写入已开始，完整性尚未确认。'
        : '现有记录不足以确定目标存储是否已修改。',
    recovery: '导出诊断记录，核对最后一个成功阶段、设备身份和镜像后重新检测。',
    evidence: error,
  };
}

export function physicalReadback(details: Details, logs: string[], expectedHash: string) {
  if (!positive(details.physicalVerifyState)) return null;
  const marker = logs
    .map((line) =>
      line.match(
        /RAW_SPINOR_PHYSICAL_VERIFY_SUCCESS:imageSha256=([a-f\d]{64}),imageBytes=(\d+),fullFlashBytes=(\d+),outsideImage=unchanged/i
      )
    )
    .find((match) => match != null);
  const imageHash = (details.physicalImageSha256 || marker?.[1] || '').toLowerCase();
  return {
    matches: imageHash && expectedHash ? imageHash === expectedHash.toLowerCase() : null,
    imageHash,
    fullBytes: marker ? Number(marker[3]) : null,
    outsidePreserved: details.outsideImageState === 'unchanged' || !!marker,
  };
}

export interface BootScope {
  taskId: string;
  deviceId: number;
  imageSha256: string;
}
export interface BootReport extends BootScope {
  result: 'pass' | 'fail';
  operator: string;
  evidence: string;
  coldBoot: boolean;
  recordedAt: number;
}

export function selectBootAcceptance(
  records: FirmwareAcceptance[],
  taskId: string,
  deviceId: number,
  imageSha256?: string | null,
  candidateId?: string
): BootReport | null {
  // Firmware workspace returns newest first, including a stable tie-break for same-second reports.
  const record = records.find(
    (r) =>
      r.taskId === taskId &&
      (!candidateId || r.candidateId === candidateId) &&
      (!imageSha256 || r.imageSha256.toLowerCase() === imageSha256.toLowerCase()) &&
      r.items.some((item) => item.item === 'boot')
  );
  const boot = record?.items.find((item) => item.item === 'boot');
  if (!record || !boot || boot.result === 'not_verified') return null;
  return {
    taskId,
    deviceId,
    imageSha256: record.imageSha256,
    result: boot.result,
    operator: record.operator,
    recordedAt: record.recordedAt,
    evidence: boot.evidence,
    coldBoot: record.items.some((item) => item.item === 'power_cycle' && item.result === 'pass'),
  };
}

export function sameBootScope(report: BootScope, scope: BootScope) {
  return (
    report.taskId === scope.taskId &&
    report.deviceId === scope.deviceId &&
    report.imageSha256.toLowerCase() === scope.imageSha256.toLowerCase()
  );
}

export function inspectSerialEvidence(contents: string, expected: string) {
  const marker = expected.trim();
  if (marker.length < 4 || marker.length > 120) return null;
  const lines = contents
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(
      (line) =>
        line.includes(marker) && line.length < 512 && !/(?:^|\s)(?:echo|printf)\s/i.test(line)
    );
  if (!lines.length) return null;
  // Keep a bounded excerpt for the existing acceptance record, never the complete serial capture.
  const clean = (line: string) =>
    Array.from(line)
      .filter((c) => c.charCodeAt(0) >= 32 && c.charCodeAt(0) !== 127)
      .join('');
  return {
    count: lines.length,
    first: clean(lines[0]),
    last: lines.length > 1 ? clean(lines[lines.length - 1]) : '',
  };
}
