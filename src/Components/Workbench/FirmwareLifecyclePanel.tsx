import { useCallback, useEffect, useState } from 'react';
import { open, save, confirm } from '@tauri-apps/plugin-dialog';
import { writeTextFile } from '@tauri-apps/plugin-fs';
import { invokeCommand, normalizeIpcError } from '../../Platform/IPC';
import type {
  FirmwareWorkspace,
  FirmwareHardwareProfile,
  FirmwareAcceptanceItem,
  FirmwareCandidate,
  FirmwarePublishRequest,
  WorkbenchDevice,
} from '../../Platform/IPC/contracts/Workbench';
import { FLASH_PHASE_LABELS } from './flashTelemetryPresentation';
import { formatLinuxBytes } from './linuxWorkbenchModel';
import './FirmwareLifecyclePanel.css';
import { LynxIcon } from '../../CoreUI';
import { NekoBoxIllustration } from './FirmwareVisuals';
import { firmwarePublicationRequestId } from './firmwareApplications';

export const checkLabels: Record<string, string> = {
  boot: '固件启动',
  display: '显示',
  touch: '触摸',
  video: '视频',
  audio: '音频',
  network: '网络',
  power_cycle: '断电上电',
};
export const stateLabel = (value: unknown) =>
  (
    ({
      unknown: '状态待确认',
      success: '通过',
      pass: '通过',
      failed: '失败',
      fail: '失败',
      running: '执行中',
      not_verified: '未验证',
      pending: '待执行',
      unchanged: '未变化',
      published: '已发布',
      withdrawn: '已撤回',
      test: '测试版',
      stable: '正式版',
    }) as Record<string, string>
  )[String(value)] || (value ? String(value) : '未验证');
