import React, { useEffect, useMemo, useRef, useState } from 'react';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import BasicCatalog from './BasicCatalog';
import { BasicBoardDrawing } from './BasicBoardDrawing';
import { flashOptions, uniqueUsbTarget, type BasicRoute } from './BasicPlan';
import { BasicTask, ended, stateName, useBasicTask } from './BasicTask';
import {
  formatLinuxBytes,
  formatUsbBinding,
  linuxVendorLabel,
} from '../Workbench/linuxWorkbenchModel';
import { formatErrorMessage } from '../../Utils';
import type {
  FirmwareCandidate,
  FirmwareConfirmation,
  WorkbenchFlasherDevice,
} from '../../Platform/IPC/contracts/Workbench';
import { open } from '@tauri-apps/plugin-dialog';
import { invokeCommand, type WorkbenchDevice } from '../../Platform/IPC';
import './ImagerPage.css';

type BasicStep = 1 | 2 | 3 | 4 | 5;

type BoardPreset = {
  id: string;
  name: string;
  chipVendor: WorkbenchDevice['chipVendor'];
  chip: string;
  storage: string;
  mode: string;
  route: BasicRoute;
  needsLoader: boolean;
  loaderLabel?: string;
};

const BOARD_PRESETS: BoardPreset[] = [
  {
    id: 't113-pro',
    name: 'T113-Pro',
    chipVendor: 'allwinner',
    chip: '全志 · T113',
    storage: 'SPI NAND',
    mode: 'FEL',
    route: 'vendor',
    needsLoader: false,
  },
  {
    id: 'f101',
    name: 'F101',
    chipVendor: 'allwinner',
    chip: '全志 · F101',
    storage: 'SPI NOR',
    mode: 'FEL',
    route: 'mainline_spinor',
    needsLoader: true,
  },
  {
    id: 'rk3588',
    name: 'RK3588 系列',
    chipVendor: 'rockchip',
    chip: '瑞芯微 · RK3588',
    storage: 'eMMC',
    mode: 'Maskrom / Loader',
    route: 'rockchip_raw_emmc',
    needsLoader: true,
    loaderLabel: 'Rockchip Loader',
  },
  {
    id: 'nxp-imx',
    name: 'i.MX 系列',
    chipVendor: 'nxp',
    chip: 'NXP · i.MX 8 / 9',
    storage: 'eMMC / SD',
    mode: 'SDP / Fastboot',
    route: 'vendor',
    needsLoader: false,
  },
  {
    id: 'stm32mp1',
    name: 'STM32MP1 系列',
    chipVendor: 'st',
    chip: 'ST · STM32MP1',
    storage: 'eMMC / SD',
    mode: 'USB DFU / DfuSe',
    route: 'vendor',
    needsLoader: false,
  },
  {
    id: 'canaan-k230',
    name: 'K230 系列',
    chipVendor: 'kendryte',
    chip: 'Canaan · Kendryte K230',
    storage: 'eMMC / SD',
    mode: 'BROM / U-Boot',
    route: 'vendor',
    needsLoader: false,
  },
];

function BoardGlyph({ board, large = false }: { board: BoardPreset; large?: boolean }) {
  if (board.id === 't113-pro') return <BasicBoardDrawing />;
  return (
    <svg
      className={`imager-board-glyph ${large ? 'is-large' : ''}`}
      viewBox="0 0 160 92"
      role="img"
      aria-label={`${board.name} 示意图`}
    >
      <rect x="10" y="12" width="140" height="68" rx="8" />
      <circle cx="20" cy="22" r="4" />
      <circle cx="140" cy="22" r="4" />
      <circle cx="20" cy="70" r="4" />
      <circle cx="140" cy="70" r="4" />
      <rect className="imager-board-glyph-chip" x="62" y="28" width="36" height="30" rx="4" />
      <rect x="30" y="28" width="18" height="8" rx="2" />
      <rect x="108" y="30" width="20" height="8" rx="2" />
      <rect x="34" y="50" width="13" height="8" rx="2" />
      <rect x="105" y="50" width="17" height="8" rx="2" />
      {board.chipVendor === 'rockchip' && <path d="M122 42h12v22h-12z" />}
      {board.chipVendor === 'nxp' && <rect x="48" y="62" width="26" height="7" rx="2" />}
      {board.chipVendor === 'st' && <path d="M112 19v12m6-12v12m6-12v12" />}
      {board.chipVendor === 'kendryte' && <rect x="28" y="40" width="12" height="12" rx="2" />}
    </svg>
  );
}

