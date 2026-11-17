import React, { useEffect, useRef, useState } from 'react';
import { ask, open, save } from '@tauri-apps/plugin-dialog';
import { stat, writeTextFile } from '@tauri-apps/plugin-fs';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { LynxIcon } from '../../CoreUI';
import { invokeCommand } from '../../Platform/IPC';
import type { WorkbenchDevice, WorkbenchTaskHistory } from '../../Platform/IPC/Commands';
import { formatErrorMessage } from '../../Utils';
import { firmwareSelectionKey, readFirmwareSelection } from './firmwareSelection';
import {
  flashOutcomeLabel,
  formatIndependentVerification,
  formatMediaWriteState,
  formatFlashPhase,
  formatFlashRate,
  formatPostFlashState,
  formatVerifyState,
  restoreFlashOutputTail,
} from './flashTelemetryPresentation';
import './FirmwareFlashPanel.css';
import FirmwareApplicationsPanel from './FirmwareApplicationsPanel';

interface FlashSnapshot {
  flashLines: string[];
  flashProgress: number;
  flashState: string;
  flashPhase: string;
  flashPartition: string;
  flashRate: number | null;
  flashStorageType?: string;
  flashPhysicalVerify?: string;
  flashBootVerify?: string;
  flashCommittedBytes?: number | null;
  flashTotalBytes?: number | null;
  flashVerify: string;
  flashPostActionKind: string;
  flashPostAction: string;
  flashTaskStatus: string;
}

interface Props {
  applicationCandidateId?: string;
  board: WorkbenchDevice;
  detected: boolean;
  protocol: string;
  runtime: FlashSnapshot;
  tasks: WorkbenchTaskHistory[];
  aiChannel: React.ReactNode;
  onOpenSettings(): void;
  onTaskStarted(taskId: string): void;
}