export function firmwareError(error: unknown) {
  const e = normalizeIpcError(error);
  const code = e.machineCode || e.message;
  const names: Record<string, string> = {
    FIRMWARE_PROFILE_REQUIRED: '请先核对并保存单板硬件信息',
    FIRMWARE_BINDING_CHANGED: '单板绑定已变化，请重新核对',
    FIRMWARE_LOADER_NOT_VERIFIED: 'Loader 不属于已验证的 F101 组件',
    FIRMWARE_CONFIRMATION_EXPIRED_OR_USED: '确认已过期或已使用，请重新预检',
    FIRMWARE_VERSION_WITHDRAWN: '版本已撤回',
    FIRMWARE_HARDWARE_ACCEPTANCE_REQUIRED: '正式发布还缺实机验收',
    FIRMWARE_PHYSICAL_EVIDENCE_REQUIRED: '缺少当前镜像的物理回读与重启证据',
    FIRMWARE_TRUSTED_BUILD_RECORD_REQUIRED: '需要关联本工具生成的构建记录',
    FIRMWARE_GITHUB_CLI_REQUIRED: '请安装 GitHub CLI 并登录有该仓库发布权限的账号',
    FIRMWARE_GITHUB_LOGIN_REQUIRED: '请先通过 GitHub CLI 登录',
    FIRMWARE_PUBLISH_PERMISSION_DENIED: '当前 GitHub 账号没有仓库发布权限',
    FLASH_TARGET_UNBOUND: '请先在设备设置中绑定 USB 烧录接口',
  };
  return names[code] ? `${names[code]}（${code}）` : `${e.machineCode || e.name}：${e.message}`;
}
export function useFirmwareWorkspace(board: WorkbenchDevice) {
  const [workspace, setWorkspace] = useState<FirmwareWorkspace | null>(null);
  const [error, setError] = useState('');
  const refresh = useCallback(async () => {
    if (board.id == null) return;
    const data = await invokeCommand('workbench_firmware_workspace', { deviceId: board.id });
    if (data.boardUid !== board.boardUid) throw new Error('FIRMWARE_SCOPE_CHANGED');
    setWorkspace(data);
    setError('');
  }, [board.id, board.boardUid]);
  useEffect(() => {
    let disposed = false;
    const read = async () => {
      if (board.id == null) return;
      try {
        const data = await invokeCommand('workbench_firmware_workspace', { deviceId: board.id });
        if (!disposed && data.boardUid === board.boardUid) {
          setWorkspace(data);
          setError('');
        }
      } catch (e) {
        if (!disposed) setError(firmwareError(e));
      }
    };
    void read();
    const timer = setInterval(() => void read(), 3000);
    return () => {
      disposed = true;
      clearInterval(timer);
    };
  }, [board.id, board.boardUid]);
  return { workspace, error, refresh };
}
export default function FirmwareLifecyclePanel({
  board,
  candidateId,
  onBack,
  onTaskStarted,
  initialView = 'confirm',
}: {
  board: WorkbenchDevice;
  candidateId?: string;
  onBack?: () => void;
  onTaskStarted?: (id: string) => void;
  initialView?: 'confirm' | 'verify' | 'publish';
}) {
  const { workspace, error: readError, refresh } = useFirmwareWorkspace(board);
  const [selected, setSelected] = useState(candidateId || '');
  const [view, setView] = useState(initialView);
  // Publication belongs to Runtime while retaining the current saved-board workspace.
  const inBoardWorkspace = Boolean(onBack);
  useEffect(() => {
    if (!inBoardWorkspace) return;
    const notify = (publication: boolean) =>
      window.dispatchEvent(
        new CustomEvent('lynx:firmware-navigation', { detail: { publication } })
      );
    notify(view === 'publish');
    return () => {
      notify(false);
    };
  }, [view, inBoardWorkspace]);

  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [postAction, setPostAction] = useState('reboot');
  const [ticket, setTicket] = useState<{ id: string; expiresAt: number } | null>(null);
  const [confirmed, setConfirmed] = useState(false);
  const [taskChoice, setTaskChoice] = useState('');
  const c = selected
    ? workspace?.candidates.find((c) => c.id === selected)
    : workspace?.candidates[0];
  useEffect(() => {
    if (candidateId) setSelected(candidateId);
  }, [candidateId]);
  const taskId =
    taskChoice && c?.taskIds.includes(taskChoice) ? taskChoice : c?.taskIds[c.taskIds.length - 1];
  const task = workspace?.tasks.find((t) => t.taskId === taskId);
  const details = task?.details || {};
  const currentChecks =
    workspace?.acceptance
      .filter((a) => a.candidateId === c?.id && a.taskId === taskId)
      .flatMap((a) => a.items) || [];
  const bootResult = currentChecks.find((i) => i.item === 'boot')?.result || 'not_verified';
  const functionResult = Object.keys(checkLabels).every(
    (key) => currentChecks.find((i) => i.item === key)?.result === 'pass'
  )
    ? 'pass'
    : 'not_verified';
  useEffect(() => {
    setTicket(null);
    setConfirmed(false);
  }, [selected, postAction, board.usbBinding, board.serialPort, workspace?.profile?.recordedAt]);
  async function run(fn: () => Promise<void>) {
    if (busy) return;
    setBusy(true);
    setError('');
    try {
      await fn();
      await refresh();
    } catch (e) {
      setError(firmwareError(e));
    } finally {
      setBusy(false);
    }
  }
  async function importLocal() {
    const p = await open({
      multiple: false,
      filters: [{ name: '固件清单', extensions: ['json'] }],
    });
    if (typeof p !== 'string' || board.id == null) return;
    const c = await invokeCommand('workbench_firmware_import', {
      deviceId: board.id,
      manifestPath: p,
    });
    setSelected(c.id);
    setView('confirm');
  }
  return (
    <div className={`fw-lifecycle fw-view-${view}`} data-testid={`firmware-${view}-page`}>
      <header className="fw-page-title">
        <div>
          <h3>
            {view === 'publish' ? '运行中心 › ' : '固件烧录 › '}
            {view === 'confirm'
              ? '烧录确认'
              : view === 'verify'
                ? '烧录与启动验收'
                : '固件版本发布'}
          </h3>
          <p>
            {view === 'confirm'
              ? '核对固件、设备与覆盖范围，确认无误后开始烧录。'
              : view === 'verify'
                ? '写入、物理回读、重启与功能验收分别记录。'
                : '发布已构建的同一产物，检查版本说明、权限与验收证据。'}
          </p>
        </div>
        {onBack && <button onClick={onBack}>返回应用目录</button>}
      </header>
      <details className="fw-task-options">
        <summary>
          候选包与任务记录 <LynxIcon name="chevron-down" size={16} />
        </summary>
        {view === 'verify' && (
          <>
            {' '}
            <details className="fw-task-identity">
              <summary>任务与阶段</summary>
              <select
                aria-label="烧录任务"
                value={taskId || ''}
                onChange={(e) => setTaskChoice(e.target.value)}
              >
                <option value="">选择历史任务</option>
                {c?.taskIds.map((t) => (
                  <option key={t}>{t}</option>
                ))}
              </select>
              <p>阶段：{FLASH_PHASE_LABELS[task?.phase || ''] || task?.phase || '待执行'}</p>
              {task?.progressPct != null && <progress max={100} value={task.progressPct} />}
            </details>{' '}
            <details className="fw-file-evidence">
              <summary>写入状态与物理回读 SHA-256</summary>
              <p>写入完成：{stateLabel(details.writeState)}</p>
              <code>{String(details.physicalImageSha256 || '未验证')}</code>
            </details>
          </>
        )}
        <nav className="fw-page-tabs">
          {onBack && <button onClick={onBack}>返回应用目录</button>}
          <button onClick={() => setView('confirm')}>烧录确认</button>
          <button onClick={() => setView('verify')}>验收与记录</button>
          <button onClick={() => setView('publish')}>版本发布</button>
          <button disabled={busy} onClick={() => void run(importLocal)}>
            导入本地候选包
          </button>
        </nav>
        <label className="fw-row">
          候选产物
          <select
            aria-label="候选固件"
            value={c?.id || ''}
            onChange={(e) => {
              setSelected(e.target.value);
              setTaskChoice('');
            }}
          >
            <option value="">请选择候选包</option>
            {workspace?.candidates.map((c) => (
              <option key={c.id} value={c.id}>
                {c.manifest.version} · {c.manifest.image.sha256.slice(0, 12)}
              </option>
            ))}
          </select>
        </label>
        <details className="fw-file-evidence">
          <summary>镜像 SHA-256 与 Loader 来源</summary>
          <code>{c?.manifest.image.sha256 || '待核对'}</code>
          <p>{c?.manifest.loaderSource || '未核对'}</p>
        </details>{' '}
        <details className="fw-postaction">
          <summary>
            烧后动作：
            {
              {
                reboot: '校验通过后重启',
                none: '保持当前状态',
                poweroff: '校验通过后关机',
              }[postAction]
            }
          </summary>
          <label className="fw-row">
            烧后动作
            <select value={postAction} onChange={(e) => setPostAction(e.target.value)}>
              <option value="reboot">校验通过后重启</option>
              <option value="none">保持当前状态</option>
              <option value="poweroff">校验通过后关机</option>
            </select>
          </label>
        </details>{' '}
      </details>
      {(error || readError) && (
        <p role="alert" className="firmware-error">
          {error || readError}
        </p>
      )}
      {view === 'confirm' && (
        <>
          {!workspace?.profile && (
            <details className="fw-profile-disclosure">
              <summary>
                <LynxIcon name="warning" size={16} /> 板型与硬件配置尚未核对 · 点击核对
              </summary>
              <ProfileForm board={board} onSaved={refresh} />
            </details>
          )}
          <div className="fw-two-columns">
            <div>
              <section className="fw-card">
                <h4>
                  <span className="fw-section-number">01</span> 固件与设备
                </h4>
                <dl>
                  <dt>固件文件</dt>
                  <dd>
                    <span className="fw-file-name" title={c?.manifest.image.fileName}>
                      {c?.manifest.image.fileName || '尚未选择'}
                    </span>
                    <small>{c && formatLinuxBytes(c.manifest.image.sizeBytes)}</small>
                  </dd>
                  <dt>目标设备</dt>
                  <dd>
                    <span
                      className="fw-file-name"
                      title={`${board.name} · ${workspace?.profile?.revision || '硬件配置尚未核对'}`}
                    >
                      {board.name}
                    </span>
                  </dd>
                  <dt>系统</dt>
                  <dd>Zephyr</dd>
                </dl>
              </section>
              <section className="fw-card">
                <h4>
                  <span className="fw-section-number">02</span> 烧录方式
                </h4>
                <div className="fw-flash-method">
                  <label>
                    烧录链路{' '}
                    <span>
                      FEL → FES <LynxIcon name="chevron-down" size={16} />
                    </span>
                  </label>
                  <label>
                    目标介质{' '}
                    <span>
                      SPI NOR 完整镜像 <LynxIcon name="chevron-down" size={16} />
                    </span>
                  </label>
                </div>
                <div className="fw-loader-row">
                  <strong>FES Loader</strong>
                  <span title={c?.manifest.loader.fileName}>
                    <LynxIcon name="document" size={16} />
                    {c?.manifest.loader.fileName || '候选包中需提供 Loader'}
                  </span>
                  <button disabled={busy} onClick={() => void run(importLocal)}>
                    选择候选包
                  </button>
                </div>
              </section>
              <section className="fw-card">
                <h4>
                  <span className="fw-section-number">03</span> 写入范围
                </h4>
                <div className="fw-layout-map">
                  <span>
                    启动程序
                    <br />
                    0x000000
                  </span>
                  <span>
                    参数区
                    <br />
                    0x020000
                  </span>
                  <span>
                    NekoBox
                    <br />
                    0x080000
                  </span>
                </div>
                <p>
                  布局示意，非比例。Flash 容量 16 MiB；本镜像覆盖{' '}
                  {c ? `0x000000—0x${(c.manifest.image.sizeBytes - 1).toString(16)}` : '待核对'}。
                </p>
                <p className="fw-warning">覆盖镜像范围内数据，需确认备份；不承诺保留配置。</p>
                <label>
                  <input
                    type="checkbox"
                    checked={confirmed}
                    onChange={(e) => setConfirmed(e.target.checked)}
                    disabled={!ticket}
                  />{' '}
                  我已核对目标设备与覆盖范围
                </label>
              </section>
            </div>
            <aside className="fw-card">
              <h4>本次烧录</h4>
              <ol className="fw-steps">
                {['检查固件', '确认设备', '写入固件', '物理回读', '启动验收'].map((x, i) => (
                  <li key={x}>
                    <span>
                      <b className="fw-step-number">{i + 1}</b> {x}
                    </span>
                    <small>{i === 0 && ticket ? '预检通过' : '待执行'}</small>
                  </li>
                ))}
              </ol>
              <div className="fw-target-ports">
                <p>
                  <LynxIcon name="plug" size={16} /> USB{' '}
                  <span title={board.usbBinding}>{board.usbBinding ? '已绑定' : '未绑定'}</span>
                </p>
                <p>
                  <LynxIcon name="terminal" size={16} /> 串口{' '}
                  <span>{board.serialPort || '未绑定'}</span>
                </p>
              </div>
              <button
                disabled={!c || busy || !workspace?.profile}
                onClick={() =>
                  void run(async () => {
                    const t = await invokeCommand('workbench_firmware_prepare', {
                      deviceId: board.id!,
                      candidateId: c!.id,
                      postAction,
                    });
                    setTicket(t);
                    setConfirmed(false);
                  })
                }
              >
                核对文件与安装条件
              </button>
              <button
                className="fw-primary"
                disabled={busy || !ticket || !confirmed || ticket.expiresAt < Date.now() / 1000}
                onClick={() =>
                  void run(async () => {
                    const t = ticket;
                    setTicket(null);
                    setConfirmed(false);
                    const id = await invokeCommand('workbench_firmware_start', {
                      deviceId: board.id!,
                      confirmationId: t!.id,
                      confirmed: true,
                    });
                    setTaskChoice(id);
                    setView('verify');
                    onTaskStarted?.(id);
                  })
                }
              >
                确认并开始烧录
              </button>
              <p className="fw-warning">
                请先按住 FEL 键接入 USB。只查找当前保存的绑定端口；未找到时不会替换设备或自动重试。
              </p>
              <p>当前 NekoBox 无交互 Shell，进入 FEL 需要手动操作。</p>
            </aside>
          </div>
        </>
      )}
      {view === 'verify' && (
        <div className="fw-two-columns">
          <div>
            <section className="fw-card">
              <h4 className="fw-verification-title">
                <LynxIcon name={task?.status === 'failed' ? 'warning' : 'info'} size={24} />
                {details.physicalVerifyState === 'success'
                  ? bootResult === 'pass'
                    ? '写入与回读通过，启动已验收'
                    : bootResult === 'fail'
                      ? '写入与回读通过，启动验收失败'
                      : '写入与回读通过，启动待验收'
                  : task
                    ? `烧录任务：${stateLabel(task.status)}`
                    : '等待烧录与启动验收'}
              </h4>
              <ol className="fw-verify-timeline">
                {['检查固件', '确认设备', '写入固件', '物理回读', '启动验收'].map((label, i) => (
                  <li key={label}>
                    <span
                      className={
                        (i < 2 && !!task) ||
                        (i === 2 && details.writeState === 'success') ||
                        (i === 3 && details.physicalVerifyState === 'success')
                          ? 'is-passed'
                          : ''
                      }
                    >
                      {(i < 2 && !!task) ||
                      (i === 2 && details.writeState === 'success') ||
                      (i === 3 && details.physicalVerifyState === 'success') ? (
                        <LynxIcon name="check" size={16} />
                      ) : (
                        i + 1
                      )}
                    </span>
                    <strong>{label}</strong>
                  </li>
                ))}
              </ol>

              <dl className="fw-verification-facts">
                <dt>镜像物理回读</dt>
                <dd>{stateLabel(details.physicalVerifyState)}</dd>
                <dt>镜像范围外</dt>
                <dd>{stateLabel(details.outsideImageState)}</dd>
                <dt>烧后动作</dt>
                <dd>{stateLabel(details.postFlashState)}</dd>
                <dt>固件启动</dt>
                <dd>{stateLabel(bootResult)}</dd>
                <dt>功能验证</dt>
                <dd>{stateLabel(functionResult)}</dd>
              </dl>
            </section>
            <section className="fw-card">
              <h4>过程日志（终端输出）</h4>
              <pre>
                {details.outputTail?.join('\n') || '尚无过程日志'}
                {task?.error && `\n${task.error}`}
              </pre>
              <button
                disabled={!c}
                onClick={() =>
                  void run(async () => {
                    const path = await save({
                      defaultPath: `${c!.id}-acceptance.json`,
                      filters: [{ name: '验收记录', extensions: ['json'] }],
                    });
                    if (path) {
                      const report = await invokeCommand('workbench_firmware_report', {
                        deviceId: board.id!,
                        candidateId: c!.id,
                      });
                      await writeTextFile(path, report);
                    }
                  })
                }
              >
                导出验收报告
              </button>
            </section>
          </div>
          <aside className="fw-card">
            <h4>启动验收</h4>
            <p>
              串口：{board.serialPort || '未绑定'} · {board.baudRate} / 8N1
            </p>
            <NekoBoxIllustration waiting />
            <p className="fw-evidence-note">
              尚未关联实机画面；请按实际观察填写，示意图不作为启动证据。
            </p>
            {c && workspace && (
              <AcceptanceForm
                key={`${c.id}:${task?.taskId || 'pending'}`}
                board={board}
                candidate={c}
                taskId={task?.taskId || ''}
                workspace={workspace}
                disabled={!task || task.status === 'running'}
                onSaved={refresh}
              />
            )}
            <button
              onClick={() =>
                window.dispatchEvent(
                  new CustomEvent('lynx:firmware-serial', { detail: { deviceId: board.id } })
                )
              }
            >
              查看当前单板串口
            </button>
            <p>当前固件无交互 Shell。只有实际观察并记录的项目才能标为通过。</p>
            <p className="fw-warning">
              重启命令成功不等于启动验证通过；软件进度 100% 不代表所有功能通过。
            </p>
          </aside>
        </div>
      )}
      {view === 'publish' && workspace && (
        <PublicationForm
          board={board}
          candidate={c}
          workspace={workspace}
          onSaved={refresh}
          onCandidateSelected={setSelected}
        />
      )}
    </div>
  );
}

