import type { FirmwarePublishRequest } from '../../Platform/IPC/contracts/Workbench';
import type {
  FirmwareApplication,
  FirmwareReleaseDiscovery,
  FirmwareCandidateInspection,
  WorkbenchDevice,
} from '../../Platform/IPC/contracts/Workbench';

export function filterFirmwareApplications(
  applications: FirmwareApplication[],
  query: string,
  boardModel: string,
  system: string
) {
  const text = query.trim().toLocaleLowerCase();
  return applications.filter(
    (app) =>
      (!boardModel || app.boardModel === boardModel) &&
      (!system || app.system === system) &&
      (!text || `${app.name} ${app.description}`.toLocaleLowerCase().includes(text))
  );
}

export function firmwareRouteLabel(routes: string[]) {
  return routes.length === 1 && routes[0] === 'xip'
    ? 'XIP · 完整镜像'
    : routes.map((route) => route.toUpperCase()).join(' / ');
}

export function candidateMatchesScope(
  candidate: FirmwareCandidateInspection,
  board: Pick<WorkbenchDevice, 'id' | 'boardUid'>,
  appId: string
) {
  return (
    board.id != null &&
    candidate.deviceId === board.id &&
    candidate.boardUid === board.boardUid &&
    candidate.manifest.applicationId === appId
  );
}

export function candidateErrorMessage(error: unknown) {
  const value =
    error && typeof error === 'object'
      ? (error as { machineCode?: unknown; message?: unknown })
      : null;
  const code =
    typeof value?.machineCode === 'string'
      ? value.machineCode
      : typeof value?.message === 'string'
        ? value.message
        : typeof error === 'string'
          ? error
          : '';
  const messages: Record<string, string> = {
    FIRMWARE_CANDIDATE_ROUTE_UNSUPPORTED: '此候选包不属于当前支持核对的 NekoBox XIP 路线。',
    FIRMWARE_CANDIDATE_LAYOUT_INVALID: '镜像大小、存储容量或 XIP 布局与配方不符。',
    FIRMWARE_CANDIDATE_HASH_MISMATCH: '文件 SHA-256 与清单不一致，候选包不可使用。',
    FIRMWARE_CANDIDATE_FILE_CHANGED: '核对期间文件发生变化，请重新取得完整候选包。',
    FIRMWARE_CANDIDATE_FILE_SIZE_OR_TYPE: '文件大小或类型不符；不能使用半包或符号链接。',
    FIRMWARE_CANDIDATE_FILE_MISSING: '清单对应的镜像或 Loader 文件缺失。',
    FIRMWARE_CANDIDATE_FILE_INVALID: '清单文件路径或校验信息不合法。',
    FIRMWARE_CANDIDATE_MANIFEST_INVALID: '候选清单格式不受支持或含未知字段。',
    FIRMWARE_CANDIDATE_METADATA_INVALID: '候选版本、源码提交或 Loader 来源信息缺失。',
  };
  return messages[code] || '候选包无法核对，请检查清单与文件是否完整。';
}

export function releaseDiscoveryMessage(result: FirmwareReleaseDiscovery | null) {
  if (!result) return 'GitHub 固件源已配置 · 尚未读取发布目录';
  if (!result.stableReleases) return '该仓库尚未发布稳定版本';
  if (!result.indices.length)
    return `已有 ${result.stableReleases} 个稳定版本，但未提供固件目录文件`;
  return `发现 ${result.indices.length} 份固件目录文件 · 内容与下载完整性尚未核对`;
}

export function releaseErrorMessage(error: unknown) {
  const value =
    error && typeof error === 'object'
      ? (error as { machineCode?: unknown; message?: unknown })
      : null;
  const code =
    typeof value?.machineCode === 'string'
      ? value.machineCode
      : typeof value?.message === 'string'
        ? value.message
        : typeof error === 'string'
          ? error
          : '';
  const messages: Record<string, string> = {
    FIRMWARE_RELEASE_RATE_LIMITED: 'GitHub 请求受限，请稍后重试。',
    FIRMWARE_RELEASE_TIMEOUT: '读取 GitHub 发布目录超时，请检查网络后重试。',
    FIRMWARE_RELEASE_NETWORK_FAILED: '无法连接 GitHub，请检查网络后重试。',
    FIRMWARE_RELEASE_SOURCE_UNAVAILABLE: '固件仓库无法访问，请核对仓库状态。',
    FIRMWARE_RELEASE_DATA_INVALID: '发布目录格式不符合要求，不能作为可用固件使用。',
    FIRMWARE_RELEASE_INDEX_INVALID: '固件目录文件的来源或元信息异常，已停止读取。',
    FIRMWARE_RELEASE_CATALOG_TOO_LARGE: '发布记录超过当前读取上限，未使用不完整目录。',
    FIRMWARE_RELEASE_RESPONSE_TOO_LARGE: '发布目录响应过大，已停止读取。',
  };
  return messages[code] || '发布目录读取失败，请稍后重试。';
}

// A persisted draft may resume only with exactly the original publication inputs.
export function firmwarePublicationRequestId(
  saved: FirmwarePublishRequest | null,
  current: FirmwarePublishRequest,
  fresh: string
): string {
  const fields = [
    'candidateId',
    'version',
    'channel',
    'notes',
    'distributionLicense',
    'rightsConfirmed',
  ] as const;
  return saved && fields.every((key) => saved[key] === current[key]) ? saved.requestId : fresh;
}