const STEPS = ['选择开发板', '选择镜像', '配置接口', '确认烧录', '写入与校验'] as const;
const RK_BOARDS = ['RK3588-EVB1', 'LubanCat-4 (RK3588S)', 'Rock 5B (RK3588)'];
type FileIdentity = { sizeBytes: number; sha256: string };
type LocalFiles = { image: FileIdentity; loader: FileIdentity | null };
type CheckedTarget = {
  device: WorkbenchDevice;
  usb: WorkbenchFlasherDevice;
  ticket: FirmwareConfirmation | null;
};

export interface ImagerPageProps {
  onOpenAdvancedWorkbench: () => void;
  onOpenRecords?: () => void;
  onOpenSettings?: () => void;
  onStatusChange?: (status: { target: string; task: string }) => void;
}

function PropsTable({ items }: { items: Array<[string, React.ReactNode]> }) {
  return (
    <dl className="basic-props">
      {items.map(([label, value]) => (
        <div key={label}>
          <dt>{label}</dt>
          <dd>{value}</dd>
        </div>
      ))}
    </dl>
  );
}
function signature(device: WorkbenchDevice) {
  return JSON.stringify([
    device.id,
    device.boardUid,
    device.chipVendor,
    device.usbBinding,
    device.flasherKind,
    device.flasherPath,
    device.flasherArgs,
  ]);
}
async function identities(imagePath: string, loaderPath: string): Promise<LocalFiles> {
  const [image, loader] = await Promise.all([
    invokeCommand('imager_file_identity', { path: imagePath }),
    loaderPath ? invokeCommand('imager_file_identity', { path: loaderPath }) : null,
  ]);
  return { image, loader };
}
async function checkTarget(selected: WorkbenchDevice) {
  const devices = await invokeCommand('workbench_list_devices', undefined);
  const device = devices.find((d) => d.id === selected.id);
  if (!device || signature(device) !== signature(selected))
    throw new Error('保存的设备或接口配置已变化。请刷新列表并重新选择。');
  if (!device.usbBinding)
    throw new Error('该设备尚未绑定 USB 接口。请到高级烧录的设备设置中绑定。');
  const found = await invokeCommand('workbench_scan_flasher_devices', {
    kind: device.flasherKind,
    path: device.flasherPath,
  });
  const usb = uniqueUsbTarget(device.usbBinding, device.flasherKind, found);
  return { device, usb };
}

