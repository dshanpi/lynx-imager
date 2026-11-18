import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { save } from '@tauri-apps/plugin-dialog';
import { writeTextFile } from '@tauri-apps/plugin-fs';
import { LynxIcon, type LynxIconName } from '../../CoreUI';
import { formatErrorMessage } from '../../Utils';
import { formatLinuxBytes } from '../Workbench/linuxWorkbenchModel';
import { ended, stateName, type useBasicTask } from './BasicTask';
import { copyVerifiedText } from './imagerAIControls';
import './V2Write.css';

export interface V2WriteSnapshot {
  board: string;
  chip: string;
  vendor: string;
  storage: string;
  route: string;
  imagePath: string;
  imageSize: number;
  imageSha256: string;
  loaderPath?: string;
  loaderSha256?: string;
  device: string;
  usb: string;
  mode: string;
  verify: string;
  postAction: string;
  boardArt?: ReactNode;
}
type Task = ReturnType<typeof useBasicTask>;
interface Props {
  snapshot: V2WriteSnapshot;
  task?: Task;
  confirmed: boolean;
  busy: boolean;
  onConfirmChange: (value: boolean) => void;
  onStart: () => void;
  onBack: () => void;
  onRetry: () => void;
  onOpenRecords?: () => void;
}
const stages: Array<[string, LynxIconName]> = [
  ['准备', 'list-check'],
  ['擦除', 'broom'],
  ['写入', 'download'],
  ['校验', 'shield'],
  ['结束动作', 'stop'],
];
const basename = (path: string) => path.split(/[\\/]/).pop() || path;
const size = (bytes?: number | null) => (bytes == null ? '—' : formatLinuxBytes(bytes));
function duration(seconds: number) {
  const s = Math.max(0, Math.floor(seconds));
  return s >= 3600
    ? `${Math.floor(s / 3600)} 时 ${Math.floor((s % 3600) / 60)} 分`
    : s >= 60
      ? `${Math.floor(s / 60)} 分 ${s % 60} 秒`
      : `${s} 秒`;
}
const time = (s: number | null) => (s == null ? '未报告' : new Date(s * 1000).toLocaleString());
const good = (s?: string | null) =>
  ['success', 'verified', 'completed', 'passed', 'executed'].includes(s ?? '');
function evidence(value?: string | null, success = '已完成', unknown = '未报告') {
  if (good(value)) return { label: success, tone: 'good' };
  if (value === 'failed') return { label: '失败', tone: 'bad' };
  if (value === 'running') return { label: '进行中', tone: 'live' };
  if (!value || value === 'not_verified' || value === 'pending')
    return { label: unknown, tone: 'wait' };
  return { label: stateName(value), tone: 'neutral' };
}
function stageIndex(phase: string) {
  if (/post_flash|reboot|poweroff/.test(phase)) return 4;
  if (/verify|verifying/.test(phase)) return 3;
  if (/write|writing/.test(phase)) return 2;
  if (/eras/.test(phase)) return 1;
  if (/preflight|prepar|query|metadata|bootstrap|reconnect|loader|probe|backup/.test(phase))
    return 0;
  return -1;
}
function phaseLabel(task: Task) {
  const phase = task.details.phase || task.phase;
  const labels: Record<string, string> = {
    spinor_preflight_blocked: 'SPI NOR 预检',
    spinor_physical_verify: 'SPI NOR 物理回读',
    spinor_write_complete: 'SPI NOR 写入完成',
    spinor_awaiting_boot: '等待启动验证',
    spinor_awaiting_acceptance: '等待验收',
    raw_mbr_verify: 'MBR 校验',
    mbr_write: '写入 MBR',
    emmc_boot0_preflight: 'eMMC 引导区预检',
    emmc_boot0_write: '写入 eMMC 引导区',
    emmc_boot0_verify: '校验 eMMC 引导区',
    post_flash_action: '结束动作',
    complete: '完成',
    query_device: '读取设备信息',
    reconnecting: '等待设备重新连接',
    rk_loader: '下发 Loader',
  };
  return (
    labels[phase] ??
    (stageIndex(phase) >= 0 ? stages[stageIndex(phase)][0] : phase || stateName(task.status))
  );
}

