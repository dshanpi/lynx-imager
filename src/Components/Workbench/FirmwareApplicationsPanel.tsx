import FirmwareLifecyclePanel, { firmwareError } from './FirmwareLifecyclePanel';
import { useEffect, useRef, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { openUrl } from '@tauri-apps/plugin-opener';
import { LynxIcon } from '../../CoreUI';
import { invokeCommand } from '../../Platform/IPC';
import type {
  FirmwareApplication,
  FirmwareApplicationCatalog,
  FirmwareCandidateInspection,
  FirmwareReleaseDiscovery,
  FirmwareRemoteCatalog,
  WorkbenchDevice,
} from '../../Platform/IPC/contracts/Workbench';
import { formatLinuxBytes, formatUsbBinding } from './linuxWorkbenchModel';
import {
  candidateErrorMessage,
  candidateMatchesScope,
  filterFirmwareApplications,
  firmwareRouteLabel,
  releaseDiscoveryMessage,
  releaseErrorMessage,
} from './firmwareApplications';
import './FirmwareApplicationsPanel.css';
import { FirmwareAppIcon, NekoBoxIllustration } from './FirmwareVisuals';

export default function FirmwareApplicationsPanel({
  board,
  candidateId,
  onTaskStarted,
}: {
  board: WorkbenchDevice;
  candidateId?: string;
  onTaskStarted?: (id: string) => void;
}) {
  const [workflow, setWorkflow] = useState(candidateId || '');
  const [showWorkflow, setShowWorkflow] = useState(!!candidateId);
  const [manifestPath, setManifestPath] = useState('');
  const [channel, setChannel] = useState<'stable' | 'test'>('stable');
  const [remoteCatalog, setRemoteCatalog] = useState<FirmwareRemoteCatalog | null>(null);
  const [remoteVersion, setRemoteVersion] = useState('');
  const [downloading, setDownloading] = useState(false);
  useEffect(() => {
    if (candidateId) {
      setWorkflow(candidateId);
      setShowWorkflow(true);
    }
  }, [candidateId]);
  const [catalog, setCatalog] = useState<FirmwareApplicationCatalog | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [refresh, setRefresh] = useState(0);
  const [query, setQuery] = useState('');
  const [boardFilter, setBoardFilter] = useState('');
  const [systemFilter, setSystemFilter] = useState('zephyr');
  const [appId, setAppId] = useState('');
  const [candidate, setCandidate] = useState<FirmwareCandidateInspection | null>(null);
  const [candidateError, setCandidateError] = useState('');
  const [inspecting, setInspecting] = useState(false);
  const [notice, setNotice] = useState('');
  const attempt = useRef(0);
  const releaseAttempt = useRef(0);
  const [releases, setReleases] = useState<FirmwareReleaseDiscovery | null>(null);
  const [readingReleases, setReadingReleases] = useState(false);
  const [releaseError, setReleaseError] = useState('');

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setCatalog(null);
    setError('');
    if (board.id == null) {
      setLoading(false);
      setError('请先保存当前单板，再查看应用固件。');
      return;
    }
    void invokeCommand('workbench_firmware_applications', { deviceId: board.id })
      .then((value) => {
        if (cancelled) return;
        if (value.deviceId !== board.id || value.boardUid !== board.boardUid) {
          throw new Error('scope changed');
        }
        setCatalog(value);
      })
      .catch(() => {
        if (!cancelled) setError('应用目录读取失败，请重试。不会连接或操作设备。');
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [board.id, board.boardUid, refresh]);

  useEffect(() => {
    return () => {
      attempt.current += 1;
      releaseAttempt.current += 1;
    };
  }, []);

  const selected = catalog?.applications.find((app) => app.id === appId);
  const visible = filterFirmwareApplications(
    catalog?.applications ?? [],
    query,
    boardFilter,
    systemFilter
  );

  function selectApplication(id: string) {
    attempt.current += 1;
    setAppId(id);
    setCandidate(null);
    setCandidateError('');
    setInspecting(false);
    setNotice('');
  }

  async function showDocumentation(app: FirmwareApplication) {
    // Only the bundled, reviewed documentation domain is allowed here.
    if (!app.documentationUrl.startsWith('https://zephyr.100ask.net/docs/yuzukineko/')) {
      setNotice('文档地址未通过检查。');
      return;
    }
    try {
      await openUrl(app.documentationUrl);
    } catch {
      setNotice('无法打开文档，请检查浏览器设置。');
    }
  }

  async function refreshReleases() {
    if (board.id == null || readingReleases) return;
    const currentAttempt = ++releaseAttempt.current;
    setReleases(null);
    setRemoteCatalog(null);
    setRemoteVersion('');
    setReleaseError('');
    setReadingReleases(true);
    try {
      const result = await invokeCommand('workbench_firmware_releases', { deviceId: board.id });
      if (currentAttempt !== releaseAttempt.current) return;
      if (result.deviceId !== board.id || result.boardUid !== board.boardUid)
        throw new Error('FIRMWARE_RELEASE_SCOPE_CHANGED');
      setReleases(result);
      const metadata = await invokeCommand('workbench_firmware_catalog', {
        deviceId: board.id,
        channel,
      });
      if (currentAttempt !== releaseAttempt.current) return;
      if (metadata.boardUid !== board.boardUid) throw new Error('FIRMWARE_RELEASE_SCOPE_CHANGED');
      setRemoteCatalog(metadata);
    } catch (failure) {
      if (currentAttempt === releaseAttempt.current) setReleaseError(releaseErrorMessage(failure));
    } finally {
      if (currentAttempt === releaseAttempt.current) setReadingReleases(false);
    }
  }

  async function inspectLocalCandidate() {
    if (!selected || selected.id !== 'nekobox' || board.id == null || inspecting) return;
    const currentAttempt = ++attempt.current;
    setCandidate(null);
    setCandidateError('');
    setInspecting(true);
    try {
      const path = await open({
        title: '核对 NekoBox 本地候选包清单',
        multiple: false,
        directory: false,
        filters: [{ name: '候选固件清单', extensions: ['json'] }],
      });
      if (typeof path !== 'string' || currentAttempt !== attempt.current) return;
      const result = await invokeCommand('workbench_inspect_firmware_candidate', {
        deviceId: board.id,
        manifestPath: path,
      });
      if (currentAttempt !== attempt.current) return;
      if (!candidateMatchesScope(result, board, selected.id))
        throw new Error('FIRMWARE_CANDIDATE_SCOPE_CHANGED');
      setCandidate(result);
      setManifestPath(path);
    } catch (failure) {
      if (currentAttempt === attempt.current) setCandidateError(candidateErrorMessage(failure));
    } finally {
      if (currentAttempt === attempt.current) setInspecting(false);
    }
  }

  const versions =
    remoteCatalog?.entries.filter((v) => v.entry.manifest.applicationId === appId) || [];
  const version = versions.find((v) => String(v.releaseId) === remoteVersion);
  async function download() {
    if (!version || board.id == null || downloading) return;
    setDownloading(true);
    setNotice('');
    try {
      const loader = await open({ title: '选择本地 F101 RAM FES Loader', multiple: false });
      if (typeof loader !== 'string') return;
      const c = await invokeCommand('workbench_firmware_download', {
        deviceId: board.id,
        request: {
          releaseId: version.releaseId,
          applicationId: appId,
          version: version.entry.manifest.version,
          channel,
          loaderPath: loader,
        },
      });
      setWorkflow(c.id);
      setShowWorkflow(true);
    } catch (e) {
      setNotice(firmwareError(e));
    } finally {
      setDownloading(false);
    }
  }
  if (showWorkflow)
    return (
      <FirmwareLifecyclePanel
        key={board.boardUid}
        board={board}
        candidateId={workflow || undefined}
        onBack={() => {
          setShowWorkflow(false);
          setWorkflow('');
        }}
        onTaskStarted={onTaskStarted}
      />
    );
  if (loading)
    return (
      <div className="firmware-app-state" role="status">
        正在读取应用目录…
      </div>
    );
  if (error)
    return (
      <div className="firmware-app-state" role="alert">
        <p>{error}</p>
        <button onClick={() => setRefresh((n) => n + 1)}>重新读取目录</button>
      </div>
    );

  return (
    <div className="firmware-applications" data-testid="firmware-applications">
      {selected ? (
        <>
          <nav className="firmware-app-breadcrumb" aria-label="应用位置">
            <button onClick={() => selectApplication('')}>应用固件</button>
            <LynxIcon name="chevron-right" size={16} />
            <strong>{selected.name}</strong>
          </nav>
          <div className="firmware-app-detail-layout">
            <section className="firmware-app-detail-card">
              <div className="firmware-app-introduction">
                <div
                  className={`firmware-app-art app-${selected.id}`}
                  aria-label={`${selected.name} 应用图标，非设备实拍`}
                >
                  {selected.id === 'nekobox' ? (
                    <NekoBoxIllustration />
                  ) : (
                    <>
                      <FirmwareAppIcon id={selected.id} />
                      <small>应用图标 · 非实拍</small>
                    </>
                  )}
                </div>
                <div className="firmware-app-description">
                  <h3>
                    {selected.name} / {selected.description}
                  </h3>
                  <span className="firmware-app-route">{firmwareRouteLabel(selected.routes)}</span>
                  <p>
                    面向 YuzukiNeko / F101 的 Zephyr
                    应用。适配配置与功能范围以具体版本配方及验收记录为准。
                  </p>
                  {selected.id === 'nekobox' && (
                    <p className="firmware-app-features">
                      媒体 · 图片 · 游戏 · USB 副屏
                      <br />
                      <small>功能以所选版本与实际验收为准</small>
                    </p>
                  )}
                  <dl className="firmware-app-specs">
                    <div>
                      <dt>目标板型</dt>
                      <dd>YuzukiNeko · F101</dd>
                    </div>
                    <div>
                      <dt>屏幕</dt>
                      <dd>{version ? '1024 × 768（配方）' : '待选择版本'}</dd>
                    </div>
                    <div>
                      <dt>存储容量</dt>
                      <dd>
                        {candidate
                          ? `${formatLinuxBytes(candidate.manifest.flashCapacityBytes)}（清单声明）`
                          : '待选择版本'}
                      </dd>
                    </div>
                    <div>
                      <dt>触摸</dt>
                      <dd>{version ? 'GT967（配方）' : '待选择版本'}</dd>
                    </div>
                  </dl>
                </div>
              </div>
              <label className="firmware-app-version">
                <span>版本</span>
                <select
                  aria-label="固件版本"
                  value={remoteVersion}
                  onChange={(e) => {
                    setRemoteVersion(e.target.value);
                    setCandidate(null);
                    setManifestPath('');
                  }}
                >
                  <option value="">
                    {candidate ? `${candidate.manifest.version} · 本地候选` : '选择已发布版本'}
                  </option>
                  {versions.map((v) => (
                    <option key={v.releaseId} value={String(v.releaseId)}>
                      {v.entry.manifest.version} · {v.channel === 'stable' ? '正式版' : '测试版'}
                      {v.entry.status === 'withdrawn' ? ' · 已撤回' : ''}
                    </option>
                  ))}
                </select>
              </label>
              <div className="firmware-app-version">
                <span>镜像文件</span>
                <code>
                  {candidate
                    ? candidate.manifest.image.fileName
                    : version?.entry.manifest.image.fileName || '暂无已发布镜像'}
                </code>
              </div>
              {version && (
                <div className="firmware-candidate-results">
                  <p>{version.entry.notes}</p>
                  <p>
                    16 MiB · 1024×768 / GT967 ·{' '}
                    {version.entry.status === 'withdrawn'
                      ? `已撤回：${version.entry.withdrawalReason}`
                      : '当前可用'}
                  </p>
                  <code>{version.entry.manifest.image.sha256}</code>
                  <p>
                    构建：{String(version.entry.validation.build || '未验证')} · 实机：
                    {String(version.entry.validation.hardware || '未验证')}
                  </p>
                </div>
              )}
              {candidate && (
                <div className="firmware-candidate-results" role="status">
                  <p>
                    <LynxIcon name="check-circle" size={20} /> 镜像与 Loader 文件完整性核对通过
                  </p>
                  <dl>
                    <dt>实际镜像大小</dt>
                    <dd>{formatLinuxBytes(candidate.manifest.image.sizeBytes)}</dd>
                    <dt>镜像 SHA-256</dt>
                    <dd>
                      <code>{candidate.manifest.image.sha256}</code>
                    </dd>
                    <dt>Loader</dt>
                    <dd>{candidate.manifest.loader.fileName}</dd>
                    <dt>Loader 来源声明</dt>
                    <dd>{candidate.manifest.loaderSource}</dd>
                    <dt>源码提交</dt>
                    <dd>
                      <code>{candidate.manifest.sourceCommit}</code>
                    </dd>
                  </dl>
                  <p className="firmware-app-muted">
                    文件哈希一致不代表 Loader 适配、物理布局或启动验证通过。
                  </p>
                </div>
              )}
              {candidateError && (
                <p className="firmware-error" role="alert">
                  {candidateError}
                </p>
              )}
              <p className="firmware-app-warning">
                <LynxIcon name="warning" size={20} />
                {selected.routes.includes('xip')
                  ? 'XIP 完整镜像不适用 UF2 拖放路线'
                  : 'RAM / NOR / UF2 自动编排尚未接入，请按对应文档核对条件'}
              </p>
              {selected.id === 'nekobox' && (
                <button
                  className="firmware-candidate-open"
                  disabled={inspecting}
                  onClick={() => void inspectLocalCandidate()}
                >
                  <LynxIcon name="folder" size={16} />
                  {inspecting ? '正在校验文件…' : '核对本地候选包'}
                </button>
              )}
            </section>
            <aside className="firmware-app-install-card">
              <h3>安装到当前单板</h3>
              <p>
                <LynxIcon name="plug" size={20} /> {formatUsbBinding(board.usbBinding)}
              </p>
              <p>
                <LynxIcon name="terminal" size={20} />{' '}
                {board.serialPort ? `${board.serialPort} / ${board.baudRate}` : '串口未绑定'}
              </p>
              <div className="firmware-app-fact">板型与硬件配置：未核对</div>
              <div className="firmware-app-fact">
                {candidate ? '本地候选 · 尚未取得安装资格' : '尚无已核对固件 · GitHub 发布源已配置'}
              </div>
              <button
                className="firmware-app-download"
                disabled={
                  !version ||
                  version.entry.status !== 'active' ||
                  downloading ||
                  !remoteCatalog ||
                  remoteCatalog.expiresAt < Date.now() / 1000
                }
                onClick={() => void download()}
                aria-describedby="firmware-app-install-reason"
              >
                <LynxIcon name="download" size={20} />
                下载并核对
              </button>
              {candidate && manifestPath && (
                <button
                  disabled={inspecting}
                  onClick={async () => {
                    setInspecting(true);
                    try {
                      const c = await invokeCommand('workbench_firmware_import', {
                        deviceId: board.id!,
                        manifestPath,
                      });
                      setWorkflow(c.id);
                      setShowWorkflow(true);
                    } catch (e) {
                      setCandidateError(firmwareError(e));
                    } finally {
                      setInspecting(false);
                    }
                  }}
                >
                  进入烧录确认
                </button>
              )}
              <button onClick={() => void showDocumentation(selected)}>
                <LynxIcon name="book" size={20} />
                查看文档
              </button>
              <p id="firmware-app-install-reason" className="firmware-app-muted">
                {candidate
                  ? '还需核对真实板型、Loader 适配及安装预检。本地文件核对不会发起烧录。'
                  : '没有已发布版本及兼容证据，当前只能查看文档或核对本地候选包。'}
              </p>
              <p className="firmware-app-muted">{selected.documentationScope}</p>
            </aside>
          </div>
        </>
      ) : (
        <>
          <h3>选择应用固件</h3>
          <div className="firmware-app-filters">
            <select
              aria-label="应用目标板型"
              value={boardFilter}
              onChange={(e) => setBoardFilter(e.target.value)}
            >
              <option value="">全部板型 · 当前板型未核对</option>
              <option value="yuzukineko">YuzukiNeko / F101</option>
            </select>
            <select
              aria-label="应用运行系统"
              value={systemFilter}
              onChange={(e) => setSystemFilter(e.target.value)}
            >
              <option value="">全部系统</option>
              <option value="zephyr">Zephyr</option>
            </select>
            <label className="firmware-app-search">
              <LynxIcon name="search" size={20} />
              <input
                aria-label="搜索应用固件"
                placeholder="搜索应用固件（如 NekoBox、LVGL…）"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
              />
            </label>
          </div>
          <div className="firmware-app-grid">
            {visible.map((app) => (
              <article className={`firmware-application-card app-${app.id}`} key={app.id}>
                <div className="firmware-app-card-head">
                  <div className="firmware-app-icon">
                    <FirmwareAppIcon id={app.id} />
                  </div>
                  <div className="firmware-app-card-name">
                    <h4>{app.name}</h4>
                    <p>{app.description}</p>
                    <span className="firmware-app-route">{firmwareRouteLabel(app.routes)}</span>
                  </div>
                  <button
                    onClick={() => selectApplication(app.id)}
                    aria-label={`查看 ${app.name} 版本`}
                  >
                    查看版本
                  </button>
                </div>
                <p className="firmware-app-card-status">
                  {remoteCatalog?.entries.some(
                    (v) => v.entry.manifest.applicationId === app.id && v.entry.status === 'active'
                  )
                    ? '已有发布版本 · 查看详情'
                    : '尚无可用固件 · 可查看文档'}
                </p>
              </article>
            ))}
          </div>
          {!visible.length && (
            <div className="firmware-app-state">
              {catalog?.applications.length
                ? '没有匹配的应用，请调整搜索或筛选。'
                : '应用目录为空，尚无可用固件。'}
            </div>
          )}
        </>
      )}
      <details className="firmware-catalog-options">
        <summary>
          <LynxIcon name="info" size={16} /> 固件版本与兼容信息由发布清单提供{' '}
          <span>发布源与本地候选</span>
        </summary>{' '}
        <section
          className="firmware-release-source"
          data-testid="firmware-release-source"
          aria-label="固件发布源"
        >
          <div>
            <strong>固件源 · GitHub Releases</strong>
            <span>{catalog?.repository}</span>
            <p role={releaseError ? 'alert' : 'status'}>
              {readingReleases
                ? '正在读取稳定版本目录…'
                : releaseError || releaseDiscoveryMessage(releases)}
            </p>
            {releases && (
              <small>
                本次读取：{new Date(releases.checkedAtMs).toLocaleString()} · 仅稳定版本
              </small>
            )}
          </div>
          <button
            data-testid="firmware-release-refresh"
            onClick={() => void refreshReleases()}
            disabled={readingReleases}
          >
            {readingReleases ? '正在读取…' : releaseError ? '重试读取' : '读取发布目录'}
          </button>
        </section>
        <nav className="fw-page-tabs">
          <label>
            发布通道{' '}
            <select
              aria-label="固件发布通道"
              disabled={readingReleases}
              value={channel}
              onChange={(e) => {
                setChannel(e.target.value as 'stable' | 'test');
                setRemoteCatalog(null);
                setRemoteVersion('');
              }}
            >
              <option value="stable">正式版</option>
              <option value="test">测试版</option>
            </select>
          </label>
          <button onClick={() => setShowWorkflow(true)}>本地候选 / 验收记录 / 发布</button>
        </nav>
      </details>
      {notice && <p role="alert">{notice}</p>}
    </div>
  );
}