export const ImagerPage: React.FC<ImagerPageProps> = ({
  onOpenAdvancedWorkbench,
  onOpenRecords,
  onOpenSettings,
  onStatusChange,
}) => {
  const [step, setStep] = useState<BasicStep>(1);
  const [boardId, setBoardId] = useState('t113-pro');
  const [rkBoard, setRkBoard] = useState('');
  const [search, setSearch] = useState('');
  const [vendor, setVendor] = useState('all');
  const [imagePath, setImagePath] = useState('');
  const [loaderPath, setLoaderPath] = useState('');
  const [files, setFiles] = useState<LocalFiles | null>(null);
  const [source, setSource] = useState<'local' | 'catalog'>('local');
  const [candidate, setCandidate] = useState<FirmwareCandidate | null>(null);
  const [devices, setDevices] = useState<WorkbenchDevice[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [target, setTarget] = useState<CheckedTarget | null>(null);
  const [confirmed, setConfirmed] = useState(false);
  const [taskId, setTaskId] = useState('');
  const [busy, setBusy] = useState('');
  const [error, setError] = useState('');
  const lock = useRef(false);
  const dropZone = useRef<HTMLDivElement>(null);
  const board = BOARD_PRESETS.find((item) => item.id === boardId)!;
  const boardName = board.id === 'rk3588' && rkBoard ? rkBoard : board.name;
  const selected = devices.find((item) => item.id === selectedId);
  const task = useBasicTask(taskId);
  const terminal = !!taskId && ended(task.status);
  const running = !!taskId && !terminal;
  const locked = !!busy || running;
  const visible = useMemo(
    () =>
      BOARD_PRESETS.filter(
        (b) =>
          (vendor === 'all' || vendor === b.chipVendor) &&
          [
            b.name,
            b.chip,
            b.storage,
            linuxVendorLabel(b.chipVendor),
            ...(b.id === 'rk3588' ? RK_BOARDS : []),
          ]
            .join(' ')
            .toLowerCase()
            .includes(search.trim().toLowerCase())
      ),
    [search, vendor]
  );
  const usesSavedVendorRecipe = board.route === 'vendor' && board.chipVendor !== 'allwinner';
  const statusText = taskId
    ? stateName(task.status)
    : busy || (step === 4 ? '等待确认' : '任务空闲');
  const statusTarget = selected?.name || boardName;
  const lastReported = useRef<{ target: string; task: string } | null>(null);
  useEffect(() => {
    if (
      !onStatusChange ||
      (lastReported.current?.task === statusText && lastReported.current.target === statusTarget)
    )
      return;
    lastReported.current = { target: statusTarget, task: statusText };
    onStatusChange(lastReported.current);
  }, [onStatusChange, statusText, statusTarget]);

  function clearCheck() {
    setConfirmed(false);
    setTarget(null);
    setError('');
  }
  function editFile(which: 'image' | 'loader', value: string) {
    if (which === 'image') setImagePath(value);
    else setLoaderPath(value);
    setFiles(null);
    setCandidate(null);
    clearCheck();
  }
  useEffect(() => {
    if (step !== 2 || source !== 'local') return;
    let disposed = false;
    let release: (() => void) | undefined;
    try {
      void getCurrentWebview()
        .onDragDropEvent((e) => {
          if (disposed || e.payload.type !== 'drop' || e.payload.paths.length !== 1 || lock.current)
            return;
          const zone = dropZone.current;
          if (!zone || getComputedStyle(zone).visibility === 'hidden') return;
          const rect = zone.getBoundingClientRect();
          const x = e.payload.position.x / window.devicePixelRatio,
            y = e.payload.position.y / window.devicePixelRatio;
          if (x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom) {
            setImagePath(e.payload.paths[0]);
            setFiles(null);
            setCandidate(null);
            setConfirmed(false);
            setTarget(null);
            setError('');
          }
        })
        .then((stop) => {
          if (disposed) stop();
          else release = stop;
        })
        .catch(() => {
          /* File picker stays available. */
        });
    } catch {
      /* A web-only preview cannot receive native paths. */
    }
    return () => {
      disposed = true;
      release?.();
    };
  }, [step, source]);

  async function run(label: string, action: () => Promise<void>) {
    if (lock.current || running) return;
    lock.current = true;
    setBusy(label);
    setError('');
    try {
      await action();
    } catch (e) {
      setError(formatErrorMessage(e));
      setConfirmed(false);
    } finally {
      lock.current = false;
      setBusy('');
    }
  }
  function changeBoard(id: string) {
    if (id === boardId) return;
    setBoardId(id);
    setRkBoard('');
    setSource('local');
    setImagePath('');
    setLoaderPath('');
    setFiles(null);
    setDevices([]);
    setLoaded(false);
    setSelectedId(null);
    setCandidate(null);
    clearCheck();
  }
  async function readDevices() {
    const result = await invokeCommand('workbench_list_devices', undefined, { timeoutMs: 8000 });
    const matching = result.filter((d) => d.id != null && d.chipVendor === board.chipVendor);
    setDevices(matching);
    setLoaded(true);
    if (!matching.some((d) => d.id === selectedId)) setSelectedId(null);
  }
  function goBack(to: BasicStep) {
    if (locked || to >= step || step === 5) return;
    clearCheck();
    setStep(to);
  }
  async function chooseFile(which: 'image' | 'loader') {
    const path = await open({
      multiple: false,
      directory: false,
      title: which === 'image' ? '选择本地镜像' : '选择匹配的 Loader',
      filters:
        which === 'image'
          ? [
              {
                name: '固件文件',
                extensions: ['img', 'bin', 'xz', 'gz', 'wic', 'dfu', 'tsv', 'uuu'],
              },
              { name: '所有文件', extensions: ['*'] },
            ]
          : [{ name: 'Loader', extensions: ['fex', 'bin', 'img'] }],
    });
    if (typeof path === 'string') editFile(which, path);
  }
  async function continueFiles() {
    const idents = await identities(imagePath.trim(), board.needsLoader ? loaderPath.trim() : '');
    if (!idents.image.sizeBytes || (idents.loader && !idents.loader.sizeBytes))
      throw new Error('所选镜像或 Loader 为空。请选择有效文件。');
    if (
      candidate &&
      (candidate.manifest.image.sha256 !== idents.image.sha256 ||
        candidate.manifest.loader.sha256 !== idents.loader?.sha256)
    )
      throw new Error('候选文件的 SHA-256 与发布清单不符。请重新获取该版本。');
    setFiles(idents);
    setStep(3);
    clearCheck();
    await readDevices();
  }
  async function preflight() {
    if (!selected?.id || !files) return;
    clearCheck();
    const checked = await checkTarget(selected);
    if (
      candidate &&
      (candidate.deviceId !== selected.id || candidate.boardUid !== selected.boardUid)
    )
      throw new Error('候选包绑定的设备已更改。请重新获取发布资源。');
    const ticket = candidate
      ? await invokeCommand('workbench_firmware_prepare', {
          deviceId: selected.id,
          candidateId: candidate.id,
          postAction: 'reboot',
        })
      : null;
    setTarget({ ...checked, ticket });
    setStep(4);
  }
  async function startFlash() {
    if (!confirmed || !files || !target?.device.id || taskId) return;
    let current: LocalFiles;
    try {
      current = await identities(imagePath.trim(), board.needsLoader ? loaderPath.trim() : '');
    } catch (e) {
      setFiles(null);
      setTarget(null);
      setStep(2);
      throw e;
    }
    if (
      current.image.sha256 !== files.image.sha256 ||
      current.loader?.sha256 !== files.loader?.sha256
    ) {
      setFiles(null);
      setTarget(null);
      setStep(2);
      throw new Error('文件内容已变化。请重新核对镜像和 Loader。');
    }
    let fresh;
    try {
      fresh = await checkTarget(target.device);
    } catch (e) {
      setTarget(null);
      setStep(3);
      throw e;
    }
    if (
      fresh.usb.key !== target.usb.key ||
      fresh.usb.mode !== target.usb.mode ||
      fresh.usb.vid !== target.usb.vid ||
      fresh.usb.pid !== target.usb.pid
    ) {
      setTarget(null);
      setStep(3);
      throw new Error('USB 设备标识或工作模式已变化，请重新检查接口。');
    }
    const options = flashOptions(board.route, board.chipVendor, loaderPath);
    setConfirmed(false);
    try {
      const started = target.ticket
        ? await invokeCommand('workbench_firmware_start', {
            deviceId: target.device.id,
            confirmationId: target.ticket.id,
            confirmed: true,
          })
        : await invokeCommand('workbench_start_flash', {
            deviceId: target.device.id,
            firmwarePath: imagePath.trim(),
            options,
          });
      setTaskId(started);
      setStep(5);
    } catch (e) {
      // Firmware publication bookkeeping can fail after the native flash task already started.
      const startedTask =
        target.ticket &&
        /FIRMWARE_TASK_STARTED_RECORD_FAILED:([^:\s]+):DO_NOT_RETRY/.exec(formatErrorMessage(e));
      if (startedTask) {
        setTaskId(startedTask[1]);
        setStep(5);
        setError('烧录任务已启动。固件发布记录未完整保存，请从当前任务查看执行结果。');
        return;
      }
      setTarget(null);
      setStep(3);
      throw e;
    }
  }
  function retry() {
    if (!terminal) return;
    setTaskId('');
    clearCheck();
    setSelectedId(candidate?.deviceId ?? null);
    setStep(3);
  }
  const chooseTarget = (id: number | null) => {
    setSelectedId(id);
    clearCheck();
    if (candidate && candidate.deviceId !== id) {
      setCandidate(null);
      setFiles(null);
      setImagePath('');
      setLoaderPath('');
    }
  };
  const route =
    board.route === 'mainline_spinor'
      ? 'SPI NOR 完整镜像'
      : board.route === 'rockchip_raw_emmc'
        ? 'eMMC 完整镜像'
        : usesSavedVendorRecipe
          ? '已保存的厂商烧录配置'
          : '全志厂家镜像';
  const targetPicker = (
    <label className="basic-field">
      <span>已保存的目标单板</span>
      <select
        value={selectedId ?? ''}
        onChange={(e) => chooseTarget(e.target.value ? Number(e.target.value) : null)}
        disabled={!!candidate && step === 3}
      >
        <option value="">选择目标单板…</option>
        {devices.map((d) => (
          <option value={d.id!} key={d.id}>
            {d.name || d.boardUid}
          </option>
        ))}
      </select>
    </label>
  );
  const picker = (which: 'image' | 'loader') => (
    <div className="basic-field">
      <label htmlFor={'basic-' + which}>
        {which === 'image' ? '镜像文件路径' : (board.loaderLabel || 'RAM FES Loader') + '（必填）'}
      </label>
      <div className="basic-file-row">
        <input
          id={'basic-' + which}
          required
          aria-describedby={
            which === 'loader' && imagePath && !loaderPath ? 'basic-loader-hint' : undefined
          }
          spellCheck={false}
          value={which === 'image' ? imagePath : loaderPath}
          onChange={(e) => editFile(which, e.target.value)}
          placeholder={which === 'image' ? '选择文件或粘贴完整路径…' : '选择匹配当前板型的加载器…'}
        />
        <button type="button" onClick={() => void run('选择文件…', () => chooseFile(which))}>
          选择文件
        </button>
      </div>
    </div>
  );
  return (
    <section className="imager-page" aria-label="基础烧录">
      <aside className="basic-rail">
        <h2>烧录系统</h2>
        <p>将系统写入开发板</p>
        <nav aria-label="烧录步骤">
          <ol>
            {STEPS.map((label, i) => (
              <li key={label}>
                <button
                  type="button"
                  aria-current={step === i + 1 ? 'step' : undefined}
                  disabled={locked || i + 1 >= step || step === 5}
                  onClick={() => goBack((i + 1) as BasicStep)}
                >
                  <span aria-hidden="true">{i + 1 < step ? '✓' : i + 1}</span>
                  {label}
                </button>
              </li>
            ))}
          </ol>
        </nav>
        <div className="basic-links">
          {onOpenRecords && (
            <button type="button" onClick={onOpenRecords}>
              任务记录
            </button>
          )}
          {onOpenSettings && (
            <button type="button" onClick={onOpenSettings}>
              应用设置
            </button>
          )}
        </div>
      </aside>
      <div className="basic-work">
        <header className="basic-context">
          <span className="basic-breadcrumb">
            <span aria-hidden="true">ϟ</span>
            <strong>基础烧录</strong> / 烧录向导
          </span>
          <span>步骤 {step} / 5</span>
        </header>
        <div className="basic-scroll">
          <h2 aria-live="polite" aria-atomic="true">
            {step === 5 ? (ended(task.status) ? '任务结果' : '写入进度') : STEPS[step - 1]}
          </h2>
          <fieldset className="basic-content" disabled={!!busy}>
            {step === 1 && (
              <>
                <p className="basic-intro">
                  按板上丝印选择型号。LYNX Imager 会据此筛选镜像和烧录接口。
                </p>
                <div className="basic-columns imager-board-picker">
                  <section>
                    <h3>板型目录</h3>
                    <div className="basic-filter">
                      <label className="basic-field imager-board-search">
                        <span>搜索板型</span>
                        <input
                          type="search"
                          aria-label="搜索板型、芯片或厂商"
                          placeholder="板型、芯片或存储…"
                          value={search}
                          onChange={(e) => setSearch(e.target.value)}
                        />
                      </label>
                      <label className="basic-field imager-board-filters">
                        <span>芯片厂商</span>
                        <select value={vendor} onChange={(e) => setVendor(e.target.value)}>
                          <option value="all">全部厂商</option>
                          {Array.from(new Set(BOARD_PRESETS.map((b) => b.chipVendor))).map((v) => (
                            <option value={v} key={v}>
                              {linuxVendorLabel(v)}
                            </option>
                          ))}
                        </select>
                      </label>
                    </div>
                    <table className="basic-table">
                      <thead>
                        <tr>
                          <th>板型 / 系列</th>
                          <th>厂商</th>
                          <th>目标存储</th>
                        </tr>
                      </thead>
                      <tbody>
                        {visible.map((b) => (
                          <tr className={b.id === boardId ? 'current' : ''} key={b.id}>
                            <td>
                              <label>
                                <input
                                  type="radio"
                                  name="basic-board"
                                  checked={b.id === boardId}
                                  onChange={() => changeBoard(b.id)}
                                />
                                {b.name}
                              </label>
                            </td>
                            <td>{linuxVendorLabel(b.chipVendor)}</td>
                            <td>{b.storage}</td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                    {!visible.length && <p role="status">没有匹配的板型。请更换关键词或厂商。</p>}
                  </section>
                  <section className="imager-board-detail">
                    <h3 aria-label={'已选板型 ' + board.name}>{board.name}</h3>
                    <div className="basic-drawing">
                      <BoardGlyph board={board} large />
                    </div>
                    {board.id === 'rk3588' && (
                      <label className="basic-field">
                        具体板型
                        <select
                          value={rkBoard}
                          onChange={(e) => {
                            setRkBoard(e.target.value);
                            setImagePath('');
                            setLoaderPath('');
                            setFiles(null);
                            setSelectedId(null);
                            clearCheck();
                          }}
                        >
                          <option value="">按板上标识选择…</option>
                          {RK_BOARDS.map((n) => (
                            <option key={n}>{n}</option>
                          ))}
                        </select>
                      </label>
                    )}
                    <PropsTable
                      items={[
                        ['芯片', board.chip],
                        ['目标存储', board.storage],
                        ['USB 模式', board.mode],
                        ['烧录路线', route],
                      ]}
                    />
                  </section>
                </div>
              </>
            )}
            {step === 2 && (
              <>
                <div className="basic-buttons">
                  <button
                    type="button"
                    aria-pressed={source === 'local'}
                    onClick={() => {
                      if (source !== 'local') {
                        setSource('local');
                        editFile('image', '');
                        setLoaderPath('');
                      }
                    }}
                  >
                    本地镜像
                  </button>
                  {board.id === 'f101' && (
                    <button
                      type="button"
                      aria-pressed={source === 'catalog'}
                      onClick={() => {
                        if (source !== 'catalog') {
                          editFile('image', '');
                          setLoaderPath('');
                          setSource('catalog');
                          void run('读取已保存设备…', readDevices);
                        }
                      }}
                    >
                      板型资源
                    </button>
                  )}
                </div>
                <div className="basic-columns">
                  <section>
                    {source === 'local' ? (
                      <>
                        <h3>本地文件</h3>
                        <div className="basic-drop" ref={dropZone}>
                          将镜像文件拖到此处，或选择电脑上的文件。
                        </div>
                        {picker('image')}
                        {board.needsLoader && picker('loader')}
                        {board.needsLoader && imagePath && !loaderPath && (
                          <p className="basic-note" id="basic-loader-hint" role="status">
                            请补齐 {boardName} 对应的 RAM Loader 后核对文件。
                          </p>
                        )}
                      </>
                    ) : (
                      <>
                        <h3>资源对应的设备</h3>
                        {targetPicker}
                        <button
                          type="button"
                          onClick={() => void run('读取已保存设备…', readDevices)}
                        >
                          刷新设备列表
                        </button>
                        {selected && (
                          <BasicCatalog
                            key={selected.boardUid}
                            device={selected}
                            onChoose={(c) => {
                              setCandidate(c);
                              setImagePath(c.imagePath);
                              setLoaderPath(c.loaderPath);
                              setFiles(null);
                              clearCheck();
                            }}
                          />
                        )}
                      </>
                    )}
                  </section>
                  <section>
                    <h3>镜像信息</h3>
                    <PropsTable
                      items={[
                        ['适用板型', boardName],
                        ['目标存储', board.storage],
                        ['镜像文件', imagePath || '待选择'],
                        ...(board.needsLoader
                          ? [['Loader', loaderPath || '必填 · 待选择'] as [string, string]]
                          : []),
                        [
                          '文件大小',
                          files
                            ? formatLinuxBytes(files.image.sizeBytes)
                            : candidate
                              ? formatLinuxBytes(candidate.manifest.image.sizeBytes)
                              : '读取后显示',
                        ],
                        [
                          'SHA-256',
                          files?.image.sha256 || candidate?.manifest.image.sha256 || '读取后显示',
                        ],
                      ]}
                    />
                  </section>
                </div>
              </>
            )}
            {step === 3 && (
              <div className="basic-columns">
                <section>
                  <h3>已保存的接口配置</h3>
                  {targetPicker}
                  <div className="basic-buttons">
                    <button type="button" onClick={() => void run('刷新设备列表…', readDevices)}>
                      刷新设备列表
                    </button>
                    <button type="button" onClick={onOpenAdvancedWorkbench}>
                      打开高级烧录
                    </button>
                  </div>
                  {loaded && !devices.length && (
                    <p role="status">
                      未找到此厂商的已保存单板。请在高级烧录中添加设备并绑定 USB 接口。
                    </p>
                  )}
                  {selected && (
                    <PropsTable
                      items={[
                        ['设备名称', selected.name],
                        ['烧录器', selected.flasherKind],
                        ['USB 绑定', formatUsbBinding(selected.usbBinding)],
                        ['调试串口', selected.serialPort || '未绑定'],
                      ]}
                    />
                  )}
                </section>
                <section>
                  <h3>检查前核对</h3>
                  <PropsTable
                    items={[
                      ['目标板型', boardName],
                      ['预期模式', board.mode],
                      ['目标存储', board.storage],
                      ['镜像', imagePath],
                      ['实际接口', '运行接口检查后显示'],
                    ]}
                  />
                </section>
              </div>
            )}
            {step === 4 && target && files && (
              <>
                <div className="basic-warning" role="note">
                  <strong>烧录将覆盖目标存储。</strong> 请核对物理端口、镜像指纹和需要保留的数据。
                </div>
                <div className="basic-columns">
                  <section>
                    <h3>接口与文件检查</h3>
                    <PropsTable
                      items={[
                        ['保存的单板', target.device.name],
                        ['物理接口', target.usb.key],
                        ['检测模式', target.usb.mode],
                        [
                          '设备标识',
                          target.usb.vid.toString(16).padStart(4, '0') +
                            ':' +
                            target.usb.pid.toString(16).padStart(4, '0'),
                        ],
                        ['镜像大小', formatLinuxBytes(files.image.sizeBytes)],
                        ['SHA-256', <code key="image">{files.image.sha256}</code>],
                        ...(files.loader
                          ? [
                              [
                                'Loader SHA-256',
                                <code key="loader">{files.loader.sha256}</code>,
                              ] as [string, React.ReactNode],
                            ]
                          : []),
                      ]}
                    />
                  </section>
                  <section>
                    <h3>本次写入参数</h3>
                    <PropsTable
                      items={[
                        ['目标', '' + boardName + ' · ' + board.storage],
                        ['镜像文件', imagePath],
                        ['烧录路线', route],
                        [
                          '擦除和校验',
                          usesSavedVendorRecipe
                            ? '使用已保存的厂商配置'
                            : board.route === 'mainline_spinor'
                              ? '按 SPI NOR 物理布局写入和回读'
                              : '完整擦除，启用校验',
                        ],
                        ['结束动作', usesSavedVendorRecipe ? '按已保存的设备配置' : '发送重启指令'],
                      ]}
                    />
                    <label className="basic-agree">
                      <input
                        type="checkbox"
                        checked={confirmed}
                        onChange={(e) => setConfirmed(e.target.checked)}
                      />
                      我已核对本次设备和镜像，并备份需要保留的数据。
                    </label>
                  </section>
                </div>
              </>
            )}
            {step === 5 && target && (
              <BasicTask
                task={task}
                target={target.device.name + ' · ' + boardName}
                usb={target.usb.key}
                path={imagePath}
              />
            )}
            {error && (
              <div role="alert" className="basic-warning">
                {error}
              </div>
            )}
          </fieldset>
        </div>
        <footer className="basic-actions">
          <span role="status">{busy || (running ? '任务正在运行' : statusTarget)}</span>
          <div>
            {step > 1 && step < 5 && (
              <button
                type="button"
                disabled={locked}
                onClick={() => goBack((step - 1) as BasicStep)}
              >
                上一步
              </button>
            )}
            {step === 1 && (
              <button
                className="basic-primary"
                disabled={
                  !visible.some((b) => b.id === boardId) || (board.id === 'rk3588' && !rkBoard)
                }
                onClick={() => {
                  clearCheck();
                  setStep(2);
                }}
              >
                下一步：选择镜像
              </button>
            )}
            {step === 2 && (
              <button
                className="basic-primary"
                disabled={
                  locked ||
                  !imagePath.trim() ||
                  (board.needsLoader && !loaderPath.trim()) ||
                  (source === 'catalog' && !candidate)
                }
                onClick={() => void run('读取并核对文件…', continueFiles)}
              >
                核对文件并继续
              </button>
            )}
            {step === 3 && (
              <button
                className="basic-primary"
                disabled={locked || !selected?.usbBinding}
                onClick={() => void run('检查绑定接口…', preflight)}
              >
                检查接口并继续
              </button>
            )}
            {step === 4 && (
              <button
                className="basic-primary"
                disabled={locked || !confirmed || !target}
                onClick={() => void run('复核并提交任务…', startFlash)}
              >
                开始写入
              </button>
            )}
            {step === 5 && (
              <button className="basic-primary" disabled={!terminal} onClick={retry}>
                {!terminal
                  ? '写入任务进行中…'
                  : task.status === 'success' && !candidate
                    ? '选择下一块设备'
                    : '重新检测设备'}
              </button>
            )}
          </div>
        </footer>
      </div>
    </section>
  );
};