function ProfileForm({ board, onSaved }: { board: WorkbenchDevice; onSaved: () => Promise<void> }) {
  const [profile, setProfile] = useState<FirmwareHardwareProfile>({
    model: 'yuzukineko',
    revision: '',
    chip: 'f101',
    capacityBytes: 16777216,
    display: '1024x768',
    touch: 'gt967',
    operator: '',
    evidence: '',
  });
  const [confirmed, setConfirmed] = useState(false);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  return (
    <section className="fw-card">
      <h4>先核对真实板型与硬件配置</h4>
      <p>保存别名不代表型号。当前支持 YuzukiNeko / F101、16 MiB SPI NOR、1024×768 / GT967 配方。</p>
      {(['revision', 'operator', 'evidence'] as const).map((k) => (
        <label className="fw-row" key={k}>
          {
            {
              revision: '板卡修订',
              operator: '核对人员',
              evidence: '核对依据（丝印 / BOM / 实测）',
            }[k]
          }
          <input
            value={profile[k]}
            onChange={(e) => {
              setProfile({ ...profile, [k]: e.target.value });
              setConfirmed(false);
            }}
          />
        </label>
      ))}
      <label>
        <input
          type="checkbox"
          checked={confirmed}
          onChange={(e) => setConfirmed(e.target.checked)}
        />{' '}
        我已核对以上型号、容量和显示/触摸配置
      </label>
      <button
        disabled={!confirmed || busy}
        onClick={async () => {
          setBusy(true);
          setError('');
          try {
            await invokeCommand('workbench_firmware_save_profile', {
              deviceId: board.id!,
              profile,
              confirmed,
            });
            await onSaved();
          } catch (e) {
            setError(firmwareError(e));
          } finally {
            setBusy(false);
          }
        }}
      >
        保存硬件核对
      </button>
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
function AcceptanceForm({
  board,
  candidate,
  taskId,
  workspace,
  disabled,
  onSaved,
}: {
  board: WorkbenchDevice;
  candidate: FirmwareCandidate;
  taskId: string;
  workspace: FirmwareWorkspace;
  disabled: boolean;
  onSaved: () => Promise<void>;
}) {
  const [operator, setOperator] = useState('');
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [items, setItems] = useState<FirmwareAcceptanceItem[]>(() =>
    Object.keys(checkLabels).map(
      (item) =>
        workspace.acceptance
          .filter((a) => a.candidateId === candidate.id && a.taskId === taskId)
          .flatMap((a) => a.items)
          .find((i) => i.item === item) || { item, result: 'not_verified', evidence: '' }
    )
  );
  return (
    <div className="fw-acceptance-form">
      <div className="fw-acceptance-quick">
        {['display', 'touch'].map((name) => (
          <label key={name}>
            <input
              type="checkbox"
              disabled={disabled || busy}
              checked={items.find((i) => i.item === name)?.result === 'pass'}
              onChange={(e) =>
                setItems(
                  items.map((i) =>
                    i.item === name
                      ? { ...i, result: e.target.checked ? 'pass' : 'not_verified' }
                      : i
                  )
                )
              }
            />
            {name === 'display' ? '显示正常' : '触摸正常'}
          </label>
        ))}
      </div>
      <details className="fw-acceptance-evidence">
        <summary>填写验收人员与逐项证据</summary>
        <label className="fw-row">
          验收人员
          <input value={operator} onChange={(e) => setOperator(e.target.value)} />
        </label>
        {items.map((i, index) => (
          <div className="fw-acceptance-item" key={i.item}>
            <label>
              {checkLabels[i.item]}
              <select
                value={i.result}
                onChange={(e) =>
                  setItems(
                    items.map((v, n) =>
                      n === index
                        ? { ...v, result: e.target.value as FirmwareAcceptanceItem['result'] }
                        : v
                    )
                  )
                }
              >
                <option value="not_verified">未验证</option>
                <option value="pass">通过</option>
                <option value="fail">失败</option>
              </select>
            </label>
            <input
              aria-label={`${checkLabels[i.item]}证据`}
              placeholder="实际观察、日志标识或证据文件位置"
              value={i.evidence}
              onChange={(e) =>
                setItems(
                  items.map((v, n) => (n === index ? { ...v, evidence: e.target.value } : v))
                )
              }
            />
          </div>
        ))}
      </details>
      <button
        className="fw-primary"
        disabled={disabled || busy || !operator.trim()}
        onClick={async () => {
          setBusy(true);
          try {
            await invokeCommand('workbench_firmware_acceptance', {
              deviceId: board.id!,
              candidateId: candidate.id,
              taskId,
              operator,
              items,
            });
            await onSaved();
            setError('验收记录已保存；未测项目保持未验证。');
          } catch (e) {
            setError(firmwareError(e));
          } finally {
            setBusy(false);
          }
        }}
      >
        保存验收记录
      </button>
      {error && <p role="status">{error}</p>}
      <details>
        <summary>查看历史验收记录</summary>
        {workspace.acceptance
          .filter((a) => a.candidateId === candidate.id && a.taskId === taskId)
          .map((a) => (
            <section key={a.id}>
              <p>
                {new Date(a.recordedAt * 1000).toLocaleString()} · {a.operator}
              </p>
              {a.items.map((i) => (
                <p key={i.item}>
                  {checkLabels[i.item]}：{stateLabel(i.result)} · {i.evidence || '无证据'}
                </p>
              ))}
            </section>
          ))}
      </details>
    </div>
  );
}
export function PublicationForm({
  board,
  candidate,
  workspace,
  onSaved,
  onCandidateSelected,
}: {
  board: WorkbenchDevice;
  candidate?: FirmwareCandidate;
  workspace: FirmwareWorkspace;
  onSaved: () => Promise<void>;
  onCandidateSelected: (id: string) => void;
}) {
  const [version, setVersion] = useState('');
  const [channel, setChannel] = useState<'test' | 'stable'>('test');
  const [notes, setNotes] = useState('');
  const [license, setLicense] = useState('');
  const [rights, setRights] = useState(false);
  const [confirmed, setConfirmed] = useState(false);
  const [requestId, setRequestId] = useState(() => crypto.randomUUID());
  const [resumeRequest, setResumeRequest] = useState<FirmwarePublishRequest | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [check, setCheck] = useState<{
    ready: boolean;
    localError: string | null;
    permissionError: string | null;
    actor: string | null;
  } | null>(null);
  const [reason, setReason] = useState('');
  useEffect(() => {
    setCheck(null);
    setConfirmed(false);
    setRequestId(crypto.randomUUID());
  }, [version, channel, notes, license, rights, candidate?.id]);
  const request: FirmwarePublishRequest = {
    requestId,
    candidateId: candidate?.id || '',
    version,
    channel,
    notes,
    distributionLicense: license,
    rightsConfirmed: rights,
  };
  request.requestId = firmwarePublicationRequestId(resumeRequest, request, requestId);
  async function action(fn: () => Promise<void>) {
    setBusy(true);
    setError('');
    try {
      await fn();
      await onSaved();
    } catch (e) {
      setError(firmwareError(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <div data-testid="firmware-publication-page">
      <div className="fw-two-columns">
        <section className="fw-card">
          <div className="fw-publication-fields">
            <label className="fw-row">
              应用 <input readOnly value="NekoBox" />
            </label>
            <label className="fw-row">
              目标板 <input readOnly value="YuzukiNeko · F101" />
            </label>
            <label className="fw-row">
              版本
              <input
                placeholder="例如 0.1.0-preview.1"
                value={version}
                onChange={(e) => setVersion(e.target.value)}
              />
            </label>
            <label className="fw-row">
              通道
              <select
                value={channel}
                onChange={(e) => setChannel(e.target.value as 'test' | 'stable')}
              >
                <option value="test">测试版</option>
                <option value="stable">正式版</option>
              </select>
            </label>
            <label className="fw-row">
              文件 <input readOnly value={candidate?.manifest.image.fileName || '尚未选择'} />
            </label>
            <label className="fw-row">
              文件校验{' '}
              <input
                readOnly
                value={candidate?.manifest.image.sha256 || '由构建记录提供'}
                title={candidate?.manifest.image.sha256}
              />
            </label>
            <label className="fw-row">
              版本说明
              <textarea
                value={notes}
                onChange={(e) => setNotes(e.target.value)}
                placeholder="更新内容、已知问题与实际验证范围"
              />
            </label>
          </div>
          <details className="fw-publication-license">
            <summary>分发许可与来源 · {rights && license.trim() ? '已填写' : '待核对'}</summary>{' '}
            <label className="fw-row">
              分发许可
              <input
                placeholder="核对源码与素材许可后填写"
                value={license}
                onChange={(e) => setLicense(e.target.value)}
              />
            </label>
            <label>
              <input
                type="checkbox"
                checked={rights}
                onChange={(e) => setRights(e.target.checked)}
              />{' '}
              已核对本镜像的源码和素材分发权；本次不上传 Loader
            </label>
          </details>
        </section>
        <aside className="fw-card">
          <h4>发布检查</h4>

          <dl>
            <dt>构建产物</dt>
            <dd title={candidate?.buildId || '需要可信构建记录'}>
              {candidate?.buildId ? '已关联' : '待关联'}
            </dd>
            <dt>Loader</dt>
            <dd title="本地提供，仅记录匹配要求">本地提供</dd>
            <dt>实机证据</dt>
            <dd>{channel === 'stable' ? '需完整实机验收' : '测试通道 · 未验收'}</dd>
            <dt>GitHub 身份</dt>
            <dd title={check?.actor || '使用 GitHub CLI 登录'}>{check?.actor || '待核对'}</dd>
          </dl>
          <button
            disabled={busy || !candidate}
            onClick={() =>
              void action(async () =>
                setCheck(
                  await invokeCommand('workbench_firmware_publish_check', {
                    deviceId: board.id!,
                    request,
                  })
                )
              )
            }
          >
            检查发布条件与权限
          </button>
          {check && (
            <p className={check.ready ? '' : 'fw-warning'}>
              {check.ready
                ? '发布条件已满足'
                : `${check.localError || ''} ${check.permissionError || ''}`}
            </p>
          )}
          <label>
            <input
              type="checkbox"
              checked={confirmed}
              onChange={(e) => setConfirmed(e.target.checked)}
              disabled={!check?.ready}
            />{' '}
            确认发布此版本与说明
          </label>
          <button
            className="fw-primary"
            disabled={busy || !check?.ready || !confirmed}
            onClick={() =>
              void action(async () => {
                await invokeCommand('workbench_firmware_publish', {
                  deviceId: board.id!,
                  request,
                  confirmed: true,
                });
                setConfirmed(false);
              })
            }
          >
            发布到应用目录
          </button>
          <details className="fw-publish-help">
            <summary>发布源与恢复说明</summary>
            <p>dshanpi/yuzukihdneko-zephyr</p>
            <p>
              先创建草稿并上传校验；失败后可在版本管理中恢复原请求，重新检查并确认。已发布文件不覆盖。
            </p>
          </details>
        </aside>
      </div>
      {error && (
        <p role="alert" className="firmware-error">
          {error}
        </p>
      )}
      <section className="fw-card">
        <h4>版本管理</h4>
        <label className="fw-row">
          推荐 / 撤回原因
          <input value={reason} onChange={(e) => setReason(e.target.value)} />
        </label>
        <table>
          <thead>
            <tr>
              <th>版本</th>
              <th>通道</th>
              <th>产物文件</th>
              <th>状态</th>
              <th>创建时间</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            {workspace.publications.map((p) => (
              <tr key={p.id}>
                <td>{p.version}</td>
                <td>{stateLabel(p.channel)}</td>
                <td>
                  {workspace.candidates.find((c) => c.id === p.candidateId)?.manifest.image
                    .fileName || '—'}
                </td>
                <td>
                  {stateLabel(p.status)}
                  {p.error && <small>{p.error}</small>}
                </td>
                <td>{new Date(p.createdAt * 1000).toLocaleString()}</td>
                <td>
                  {p.request &&
                    ['prepared', 'draft', 'verified_draft', 'failed'].includes(p.status) && (
                      <button
                        disabled={
                          busy || !workspace.candidates.some((c) => c.id === p.request!.candidateId)
                        }
                        onClick={() => {
                          const restored = p.request!;
                          onCandidateSelected(restored.candidateId);
                          setVersion(restored.version);
                          setChannel(restored.channel);
                          setNotes(restored.notes);
                          setLicense(restored.distributionLicense);
                          setRights(restored.rightsConfirmed);
                          setResumeRequest(restored);
                          setCheck(null);
                          setConfirmed(false);
                          setError('');
                        }}
                      >
                        恢复发布
                      </button>
                    )}
                  {(['recommend', 'withdraw'] as const).map((a) => (
                    <button
                      key={a}
                      disabled={
                        busy ||
                        p.status !== 'published' ||
                        !reason.trim() ||
                        (a === 'recommend' && p.channel !== 'stable')
                      }
                      onClick={async () => {
                        if (
                          !(await confirm(
                            a === 'withdraw'
                              ? '撤回后将阻止该版本的新安装，保留历史记录。确认撤回？'
                              : '将此正式版本设为推荐版本？'
                          ))
                        )
                          return;
                        void action(async () => {
                          await invokeCommand('workbench_firmware_release_action', {
                            deviceId: board.id!,
                            publicationId: p.id,
                            action: a,
                            reason,
                            confirmed: true,
                          });
                        });
                      }}
                    >
                      {a === 'recommend' ? '推荐' : '撤回'}
                    </button>
                  ))}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {!workspace.publications.length && <p>尚无发布记录。</p>}
      </section>
    </div>
  );
}