function ToolButton({
  icon,
  children,
  primary,
  ...props
}: React.ButtonHTMLAttributes<HTMLButtonElement> & { icon: LynxIconName; primary?: boolean }) {
  return (
    <button {...props} type="button" className={'v2w-button' + (primary ? ' v2w-primary' : '')}>
      <LynxIcon name={icon} size={16} />
      {children}
    </button>
  );
}
function Info({ children }: { children: ReactNode }) {
  return (
    <div className="v2w-info">
      <LynxIcon name="info" size={20} />
      <span>{children}</span>
    </div>
  );
}
function Heading({
  title,
  subtitle,
  icon,
}: {
  title: string;
  subtitle: string;
  icon: LynxIconName;
}) {
  return (
    <header className="v2w-heading">
      <LynxIcon name={icon} size={40} />
      <div>
        <h2>{title}</h2>
        <p>{subtitle}</p>
      </div>
    </header>
  );
}
function Board({ snapshot: s }: { snapshot: V2WriteSnapshot }) {
  return (
    <div className="v2w-board">
      {s.boardArt || <LynxIcon name="board" size={48} />}
      <div>
        <strong>{s.board}</strong>
        <span>{s.chip}</span>
      </div>
    </div>
  );
}
type Row = [LynxIconName, string, ReactNode];
function Facts({ rows }: { rows: Row[] }) {
  return (
    <dl className="v2w-facts">
      {rows.map(([icon, label, val]) => (
        <div key={label}>
          <dt>
            <LynxIcon name={icon} size={16} />
            {label}
          </dt>
          <dd>{val}</dd>
        </div>
      ))}
    </dl>
  );
}
function Fingerprint({ value }: { value: string }) {
  const [message, setMessage] = useState('');
  return (
    <span className="v2w-fingerprint">
      <code>{value}</code>
      <button
        type="button"
        aria-label="复制镜像 SHA-256"
        title="复制镜像 SHA-256"
        onClick={() => {
          void copyVerifiedText(Promise.resolve(value))
            .then(() => setMessage('已复制'))
            .catch(() => setMessage('可选中指纹复制'));
        }}
      >
        <LynxIcon name="copy" size={16} />
      </button>
      {message && <small role="status">{message}</small>}
    </span>
  );
}
function Stages({ task }: { task?: Task }) {
  const current = task ? stageIndex(task.details.phase || task.phase) : 0;
  const statuses = [
    undefined,
    undefined,
    task?.details.writeState,
    task?.details.verifyState,
    task?.details.postFlashState,
  ];
  return (
    <ol className="v2w-stages" aria-label="烧写阶段">
      {stages.map(([label, icon], i) => {
        const raw = statuses[i];
        const seen = good(raw);
        const active = i === current && (!task || !ended(task.status)) && !seen;
        const failed = raw === 'failed' || (task?.status === 'failed' && i === current && !seen);
        const cls = failed ? 'bad' : seen ? 'good' : active ? 'live' : 'neutral';
        let detail = '等待中';
        if (seen) detail = '完成';
        else if (failed) detail = '失败';
        else if (active) detail = '进行中';
        else if (raw) detail = stateName(raw);
        else if (task && (i < current || ended(task.status))) detail = '未报告';
        return (
          <li key={label} className={'v2w-' + cls}>
            <span className="v2w-stage-icon">
              <LynxIcon name={seen ? 'check' : failed ? 'dismiss' : icon} size={20} />
            </span>
            <strong>{label}</strong>
            {task && <small>{detail}</small>}
          </li>
        );
      })}
    </ol>
  );
}
function Confirm({ snapshot: s, confirmed, busy, onConfirmChange, onBack, onStart }: Props) {
  const slot = useRef<HTMLDivElement>(null);
  const modal = useRef<HTMLDialogElement>(null);
  useLayoutEffect(() => {
    const dialog = modal.current,
      anchor = slot.current;
    if (!dialog || !anchor) return;
    function position() {
      if (!dialog || !anchor) return;
      const rect = anchor.getBoundingClientRect();
      const viewport = anchor.closest('.v2w-region')?.getBoundingClientRect();
      const top = Math.max(0, rect.top, viewport?.top ?? 0);
      const bottom = Math.min(window.innerHeight, rect.bottom, viewport?.bottom ?? rect.bottom);
      Object.assign(dialog.style, {
        left: rect.left + 'px',
        top: top + 'px',
        width: rect.width + 'px',
        height: Math.max(0, bottom - top) + 'px',
      });
    }
    position();
    if (!dialog.open) dialog.showModal();
    const observer = new ResizeObserver(position);
    observer.observe(anchor);
    const region = anchor.closest('.v2w-region');
    if (region) observer.observe(region);
    window.addEventListener('resize', position);
    return () => {
      observer.disconnect();
      window.removeEventListener('resize', position);
      if (dialog.open) dialog.close();
    };
  }, []);
  function back() {
    if (busy) return;
    modal.current?.close();
    onConfirmChange(false);
    onBack();
    requestAnimationFrame(() => {
      const trigger =
        document.querySelector<HTMLElement>('#v2-preflight:enabled') ??
        document.querySelector<HTMLElement>('#v2-step-3:enabled');
      trigger?.focus({ preventScroll: true });
    });
  }
  const rows: Row[] = [
    ['board', '目标板型', s.board],
    [
      'document',
      '系统镜像',
      <>
        <b>{basename(s.imagePath)}</b>
        <small>{size(s.imageSize)}</small>
      </>,
    ],
    ['key', '镜像指纹', <Fingerprint key="image-fingerprint" value={s.imageSha256} />],
    [
      'plug',
      '设备与模式',
      <>
        <b>
          {s.device} · {s.mode}
        </b>
        <small>{s.usb}</small>
      </>,
    ],
    ['list', '烧录路线', s.route],
    ['database', '目标存储', s.storage],
    ['shield', '校验策略', s.verify],
    ['play', '结束动作', s.postAction],
  ];
  if (s.loaderPath)
    rows.splice(5, 0, [
      'download',
      'RAM Loader',
      <>
        <b>{basename(s.loaderPath)}</b>
        {s.loaderSha256 && <small className="v2w-code">{s.loaderSha256}</small>}
      </>,
    ]);
  const plan = [
    '检查设备和镜像，复核本次文件指纹。',
    '按所选路线处理目标存储。',
    '将系统镜像写入选定目标。',
    s.verify,
    s.postAction,
  ];
  return (
    <div className="v2w-layout v2w-confirm-layout">
      <section className="v2w-card v2w-confirm-base">
        <Heading
          title="烧写系统"
          subtitle="确认参数并开始烧录。执行过程中请保持设备连接和供电稳定。"
          icon="board"
        />
        <div className="v2w-plan-device">
          <Board snapshot={s} />
          <div className="v2w-tags">
            <span>
              <LynxIcon name="document" size={16} />
              {basename(s.imagePath)}
            </span>
            <span>
              <LynxIcon name="database" size={16} />
              {s.storage}
            </span>
            <span>
              <LynxIcon name="plug" size={16} />
              {s.mode}
            </span>
          </div>
        </div>
        <div className="v2w-confirm-status">
          <Stages />
          <div className="v2w-ready">
            <LynxIcon name="list-check" size={40} />
            <div>
              <b>准备就绪</b>
              <p>已完成只读预检，等待确认本次设备和镜像。</p>
            </div>
          </div>
        </div>
        <div className="v2w-plan">
          <h3>
            <LynxIcon name="list" size={20} />
            执行计划
          </h3>
          <ol>
            {stages.map(([name], i) => (
              <li key={name}>
                <span>{i + 1}</span>
                <b>{name}</b>
                <div>{plan[i]}</div>
              </li>
            ))}
          </ol>
        </div>
      </section>
      <div ref={slot} className="v2w-drawer-slot" />
      <dialog
        ref={modal}
        className="v2w-drawer"
        aria-labelledby="v2w-confirm-title"
        onCancel={(e) => {
          e.preventDefault();
          back();
        }}
      >
        <header>
          <h2 id="v2w-confirm-title">
            <LynxIcon name="shield" size={24} />
            确认烧录
          </h2>
          <button type="button" aria-label="关闭确认抽屉" onClick={back} disabled={busy}>
            <LynxIcon name="dismiss" size={20} />
          </button>
        </header>
        <p className="v2w-drawer-sub">请仔细核对以下信息，确认无误后再开始烧录。</p>
        <div className="v2w-drawer-scroll">
          <div className="v2w-warning">
            <LynxIcon name="warning" size={40} />
            <div>
              <strong>危险操作：将擦除并写入目标存储！</strong>
              <p>目标存储原有数据可能被覆盖，请先完成备份。</p>
            </div>
          </div>
          <Facts rows={rows} />
          <label className="v2w-consent">
            <input
              type="checkbox"
              checked={confirmed}
              disabled={busy}
              onChange={(e) => onConfirmChange(e.target.checked)}
            />
            我已核对设备，并备份重要数据
          </label>
        </div>
        <footer>
          <ToolButton icon="arrow-left" onClick={back} disabled={busy}>
            返回修改
          </ToolButton>
          <ToolButton icon="play" primary onClick={onStart} disabled={!confirmed || busy}>
            {busy ? '正在复核…' : '确认烧录'}
          </ToolButton>
        </footer>
      </dialog>
    </div>
  );
}
function Summary({
  snapshot: s,
  task,
  compact = false,
}: {
  snapshot: V2WriteSnapshot;
  task: Task;
  compact?: boolean;
}) {
  const finished = ended(task.status);
  const rows: Row[] = compact
    ? [
        ['plug', '目标设备', s.usb],
        ['key', '镜像指纹', <Fingerprint key="image-fingerprint" value={s.imageSha256} />],
        ['list', '烧录路线', s.route],
        ['clock', '执行时间', time(task.startedAt)],
      ]
    : [
        ['plug', '设备模式', s.mode],
        ['document', '镜像文件', basename(s.imagePath)],
        ['archive', '镜像大小', size(s.imageSize)],
        ['key', '镜像指纹', <Fingerprint key="image-fingerprint" value={s.imageSha256} />],
        ['list', '烧录路线', s.route],
        ['database', '目标存储', s.storage],
        ['shield', '校验策略', s.verify],
        ['play', '结束动作', s.postAction],
      ];
  if (finished && !compact)
    rows.push(
      ['clock', '开始时间', time(task.startedAt)],
      ['clock', '完成时间', time(task.finishedAt)],
      [
        'runtime',
        '任务耗时',
        task.startedAt && task.finishedAt ? duration(task.finishedAt - task.startedAt) : '未报告',
      ]
    );
  return (
    <section className="v2w-card v2w-summary">
      <h3>
        <LynxIcon name="list-check" size={20} />
        {compact ? '任务详情' : finished ? '本次任务摘要' : '任务摘要'}
        {!finished && <em>执行中</em>}
        {compact && <em className="v2w-red-pill">需要重新确认</em>}
      </h3>
      {!compact && <Board snapshot={s} />}
      <Facts rows={rows} />
      {!finished && (
        <Info>
          <b>当前阶段</b>
          <br />
          {phaseLabel(task)}
          {task.details.currentPartition && ' · ' + task.details.currentPartition}
        </Info>
      )}
    </section>
  );
}
function Progress({ task }: { task: Task }) {
  const d = task.details;
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);
  const pct = d.progressPct == null ? null : Math.max(0, Math.min(100, d.progressPct));
  const bytes = d.committedBytes ?? d.writtenBytes;
  const unitLabel = bytes != null ? '已写入容量' : '已传输容量';
  const total = d.totalBytes,
    transferred = d.transferredBytes;
  const eta =
    total != null &&
    transferred != null &&
    transferred <= total &&
    d.bytesPerSecond &&
    d.bytesPerSecond > 0
      ? duration((total - transferred) / d.bytesPerSecond)
      : '—';
  const metrics: Row[] = [
    ['runtime', '传输速度', d.bytesPerSecond != null ? size(d.bytesPerSecond) + '/s' : '—'],
    ['database', unitLabel, size(bytes ?? transferred)],
    ['clock', '预计剩余时间', eta],
    ['clock', '已用时间', task.startedAt ? duration(now / 1000 - task.startedAt) : '—'],
  ];
  return (
    <section className="v2w-card v2w-progress">
      <Heading
        title="正在烧录系统"
        subtitle="系统镜像正在处理，请保持 USB 连接稳定，避免断电或断开设备。"
        icon="download"
      />
      <Stages task={task} />
      <div className="v2w-progress-line">
        <strong>{pct == null ? '—' : Math.round(pct) + '%'}</strong>
        <div>
          <div className="v2w-progress-copy">
            <b>
              {phaseLabel(task)}
              {d.currentPartition && ' · ' + d.currentPartition}
            </b>
            {transferred != null && (
              <span>
                已传输 {size(transferred)}
                {total != null && ' / ' + size(total)}
              </span>
            )}
          </div>
          <progress max={100} value={pct ?? undefined} aria-label="任务进度" />
        </div>
      </div>
      <dl className="v2w-metrics">
        {metrics.map(([icon, title, value]) => (
          <div key={title}>
            <LynxIcon name={icon} size={20} />
            <div>
              <dt>{title}</dt>
              <dd>{value}</dd>
            </div>
          </div>
        ))}
      </dl>
    </section>
  );
}
function Logs({
  task,
  onExport,
  compact = false,
}: {
  task: Task;
  onExport: () => void;
  compact?: boolean;
}) {
  const [auto, setAuto] = useState(true),
    [cleared, setCleared] = useState(0),
    [expanded, setExpanded] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  const logs = task.logs.slice(Math.max(0, cleared - (task.logVersion - task.logs.length)));
  useEffect(() => {
    if (auto && box.current) box.current.scrollTop = box.current.scrollHeight;
  }, [auto, task.logVersion]);
  return (
    <section className={'v2w-card v2w-logs' + (compact ? ' v2w-compact' : '')}>
      <header>
        <h3>
          <LynxIcon name={compact ? 'list-check' : 'settings'} size={20} />
          {compact ? '执行日志摘要' : '执行日志（实时）'}
        </h3>
        {compact ? (
          <button
            type="button"
            aria-expanded={expanded}
            className="v2w-text-button"
            onClick={() => setExpanded((v) => !v)}
          >
            {expanded ? '收起日志' : '展开任务日志'}
          </button>
        ) : (
          <div className="v2w-tools">
            <label>
              <input type="checkbox" checked={auto} onChange={(e) => setAuto(e.target.checked)} />
              自动滚动
            </label>
            <ToolButton
              icon="delete"
              onClick={() => setCleared(task.logVersion)}
              disabled={!logs.length}
              title="清空当前视图，保留原始记录"
            >
              清空
            </ToolButton>
            <ToolButton icon="download" onClick={onExport}>
              导出日志
            </ToolButton>
          </div>
        )}
      </header>
      <div
        className={'v2w-log-body' + (expanded ? ' is-expanded' : '')}
        ref={box}
        tabIndex={0}
        aria-label="当前任务日志"
      >
        {(compact && !expanded ? logs.slice(-9) : logs).map((line, i) => (
          <div
            key={task.logVersion - logs.length + i}
            className={/error|fail|失败|错误/i.test(line) ? 'is-error' : ''}
          >
            {line}
          </div>
        ))}
        {!logs.length && <p>{cleared ? '视图已清空，原始记录仍可导出。' : '等待后端日志…'}</p>}
      </div>
    </section>
  );
}
function Results({ task, storage }: { task: Task; storage: string }) {
  const d = task.details;
  const items: Array<[LynxIconName, string, ReturnType<typeof evidence>]> = [
    ['database', '镜像写入', evidence(d.writeState)],
    ['shield', '协议校验', evidence(d.verifyState, '已通过', '待确认')],
  ];
  if (d.physicalVerifyState || d.storageType === 'spinor' || /spi\s*nor/i.test(storage))
    items.push(['search', '物理回读', evidence(d.physicalVerifyState, '已通过', '待确认')]);
  items.push(
    ['play', '结束动作', evidence(d.postFlashState, '已执行', '待确认')],
    ['desktop', '系统启动', evidence(d.bootVerifyState, '已验证', '待确认')]
  );
  return (
    <ul className="v2w-results">
      {items.map(([icon, name, state]) => (
        <li key={name}>
          <span>
            <LynxIcon name={icon} size={24} />
          </span>
          <strong>{name}</strong>
          <small className={'v2w-label-' + state.tone}>{state.label}</small>
        </li>
      ))}
    </ul>
  );
}
function Success({ task, storage }: { task: Task; storage: string }) {
  const d = task.details;
  const detail =
    good(d.writeState) && good(d.verifyState)
      ? '系统镜像已写入并校验通过'
      : '烧录任务已结束，执行结果见下方记录';
  return (
    <section className="v2w-card v2w-success">
      <div className="v2w-celebration">
        <div className="v2w-success-mark">
          <LynxIcon name="check" size={48} />
        </div>
        <h2>烧录完成</h2>
        <p>{detail}</p>
      </div>
      <Results task={task} storage={storage} />
      <Info>
        {good(d.bootVerifyState)
          ? '后端已记录系统启动验证结果。'
          : '请通过开发板屏幕、调试串口或有效启动日志确认系统启动。'}
      </Info>
    </section>
  );
}
const recovery: Array<[LynxIconName, string, string]> = [
  ['plug', '检查 USB 连接和供电', '确认 USB 线缆、连接端口和供电稳定。'],
  ['desktop', '确认设备模式', '按板型说明重新进入所选路线要求的烧录模式。'],
  ['refresh', '重新检测设备', '重读 USB 身份，并重新绑定本次目标。'],
  ['list', '核对镜像与 Loader', '复核镜像文件、指纹和路线所需的 Loader。'],
];
function Failure({
  task,
  onRetry,
  onExport,
}: {
  task: Task;
  onRetry: () => void;
  onExport: () => void;
}) {
  return (
    <section className="v2w-card v2w-failure">
      <Heading title="烧写系统" subtitle="查看当前任务的失败阶段和设备状态。" icon="list-check" />
      <div className="v2w-failure-banner" role="alert">
        <div className="v2w-failure-title">
          <span>
            <LynxIcon name="dismiss" size={40} />
          </span>
          <div>
            <h2>{task.status === 'failed' ? '烧录失败' : stateName(task.status)}</h2>
            <p>任务已停止，设备状态需重新确认。</p>
          </div>
        </div>
        <p>
          <b>最后报告阶段</b> {phaseLabel(task)}
        </p>
        <div className="v2w-failure-error">
          <LynxIcon name="error" size={20} />
          <span>
            {task.error ||
              task.details.postFlashError ||
              task.details.diagnosticErrorCode ||
              '后端未返回详细错误。请导出任务记录。'}
          </span>
        </div>
      </div>
      <h3>
        <LynxIcon name="wrench" size={24} />
        恢复建议
      </h3>
      <ol className="v2w-recovery">
        {recovery.map(([icon, name, text], i) => (
          <li key={name}>
            <span>
              <LynxIcon name={icon} size={20} />
            </span>
            <div>
              <strong>
                {i + 1}. {name}
              </strong>
              <p>{text}</p>
            </div>
          </li>
        ))}
      </ol>
      <div className="v2w-failure-actions">
        <ToolButton icon="refresh" primary onClick={onRetry}>
          重新检测设备
        </ToolButton>
        <ToolButton icon="download" onClick={onExport}>
          导出诊断记录
        </ToolButton>
        <small>再次写入前需要重新确认。</small>
      </div>
    </section>
  );
}
export function V2Write(props: Props) {
  const { snapshot, task, onRetry, onOpenRecords } = props;
  const [exportStatus, setExportStatus] = useState('');
  const [exporting, setExporting] = useState(false);
  async function exportTask(diagnostic = false) {
    if (!task || exporting) return;
    setExporting(true);
    setExportStatus('');
    try {
      const ext = diagnostic ? 'json' : 'txt';
      const dest = await save({
        defaultPath: 'LYNX-' + task.id.replace(/[^\w-]/g, '_') + '.' + ext,
        filters: [{ name: diagnostic ? '诊断记录' : '任务日志', extensions: [ext] }],
      });
      if (!dest) return;
      // The board artwork is a React node and has no place in a diagnostic export.
      const { boardArt: art, ...parameters } = snapshot;
      void art;
      const digest = JSON.stringify(
        {
          taskId: task.id,
          status: task.status,
          phase: task.phase,
          parameters,
          startedAt: task.startedAt,
          finishedAt: task.finishedAt,
          details: task.details,
          error: task.error,
          logTail: task.logs,
        },
        null,
        2
      );
      const lines = [
        'LYNX Imager · 任务日志',
        task.id,
        snapshot.device,
        snapshot.usb,
        snapshot.imagePath,
        'SHA-256 ' + snapshot.imageSha256,
        '',
        ...task.logs,
      ];
      if (task.error) lines.push(task.error);
      await writeTextFile(dest, diagnostic ? digest : lines.join('\n'));
      setExportStatus('已导出 ' + basename(dest));
    } catch (error) {
      setExportStatus('导出失败：' + formatErrorMessage(error));
    } finally {
      setExporting(false);
    }
  }
  let body;
  if (!task?.id) body = <Confirm {...props} />;
  else if (!ended(task.status))
    body = (
      <div className="v2w-layout v2w-running">
        <div className="v2w-main-stack">
          <Progress task={task} />
          <Logs key={task.id} task={task} onExport={() => void exportTask()} />
        </div>
        <Summary snapshot={snapshot} task={task} />
        <div className="v2w-footer-lock">
          <LynxIcon name="lock" size={24} />
          <div>
            <strong>任务执行中，设备与参数已锁定</strong>
            <p>保持连接，等待当前任务结束后再切换设备或修改镜像。</p>
          </div>
        </div>
      </div>
    );
  else if (task.status === 'success')
    body = (
      <div className="v2w-layout v2w-done">
        <Success task={task} storage={snapshot.storage} />
        <Summary snapshot={snapshot} task={task} />
        <div className="v2w-success-actions">
          <ToolButton icon="add" primary onClick={onRetry}>
            烧录另一块板
          </ToolButton>
          {onOpenRecords && (
            <ToolButton icon="document" onClick={onOpenRecords}>
              查看任务报告
            </ToolButton>
          )}
          <ToolButton icon="download" onClick={() => void exportTask()}>
            导出日志
          </ToolButton>
        </div>
      </div>
    );
  else
    body = (
      <div className="v2w-layout v2w-failed">
        <Failure task={task} onRetry={onRetry} onExport={() => void exportTask(true)} />
        <div className="v2w-main-stack">
          <Summary snapshot={snapshot} task={task} compact />
          <Logs key={task.id} compact task={task} onExport={() => void exportTask()} />
          <section className="v2w-card v2w-failure-results">
            <h3>
              <LynxIcon name="settings" size={20} />
              任务状态
            </h3>
            <Results task={task} storage={snapshot.storage} />
          </section>
        </div>
      </div>
    );
  return (
    <div className="v2w-region">
      {body}
      {task?.observationError && (
        <p className="v2w-feedback" role="alert">
          读取任务状态遇到问题，正在重读。{task.observationError}
        </p>
      )}
      {exportStatus && (
        <p className="v2w-feedback" role="status">
          {exportStatus}
        </p>
      )}
    </div>
  );
}