export default function FirmwareFlashPanel({
  applicationCandidateId,
  board,
  detected,
  protocol,
  runtime,
  tasks,
  aiChannel,
  onOpenSettings,
  onTaskStarted,
}: Props) {
  const [selection, setSelection] = useState(() =>
    readFirmwareSelection(board.id, board.flasherArgs)
  );
  const { systemType, mainlineRoute } = selection;
  const spinor =
    systemType === 'mainline' &&
    board.chipVendor === 'allwinner' &&
    mainlineRoute === 'mainline_spinor';
  const rkRaw = systemType === 'mainline' && board.chipVendor === 'rockchip';
  const loaderPath = rkRaw ? selection.rockchipLoaderPath : selection.loaderPath;
  const loaderLabel = rkRaw ? 'RK Loader' : 'FES Loader';
  const path = selection.paths[systemType];
  const setPath = (value: string) =>
    setSelection((current) => ({
      ...current,
      paths: { ...current.paths, [current.systemType]: value },
    }));
  const setSystemType = (value: 'vendor' | 'mainline') =>
    setSelection((current) => ({ ...current, systemType: value }));
  const [loaderChecking, setLoaderChecking] = useState(false);
  const [loaderRevision, setLoaderRevision] = useState(0);
  const [loaderError, setLoaderError] = useState('');
  const [loaderReadable, setLoaderReadable] = useState(false);
  const [memoryError, setMemoryError] = useState('');
  const [tab, setTab] = useState<'offline' | 'ai' | 'apps'>('offline');
  useEffect(() => {
    if (applicationCandidateId) setTab('apps');
  }, [applicationCandidateId]);
  const [fileRevision, setFileRevision] = useState(0);
  const [fileInfo, setFileInfo] = useState<{ bytes: number; entries?: number } | null>(null);
  const [checking, setChecking] = useState(false);
  const [fileError, setFileError] = useState('');
  const [message, setMessage] = useState('');
  const [starting, setStarting] = useState(false);
  const [submittedTask, setSubmittedTask] = useState('');
  const [mode, setMode] = useState<'full_erase' | 'partition' | 'partition_erase'>('full_erase');
  const [verify, setVerify] = useState(true);
  const [postAction, setPostAction] = useState<'reboot' | 'none' | 'poweroff'>('reboot');
  const [autoScroll, setAutoScroll] = useState(true);
  const [errorsOnly, setErrorsOnly] = useState(false);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const logRef = useRef<HTMLPreElement>(null);
  const tasksRef = useRef<HTMLDivElement>(null);
  const uploadRef = useRef<HTMLButtonElement>(null);
  const recentTasks = tasks
    .filter((task) => task.deviceId === board.id && /flash|mainline_boot/i.test(task.kind))
    .sort((a, b) => b.startedAt - a.startedAt);
  const latest = recentTasks[0];
  const running =
    tasks.some((task) => task.deviceId === board.id && task.status === 'running') ||
    runtime.flashTaskStatus === 'running';
  const awaitingTask = !!submittedTask && !tasks.some((task) => task.taskId === submittedTask);
  const busy = starting || running || awaitingTask;
  const configurable =
    (systemType === 'vendor' && ['openixcli', 'openrockcli'].includes(board.flasherKind)) || rkRaw;
  const requiresLoader = systemType === 'mainline' && (board.chipVendor === 'allwinner' || rkRaw);
  const needsLoader =
    requiresLoader &&
    ((!rkRaw && !mainlineRoute) || !loaderReadable || loaderChecking || !!loaderError);
  const name = path.split(/[/\\]/).pop() || '';
  const canStart =
    !busy &&
    detected &&
    !!board.usbBinding &&
    !!fileInfo &&
    !checking &&
    !fileError &&
    !needsLoader;
  const showingHistory = !busy && !path && !!latest;
  const showingTask =
    busy || showingHistory || (!!submittedTask && latest?.taskId === submittedTask);
  const spinorTask = showingTask && runtime.flashStorageType === 'spinor';
  const taskSucceeded =
    showingTask &&
    runtime.flashTaskStatus === 'success' &&
    (!spinorTask ||
      (runtime.flashPhysicalVerify === 'success' && runtime.flashBootVerify === 'success'));
  const displayLines = runtime.flashLines.flatMap((line) => {
    const raw = line.replace(/^\[(?:STDOUT|STDERR)\]\s*/, '');
    return raw.trimStart().startsWith('{') ? restoreFlashOutputTail([raw]) : [line];
  });
  const visibleLines = errorsOnly
    ? displayLines.filter((line) => /error|fail|错误|失败/i.test(line))
    : displayLines;

  useEffect(() => {
    if (board.id == null) return;
    try {
      window.localStorage.setItem(firmwareSelectionKey(board.id), JSON.stringify(selection));
      setMemoryError('');
    } catch {
      setMemoryError('无法记住文件路径，请检查本地存储空间。');
    }
  }, [board.id, selection]);

  useEffect(() => {
    let cancelled = false;
    setLoaderReadable(false);
    setLoaderError('');
    if (!requiresLoader || !loaderPath) {
      setLoaderChecking(false);
      return;
    }
    setLoaderChecking(true);
    void stat(loaderPath)
      .then((info) => {
        if (!info.isFile || !info.size)
          throw new Error(`${loaderLabel} 文件不存在或为空，请重新选择。`);
        if (!cancelled) setLoaderReadable(true);
      })
      .catch((error) => {
        if (!cancelled) setLoaderError(`${loaderLabel} 无法读取：${formatErrorMessage(error)}`);
      })
      .finally(() => {
        if (!cancelled) setLoaderChecking(false);
      });
    return () => {
      cancelled = true;
    };
  }, [requiresLoader, loaderPath, loaderRevision, loaderLabel]);

  async function chooseLoader() {
    try {
      const selected = await open({
        title: `选择 ${loaderLabel} 固件`,
        multiple: false,
        directory: false,
        filters: [{ name: loaderLabel, extensions: rkRaw ? ['bin'] : ['img', 'bin', 'fex'] }],
      });
      if (typeof selected !== 'string') return;
      setLoaderReadable(false);
      setSelection((current) => ({
        ...current,
        ...(rkRaw ? { rockchipLoaderPath: selected } : { loaderPath: selected }),
      }));
      setLoaderRevision((revision) => revision + 1);
    } catch (error) {
      setMessage(`选择 FES 失败：${formatErrorMessage(error)}`);
    }
  }

  useEffect(() => {
    let cancelled = false;
    setFileInfo(null);
    setFileError('');
    if (!path) {
      setChecking(false);
      return;
    }
    setChecking(true);
    void (async () => {
      if (rkRaw && !/\.img(?:\.(?:xz|gz))?$/i.test(path))
        throw new Error('RK raw 请选择完整 .img、.img.xz 或 .img.gz 镜像。');
      if (!rkRaw && /\.(zip|xz|gz|7z)$/i.test(path))
        throw new Error('请先解压并确认压缩包完整，再选择实际镜像文件。');
      const info = await stat(path);
      if (!info.isFile || info.size === 0) throw new Error('请选择非空固件文件。');
      let entries: number | undefined;
      if (systemType === 'vendor' && board.chipVendor === 'allwinner') {
        const parsed = await invokeCommand('firmware_parse_image', { filePath: path });
        if (parsed.last_error || !parsed.image_info || !parsed.file_headers.length)
          throw new Error(parsed.last_error || '不是可识别的 Allwinner IMAGEWTY 固件。');
        if (parsed.is_encrypted) throw new Error('当前离线入口不接受加密 IMAGEWTY 固件。');
        entries = parsed.file_headers.length;
      }
      if (!cancelled) setFileInfo({ bytes: info.size, entries });
    })()
      .catch((error) => {
        if (!cancelled) setFileError(formatErrorMessage(error));
      })
      .finally(() => {
        if (!cancelled) setChecking(false);
      });
    return () => {
      cancelled = true;
    };
  }, [path, fileRevision, systemType, board.chipVendor, rkRaw]);

  useEffect(() => {
    if (tab !== 'offline' || busy) return;
    let disposed = false;
    let unsubscribe: (() => void) | undefined;
    void getCurrentWebview()
      .onDragDropEvent(({ payload }) => {
        if (payload.type !== 'drop' || disposed) return;
        const bounds = uploadRef.current?.getBoundingClientRect();
        const x = payload.position.x / window.devicePixelRatio;
        const y = payload.position.y / window.devicePixelRatio;
        if (!bounds || x < bounds.left || x > bounds.right || y < bounds.top || y > bounds.bottom)
          return;
        if (payload.paths.length !== 1) {
          setMessage('每次请选择一个固件文件。');
          return;
        }
        setFileInfo(null);
        setPath(payload.paths[0]);
        setFileRevision((revision) => revision + 1);
        setMessage('');
      })
      .then((unlisten) => {
        if (disposed) unlisten();
        else unsubscribe = unlisten;
      })
      .catch((error) => {
        if (!disposed) setMessage(`拖放不可用，请点击选择文件：${formatErrorMessage(error)}`);
      });
    return () => {
      disposed = true;
      unsubscribe?.();
    };
  }, [tab, busy]);

  useEffect(() => {
    if (autoScroll && logRef.current) logRef.current.scrollTop = logRef.current.scrollHeight;
  }, [runtime.flashLines, autoScroll]);

  async function chooseFile() {
    try {
      const selected = await open({
        title: '选择本地固件',
        multiple: false,
        directory: false,
        filters: [
          {
            name: '固件镜像',
            extensions: rkRaw
              ? ['img', 'xz', 'gz']
              : ['img', 'bin', 'wic', 'iso', 'fex', 'dfu', 'uuu'],
          },
          { name: '所有文件', extensions: ['*'] },
        ],
      });
      if (typeof selected === 'string') {
        setFileInfo(null);
        setPath(selected);
        setFileRevision((revision) => revision + 1);
        setMessage('');
      }
    } catch (error) {
      setMessage(formatErrorMessage(error));
    }
  }

  async function startFlash() {
    if (!canStart || !board.id) return;
    setStarting(true);
    try {
      const approved = await ask(
        `确认将“${name}”烧录到“${board.name}”？\n系统类型：${systemType === 'vendor' ? '厂商系统镜像' : '主线 Linux'}${requiresLoader ? `\n${loaderLabel}：${loaderPath}\n存储路线：${rkRaw ? 'eMMC 用户区（完整擦除）' : spinor ? 'SPI NOR（完整镜像）' : mainlineRoute === 'mainline_emmc' ? 'eMMC' : 'NAND'}` : ''}\nUSB：${board.usbBinding}\n烧录将覆盖目标数据。`,
        { title: '确认固件烧录', kind: 'warning' }
      );
      if (!approved) return;
      const taskId = await invokeCommand('workbench_start_flash', {
        deviceId: board.id,
        firmwarePath: path,
        options: rkRaw
          ? {
              systemType,
              loaderPath,
              mainlineRoute: 'rockchip_raw_emmc',
              mode: 'full_erase',
              verify: true,
              postFlashAction: postAction,
            }
          : configurable
            ? { systemType, mode, verify, postFlashAction: postAction }
            : requiresLoader && mainlineRoute
              ? {
                  systemType,
                  loaderPath,
                  mainlineRoute,
                  ...(spinor ? { postFlashAction: postAction } : {}),
                }
              : { systemType },
      });
      setSubmittedTask(taskId);
      onTaskStarted(taskId);
      setMessage('烧录任务已提交。');
    } catch (error) {
      setMessage(`烧录未启动：${formatErrorMessage(error)}`);
    } finally {
      setStarting(false);
    }
  }

  async function exportLog() {
    try {
      const target = await save({
        defaultPath: `lynx-flash-${board.id}-${Date.now()}.log`,
        filters: [{ name: '烧录日志', extensions: ['log'] }],
      });
      if (target) {
        await writeTextFile(target, runtime.flashLines.join('\n'));
        setMessage('日志已导出。');
      }
    } catch (error) {
      setMessage(`导出失败：${formatErrorMessage(error)}`);
    }
  }

  return (
    <div className="firmware-page">
      <div className="firmware-title">
        <div>
          <h2>固件烧录</h2>
          <p>使用当前单板已绑定的设备与端口</p>
        </div>
        <button
          className="firmware-button"
          onClick={() => {
            setTab('offline');
            requestAnimationFrame(() =>
              tasksRef.current?.scrollIntoView({ behavior: 'smooth', block: 'nearest' })
            );
          }}
        >
          <LynxIcon name="history" size={16} />
          烧录记录
        </button>
      </div>
      <div className={`firmware-layout ${tab === 'apps' ? 'is-applications' : ''}`}>
        <section className="firmware-setup">
          <div className="firmware-tabs" role="tablist" aria-label="烧录方式">
            <button
              className="firmware-tab"
              id="firmware-offline-tab"
              role="tab"
              aria-controls="firmware-offline"
              aria-selected={tab === 'offline'}
              onClick={() => setTab('offline')}
            >
              本地离线烧录
            </button>
            <button
              className="firmware-tab"
              id="firmware-ai-tab"
              role="tab"
              aria-controls="firmware-ai"
              aria-selected={tab === 'ai'}
              onClick={() => setTab('ai')}
            >
              AI MCP 自动烧录
            </button>
            <button
              className="firmware-tab"
              id="firmware-apps-tab"
              role="tab"
              aria-controls="firmware-apps"
              aria-selected={tab === 'apps'}
              onClick={() => setTab('apps')}
            >
              应用固件
            </button>
          </div>
          {tab === 'apps' ? (
            <div role="tabpanel" id="firmware-apps" aria-labelledby="firmware-apps-tab">
              <FirmwareApplicationsPanel
                key={`${board.id}:${board.boardUid}`}
                board={board}
                candidateId={applicationCandidateId}
                onTaskStarted={onTaskStarted}
              />
            </div>
          ) : tab === 'offline' ? (
            <div role="tabpanel" id="firmware-offline" aria-labelledby="firmware-offline-tab">
              <p className="firmware-caption">本地文件即可使用，无需配置远端 SDK</p>
              <div className="firmware-form-columns">
                <section className="firmware-card">
                  <h3>
                    <b>01</b>选择固件
                  </h3>
                  <button
                    ref={uploadRef}
                    className="firmware-drop"
                    disabled={busy}
                    onClick={() => void chooseFile()}
                  >
                    <LynxIcon name="document" size={24} />
                    <span>
                      <strong>拖入固件文件，或点击选择</strong>
                      <small>
                        {rkRaw
                          ? '支持完整 .img、.img.xz、.img.gz 镜像'
                          : '压缩包请先解压，选择实际固件镜像'}
                      </small>
                    </span>
                    <span className="firmware-file-action">
                      <LynxIcon name="upload" size={16} />
                      选择文件
                    </span>
                  </button>
                  <div className="firmware-file" title={path}>
                    <LynxIcon name="document" size={24} />
                    <div>
                      <strong>{name || '尚未选择固件'}</strong>
                      <small>
                        {checking
                          ? '正在读取固件…'
                          : fileInfo
                            ? `${(fileInfo.bytes / 1024 / 1024).toFixed(1)} MB${fileInfo.entries != null ? ' · IMAGEWTY' : ''}`
                            : '等待选择本地文件'}
                      </small>
                    </div>
                    {fileInfo && (
                      <span className="firmware-file-badge">
                        <LynxIcon name="check-circle" size={16} />
                        已读取
                      </span>
                    )}
                  </div>
                  {fileError && (
                    <p className="firmware-error" role="alert">
                      {fileError}
                    </p>
                  )}
                  {fileInfo && (
                    <>
                      <div className="firmware-file-meta">
                        <span>
                          当前单板 <strong>{board.name}</strong>
                        </span>
                        <span>
                          {fileInfo.entries != null ? `${fileInfo.entries} 个组件` : '本地镜像'}
                        </span>
                        <button
                          className="firmware-link"
                          onClick={() => setDetailsOpen(!detailsOpen)}
                        >
                          {detailsOpen ? '收起信息' : '固件信息 ›'}
                        </button>
                      </div>
                      {detailsOpen && (
                        <dl className="firmware-details">
                          <dt>本地路径</dt>
                          <dd>{path}</dd>
                          <dt>文件大小</dt>
                          <dd>{fileInfo.bytes.toLocaleString()} 字节</dd>
                          <dt>检查范围</dt>
                          <dd>已读取文件与格式信息；未验证来源哈希，写入校验以任务结果为准。</dd>
                          <dt>格式</dt>
                          <dd>{fileInfo.entries != null ? 'IMAGEWTY' : '由当前烧录器检查'}</dd>
                        </dl>
                      )}
                    </>
                  )}
                  {requiresLoader && (
                    <div className="firmware-loader">
                      <div className="firmware-loader-heading">
                        <strong>{loaderLabel} 固件</strong>
                        {rkRaw ? (
                          <span>目标存储：eMMC</span>
                        ) : (
                          <label>
                            <select
                              aria-label="主线存储路线"
                              value={mainlineRoute}
                              disabled={busy}
                              onChange={(event) =>
                                setSelection((current) => ({
                                  ...current,
                                  mainlineRoute: event.target.value as typeof mainlineRoute,
                                }))
                              }
                            >
                              <option value="">选择存储路线</option>
                              <option value="mainline_nand">NAND 组件</option>
                              <option value="mainline_emmc">eMMC（含 Boot0）</option>
                              <option value="mainline_spinor">SPI NOR（完整镜像）</option>
                            </select>
                          </label>
                        )}
                      </div>
                      <div className="firmware-loader-file">
                        <LynxIcon name="document" size={20} />
                        <span title={loaderPath}>{loaderPath || `尚未选择 ${loaderLabel}`}</span>
                        <button
                          className="firmware-button"
                          disabled={busy}
                          onClick={() => void chooseLoader()}
                        >
                          {rkRaw ? '选择 RK Loader' : '选择 FES'}
                        </button>
                      </div>
                      <p className="firmware-caption">
                        {rkRaw
                          ? '选择与板卡匹配的 RAM Loader；镜像须自带启动组件。已处于 Loader 模式时沿用当前 Loader。支持 img / img.xz / img.gz，准备阶段需要临时磁盘空间。'
                          : '选择与当前单板匹配的 RAM FES Loader 固件；路径自动保存。'}
                      </p>
                      {spinor && (
                        <p className="firmware-caption">
                          使用完整镜像和独立 RAM FES Loader。烧录前检查 Loader
                          与物理地址映射，写入后进行独立物理回读校验。F101 的 nekobox-full.img
                          包含物理地址 0 的启动程序和 0x080000 的 NekoBox；不使用 eMMC Boot0 路线。
                        </p>
                      )}
                      {loaderChecking && (
                        <p className="firmware-caption">正在读取 {loaderLabel}…</p>
                      )}
                      {loaderError && (
                        <p className="firmware-error" role="alert">
                          {loaderError}
                        </p>
                      )}
                    </div>
                  )}
                  {memoryError && (
                    <p className="firmware-error" role="alert">
                      {memoryError}
                    </p>
                  )}
                </section>
                <section className="firmware-card firmware-options-card">
                  <h3>
                    <b>02</b>系统类型选择
                  </h3>
                  <label className="firmware-system-select">
                    <span>系统类型</span>
                    <select
                      value={systemType}
                      disabled={busy}
                      onChange={(event) => {
                        setFileInfo(null);
                        setSystemType(event.target.value as 'vendor' | 'mainline');
                      }}
                    >
                      <option value="vendor">厂商系统镜像</option>
                      <option value="mainline">主线 Linux</option>
                    </select>
                  </label>
                  <div className="firmware-bindings">
                    <div className="firmware-binding">
                      <small>USB 设备 · {protocol}</small>
                      <strong title={board.usbBinding}>{board.usbBinding || '未绑定 USB'}</strong>
                      <span className={detected ? 'is-connected' : ''}>
                        {detected
                          ? '已绑定 · 已检测到'
                          : board.usbBinding
                            ? '已绑定 · 未检测到'
                            : '请先在设备设置中绑定'}
                      </span>
                    </div>
                    <div className="firmware-binding">
                      <small>启动日志串口</small>
                      <strong title={board.serialPort}>
                        {board.serialPort?.replace(/^\/dev\//, '') || '未绑定'}
                      </strong>
                      <span>
                        {board.serialPort
                          ? `${board.baudRate} · 使用已保存节点`
                          : '可在设备设置中配置'}
                      </span>
                    </div>
                  </div>
                  {spinor && (
                    <label className="firmware-spinor-action">
                      <span>写入及校验通过后</span>
                      <select
                        value={postAction}
                        disabled={busy}
                        onChange={(event) => setPostAction(event.target.value as typeof postAction)}
                      >
                        <option value="reboot">重启设备</option>
                        <option value="none">保持当前状态</option>
                        <option value="poweroff">关闭设备</option>
                      </select>
                    </label>
                  )}
                  {configurable ? (
                    <>
                      <div className="firmware-option-row">
                        <label>
                          烧录方式
                          <select
                            value={mode}
                            disabled={busy}
                            onChange={(event) => setMode(event.target.value as typeof mode)}
                          >
                            <option value="full_erase">完整烧录</option>
                            {board.flasherKind === 'openixcli' && (
                              <>
                                <option value="partition">按分区写入</option>
                                <option value="partition_erase">擦除分区后写入</option>
                              </>
                            )}
                          </select>
                        </label>
                        <label>
                          完成后
                          <select
                            data-flash-options="vendor-completion"
                            value={postAction}
                            disabled={busy}
                            onChange={(event) =>
                              setPostAction(event.target.value as typeof postAction)
                            }
                          >
                            <option value="reboot">重启设备</option>
                            {(board.flasherKind === 'openixcli' ||
                              (board.flasherKind === 'openrockcli' &&
                                navigator.userAgent.toLowerCase().includes('windows'))) && (
                              <option value="none">保持当前状态</option>
                            )}
                            <option value="poweroff">关闭设备</option>
                          </select>
                        </label>
                      </div>
                      <label className="firmware-checkbox">
                        <input
                          type="checkbox"
                          checked={rkRaw || verify}
                          disabled={busy || rkRaw}
                          onChange={(event) => setVerify(event.target.checked)}
                        />
                        写入后校验
                      </label>
                    </>
                  ) : (
                    <p className="firmware-caption">
                      {systemType === 'mainline'
                        ? board.chipVendor === 'allwinner'
                          ? spinor
                            ? '写入与校验成功后执行所选动作；动作结果与启动验收分别显示。'
                            : '使用上方选择的 FES Loader，沿用现有主线烧录流程。'
                          : '沿用当前单板的专用烧录配置。'
                        : '沿用当前单板的专用烧录配置。'}
                    </p>
                  )}
                  {needsLoader && (
                    <p className="firmware-error" role="alert">
                      {rkRaw
                        ? '请选择可读取的 RK Loader。'
                        : '请选择可读取的 FES Loader 和存储路线。'}
                    </p>
                  )}
                  <div className="firmware-options-footer">
                    <button className="firmware-link" disabled={busy} onClick={onOpenSettings}>
                      设备设置
                    </button>
                    <p className="firmware-caption">烧录将覆盖目标存储数据</p>
                  </div>
                </section>
              </div>
            </div>
          ) : (
            <div role="tabpanel" id="firmware-ai" aria-labelledby="firmware-ai-tab">
              <p className="firmware-caption">
                通过当前单板已有的 AI MCP 通道执行；审批、设备绑定与任务日志沿用现有流程。
              </p>
              {aiChannel}
            </div>
          )}
        </section>
        {tab !== 'apps' && (
          <aside className="firmware-sidebar">
            <section className="firmware-card">
              <h3>
                {showingHistory ? '上次烧录' : '本次烧录'}
                <span className={`firmware-status ${taskSucceeded || canStart ? 'is-ready' : ''}`}>
                  {busy
                    ? '进行中'
                    : showingHistory
                      ? latest.status === 'success'
                        ? spinorTask
                          ? flashOutcomeLabel(
                              latest.status,
                              'spinor',
                              runtime.flashBootVerify,
                              runtime.flashPhysicalVerify
                            )
                          : '已完成'
                        : latest.status === 'failed'
                          ? '失败'
                          : latest.status
                      : canStart
                        ? '准备就绪'
                        : '等待操作'}
                </span>
              </h3>
              <p className="firmware-caption">
                {board.name} · {protocol}
              </p>
              <ol className="firmware-steps">
                <li>
                  <b>1</b>
                  <span>检查固件</span>
                  <small>
                    {showingHistory
                      ? '已提交'
                      : tab === 'ai'
                        ? '由 MCP 流程执行'
                        : checking
                          ? '读取中'
                          : fileError
                            ? '读取失败'
                            : fileInfo
                              ? '文件可读取'
                              : '等待选择'}
                  </small>
                </li>
                <li>
                  <b>2</b>
                  <span>确认设备</span>
                  <small>{showingHistory ? '已提交' : detected ? '已连接' : '未检测到'}</small>
                </li>
                <li>
                  <b>3</b>
                  <span>写入固件</span>
                  <small>
                    {showingTask
                      ? spinorTask
                        ? formatMediaWriteState(
                            runtime.flashCommittedBytes,
                            runtime.flashTotalBytes,
                            runtime.flashTaskStatus
                          )
                        : formatFlashPhase(runtime.flashPhase, runtime.flashTaskStatus)
                      : '等待开始'}
                  </small>
                </li>
                <li>
                  <b>4</b>
                  <span>{spinorTask ? 'FES 协议校验' : '校验数据'}</span>
                  <small>
                    {showingTask
                      ? formatVerifyState(runtime.flashVerify, runtime.flashTaskStatus)
                      : '未开始'}
                  </small>
                </li>
                <li>
                  <b>5</b>
                  <span>烧后动作</span>
                  <small>
                    {showingTask
                      ? formatPostFlashState(
                          runtime.flashPostActionKind,
                          runtime.flashPostAction,
                          runtime.flashTaskStatus
                        )
                      : '未开始'}
                  </small>
                </li>
                {spinorTask && (
                  <>
                    <li>
                      <b>6</b>
                      <span>物理位置回读</span>
                      <small>{formatIndependentVerification(runtime.flashPhysicalVerify)}</small>
                    </li>
                    <li>
                      <b>7</b>
                      <span>启动验证</span>
                      <small>{formatIndependentVerification(runtime.flashBootVerify)}</small>
                    </li>
                  </>
                )}
              </ol>
              {spinorTask && (
                <p className="firmware-caption">
                  FES 校验不能证明物理布局正确。启动验收须看到 SyterKit → Zephyr → NekoBox。
                </p>
              )}
              {showingTask && (
                <div className="firmware-progress">
                  <progress
                    max={100}
                    value={Math.min(100, Math.max(0, runtime.flashProgress))}
                    aria-label="烧录进度"
                  />
                  <span>{runtime.flashProgress.toFixed(1)}%</span>
                </div>
              )}
              <p className="firmware-caption firmware-transfer">
                {showingTask ? (
                  <>
                    {runtime.flashPartition || '未进入分区'} ·{' '}
                    {spinorTask && !runtime.flashRate
                      ? '未记录速率'
                      : formatFlashRate(runtime.flashRate, runtime.flashTaskStatus)}
                  </>
                ) : (
                  <>
                    待写入{' '}
                    <strong>
                      {fileInfo ? `${(fileInfo.bytes / 1024 / 1024).toFixed(1)} MB` : '—'}
                    </strong>
                  </>
                )}
              </p>
              {tab === 'offline' ? (
                <button
                  className="firmware-start"
                  disabled={!canStart}
                  onClick={() => void startFlash()}
                >
                  {busy ? '任务进行中…' : '开始烧录'}
                </button>
              ) : (
                <p className="firmware-caption">
                  由已连接的 AI 客户端发起 MCP 烧录任务，此处同步展示执行结果。
                </p>
              )}
              <p className="firmware-caption firmware-footnote">请保持供电与 USB 连接</p>
              {latest?.status === 'failed' &&
                detected &&
                /device not found|failed to open device/i.test(latest.error ?? '') && (
                  <div className="firmware-error" role="alert">
                    系统已检测到 USB，但烧录器未能打开设备。请检查是否被其他烧录软件占用；Windows 下
                    Allwinner FEL/FES 还需绑定 WinUSB 驱动。处理后由你手动重新开始。
                  </div>
                )}
            </section>
            <div className="firmware-card" ref={tasksRef}>
              <h3>最近记录</h3>
              {recentTasks.length ? (
                recentTasks.slice(0, 2).map((task) => (
                  <div
                    className="firmware-history"
                    key={task.taskId}
                    title={task.error || undefined}
                  >
                    <strong>
                      {flashOutcomeLabel(
                        task.status,
                        task.details?.storageType,
                        task.details?.bootVerifyState,
                        task.details?.physicalVerifyState
                      )}
                    </strong>
                    <small>{new Date(task.startedAt * 1000).toLocaleString()}</small>
                  </div>
                ))
              ) : (
                <p className="firmware-caption">当前单板暂无烧录记录</p>
              )}
            </div>
          </aside>
        )}
      </div>
      {message && (
        <output className="firmware-message" role="status">
          {message}
        </output>
      )}
      {tab !== 'apps' && (
        <section className="firmware-card firmware-log">
          <div className="firmware-log-toolbar">
            <h3>运行日志</h3>
            <button
              className="firmware-filter"
              aria-pressed={!errorsOnly}
              onClick={() => setErrorsOnly(false)}
            >
              全部
            </button>
            <button
              className="firmware-filter"
              aria-pressed={errorsOnly}
              onClick={() => setErrorsOnly(true)}
            >
              错误
            </button>
            <label className="firmware-checkbox">
              <input
                type="checkbox"
                checked={autoScroll}
                onChange={(event) => setAutoScroll(event.target.checked)}
              />
              自动滚动
            </label>
            <button
              className="firmware-button"
              disabled={!runtime.flashLines.length}
              onClick={() => void exportLog()}
            >
              导出日志
            </button>
          </div>
          <pre ref={logRef} aria-label="烧录运行日志">
            {visibleLines.length
              ? visibleLines.join('\n')
              : errorsOnly
                ? '暂无错误日志'
                : '等待烧录任务输出…'}
          </pre>
        </section>
      )}
    </div>
  );
}
