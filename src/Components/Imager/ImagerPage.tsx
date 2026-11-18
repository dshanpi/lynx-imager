import { useCallback, useEffect, useRef, useState } from 'react';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { open } from '@tauri-apps/plugin-dialog';
import {
  ArrowRight20Regular,
  CheckmarkCircle20Regular,
  ClipboardTaskListLtr24Regular,
  DeveloperBoard24Regular,
  Flash24Regular,
  Info20Regular,
  Layer24Regular,
  UsbPlug24Regular,
} from '@fluentui/react-icons';
import { invokeCommand, subscribeUsbHotplug, type WorkbenchDevice } from '../../Platform/IPC';
import type {
  FirmwareCandidate,
  FirmwareConfirmation,
  WorkbenchFlasherDevice,
} from '../../Platform/IPC/contracts/Workbench';
import { formatErrorMessage } from '../../Utils';
import { linuxVendorLabel } from '../Workbench/linuxWorkbenchModel';
import { ended, stateName, useBasicTask } from './BasicTask';
import { flashOptions, uniqueUsbTarget, type BasicRoute } from './BasicPlan';
import { V2BoardArt, V2BoardPicker, V2SystemPicker, type V2ImageSource } from './V2Catalog';
import { V2Interface } from './V2Interface';
import { V2Write } from './V2Write';
import './ImagerPage.css';

type BasicStep = 1 | 2 | 3 | 4;
const STEPS = ['选择板子', '选择系统', '设备连接', '烧写系统'] as const;
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

type FileIdentity = { sizeBytes: number; sha256: string };
type Files = { image: FileIdentity; loader: FileIdentity | null };
type Target = { device: WorkbenchDevice; usb: WorkbenchFlasherDevice };
type Preflight = Target & { files: Files; ticket: FirmwareConfirmation | null };
interface ImagerPageProps {
  active: boolean;
  onOpenAdvancedWorkbench: () => void;
  onOpenRecords?: () => void;
  onStatusChange?: (status: { target: string; task: string }) => void;
}
const baseName = (path: string) => path.split(/[\\/]/).pop() || path;
function useFingerprint(path: string, revision: number) {
  const [result, setResult] = useState<{
    path: string;
    revision: number;
    value: FileIdentity | null;
    error: string;
  } | null>(null);
  useEffect(() => {
    if (!path) return;
    let current = true;
    invokeCommand('imager_file_identity', { path })
      .then((value) => {
        if (current)
          setResult({
            path,
            revision,
            value: value.sizeBytes ? value : null,
            error: value.sizeBytes ? '' : '文件为空，请重新选择。',
          });
      })
      .catch((error) => {
        if (current) setResult({ path, revision, value: null, error: formatErrorMessage(error) });
      });
    return () => {
      current = false;
    };
  }, [path, revision]);
  return result?.path === path && result.revision === revision ? result : null;
}
function signature(device: WorkbenchDevice) {
  return JSON.stringify([
    device.id,
    device.boardUid,
    device.name,
    device.chipVendor,
    device.usbBinding,
    device.flasherKind,
    device.flasherPath,
    device.flasherArgs,
  ]);
}
async function identities(imagePath: string, loaderPath: string): Promise<Files> {
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
    throw new Error('该设备尚未绑定 USB 接口。请到「高级功能」的设备设置中绑定。');
  const found = await invokeCommand('workbench_scan_flasher_devices', {
    kind: device.flasherKind,
    path: device.flasherPath,
  });
  const usb = uniqueUsbTarget(device.usbBinding, device.flasherKind, found);
  return { device, usb };
}

function sameUsb(a: WorkbenchFlasherDevice, b: WorkbenchFlasherDevice) {
  return (
    a.key === b.key &&
    a.bus === b.bus &&
    a.port === b.port &&
    a.chip === b.chip &&
    a.mode === b.mode &&
    a.vid === b.vid &&
    a.pid === b.pid &&
    a.flasherKind === b.flasherKind
  );
}

export function ImagerPage({
  active,
  onOpenAdvancedWorkbench,
  onOpenRecords,
  onStatusChange,
}: ImagerPageProps) {
  const [step, setStep] = useState<BasicStep>(1);
  const [boardId, setBoardId] = useState<string | null>(null);
  const [rkBoard, setRkBoard] = useState('');
  const [source, setSource] = useState<V2ImageSource>('catalog');
  const [imagePath, setImagePath] = useState('');
  const [loaderPath, setLoaderPath] = useState('');
  const [revision, recheck] = useState(0);
  const [devices, setDevices] = useState<WorkbenchDevice[]>([]);
  const [catalogId, setCatalogId] = useState<number | null>(null);
  const [candidate, setCandidate] = useState<FirmwareCandidate | null>(null);
  const [selected, setSelected] = useState<Target | null>(null);
  const [target, setTarget] = useState<Preflight | null>(null);
  const [confirmed, setConfirmed] = useState(false);
  const [taskId, setTaskId] = useState('');
  const [count, setCount] = useState<number | null>(null);
  const [busy, setBusy] = useState('');
  const [error, setError] = useState('');
  const lock = useRef(false);
  const reviewingUsb = useRef<WorkbenchFlasherDevice | null>(null);
  const [watchReady, setWatchReady] = useState(false);
  const dropRef = useRef<HTMLDivElement>(null);
  const board = BOARD_PRESETS.find((b) => b.id === boardId);
  const boardName = (board?.id === 'rk3588' && rkBoard) || board?.name || '';
  const image = useFingerprint(imagePath, revision);
  const loader = useFingerprint(loaderPath, revision);
  const imageValid =
    !!image?.value && (!candidate || candidate.manifest.image.sha256 === image.value.sha256);
  const loaderValid =
    !!loader?.value && (!candidate || candidate.manifest.loader.sha256 === loader.value.sha256);
  const task = useBasicTask(taskId);
  const terminal = !!taskId && task.id === taskId && ended(task.status);
  const running = !!taskId && !terminal;
  const locked = !!busy || running;
  const hasBoard = !!board && (board.id !== 'rk3588' || !!rkBoard);
  const statusText = taskId ? stateName(task.status) : busy || (target ? '等待确认' : '就绪');
  const statusTone = terminal ? task.status : busy || running ? 'working' : undefined;
  const statusTarget = selected?.device.name || boardName || '未选择板子';
  const watchTarget = active && step >= 3 && !taskId;
  // Keep the same subscription from interface detection through the confirmation drawer.
  // A second board can reconnect to the same USB port with the same VID/PID.
  useEffect(() => {
    setWatchReady(false);
    if (!watchTarget) return;
    let current = true;
    let release: (() => void) | undefined;
    void subscribeUsbHotplug((event) => {
      const usb = reviewingUsb.current;
      if (
        !current ||
        !usb ||
        event.event !== 'left' ||
        event.vendorId !== usb.vid ||
        event.productId !== usb.pid ||
        (usb.bus > 0 && event.busId > 0 && usb.bus !== event.busId)
      )
        return;
      const port = /^(?:\d+\.)*(\d+)$/.exec(usb.port || '');
      if (port && event.port != null && Number(port[1]) !== event.port) return;
      reviewingUsb.current = null;
      setConfirmed(false);
      setTarget(null);
      setSelected(null);
      setCount(null);
      setStep(3);
      setError('目标 USB 已断开，请重新选择设备并预检。');
    })
      .then((stop) => {
        if (!current) stop();
        else {
          release = stop;
          setWatchReady(true);
        }
      })
      .catch((error) => {
        if (current) setError('无法监听设备连接变化：' + formatErrorMessage(error));
      });
    return () => {
      current = false;
      reviewingUsb.current = null;
      release?.();
    };
  }, [watchTarget]);
  useEffect(() => {
    onStatusChange?.({ target: statusTarget, task: statusText });
  }, [statusTarget, statusText, onStatusChange]);
  useEffect(() => {
    if (!active || step !== 2 || board?.id !== 'f101' || source !== 'catalog') return;
    let current = true;
    invokeCommand('workbench_list_devices', undefined, { timeoutMs: 8000 })
      .then((rows) => {
        if (current)
          setDevices(rows.filter((d) => d.id != null && d.chipVendor === board.chipVendor));
      })
      .catch((e) => {
        if (current) setError(formatErrorMessage(e));
      });
    return () => {
      current = false;
    };
  }, [active, board, step, source]);
  function invalidate() {
    reviewingUsb.current = null;
    setTarget(null);
    setConfirmed(false);
    setError('');
  }
  function clearFiles() {
    setImagePath('');
    setLoaderPath('');
    setCandidate(null);
  }
  function selectBoard(id: string) {
    if (boardId === id) return;
    invalidate();
    clearFiles();
    setBoardId(id);
    setRkBoard('');
    setSource('catalog');
    setDevices([]);
    setCatalogId(null);
    setSelected(null);
    setCount(null);
  }
  function selectVariant(name: string) {
    if (name === rkBoard) return;
    invalidate();
    clearFiles();
    setSelected(null);
    setRkBoard(name);
  }
  function changeSource(value: V2ImageSource) {
    if (value === source) return;
    invalidate();
    clearFiles();
    setSelected(null);
    setSource(value);
  }
  const loadImage = useCallback((path: string) => {
    setCandidate(null);
    setSource('local');
    setImagePath(path.trim());
    recheck((r) => r + 1);
    setTarget(null);
    setConfirmed(false);
    setSelected(null);
    setError('');
  }, []);
  function loadLoader(path: string) {
    if (candidate) return;
    setLoaderPath(path.trim());
    recheck((r) => r + 1);
    invalidate();
  }
  async function chooseFile(which: 'image' | 'loader') {
    const result = await open({
      multiple: false,
      directory: false,
      title: which === 'image' ? '选择本地镜像' : '选择匹配的 RAM Loader',
      filters:
        which === 'image'
          ? [
              {
                name: '系统镜像',
                extensions: ['img', 'bin', 'xz', 'gz', 'wic', 'dfu', 'tsv', 'uuu', 'iso'],
              },
              { name: '所有文件', extensions: ['*'] },
            ]
          : [{ name: 'RAM Loader', extensions: ['img', 'bin', 'fex'] }],
    });
    if (typeof result === 'string') {
      if (which === 'image') loadImage(result);
      else loadLoader(result);
    }
  }
  useEffect(() => {
    if (!active || step !== 2 || source !== 'local') return;
    let disposed = false;
    let stop: (() => void) | undefined;
    try {
      void getCurrentWebview()
        .onDragDropEvent((e) => {
          if (disposed || lock.current || e.payload.type !== 'drop' || e.payload.paths.length !== 1)
            return;
          const rect = dropRef.current?.getBoundingClientRect();
          if (!rect) return;
          const x = e.payload.position.x / window.devicePixelRatio,
            y = e.payload.position.y / window.devicePixelRatio;
          if (x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom)
            loadImage(e.payload.paths[0]);
        })
        .then((f) => {
          if (disposed) f();
          else stop = f;
        })
        .catch(() => {});
    } catch {
      /* The system file chooser is always available in the desktop application. */
    }
    return () => {
      disposed = true;
      stop?.();
    };
  }, [active, step, source, loadImage]);
  function selectCandidate(c: FirmwareCandidate) {
    invalidate();
    setSelected(null);
    setSource('catalog');
    setCandidate(c);
    setCatalogId(c.deviceId);
    setImagePath(c.imagePath);
    setLoaderPath(c.loaderPath);
    recheck((r) => r + 1);
  }
  const selectTarget = useCallback(
    (device: WorkbenchDevice | null, usb: WorkbenchFlasherDevice | null) => {
      if (lock.current) return;
      setSelected(device && usb ? { device, usb } : null);
      setTarget(null);
      setConfirmed(false);
      if (device && usb) setError('');
    },
    []
  );
  function back(to: BasicStep) {
    if (locked || taskId || to >= step) return;
    invalidate();
    setSelected(null);
    setCount(null);
    setStep(to);
  }
  async function run(label: string, fn: () => Promise<void>) {
    if (lock.current || running) return;
    lock.current = true;
    setBusy(label);
    setError('');
    try {
      await fn();
    } catch (e) {
      setError(formatErrorMessage(e));
      setConfirmed(false);
    } finally {
      setBusy('');
      lock.current = false;
    }
  }
  async function inspectFiles(): Promise<Files> {
    if (!image?.value || !board || (board.needsLoader && !loader?.value))
      throw new Error('请先核对镜像和所需的 Loader。');
    const fresh = await identities(imagePath, board.needsLoader ? loaderPath : '');
    if (
      !fresh.image.sizeBytes ||
      fresh.image.sha256 !== image.value.sha256 ||
      (board.needsLoader && fresh.loader?.sha256 !== loader?.value?.sha256)
    ) {
      recheck((r) => r + 1);
      throw new Error('文件内容已变化，请重新核对镜像与 Loader。');
    }
    if (
      candidate &&
      (candidate.manifest.image.sha256 !== fresh.image.sha256 ||
        candidate.manifest.loader.sha256 !== fresh.loader?.sha256)
    ) {
      throw new Error('文件与发布清单不符，请重新获取资源。');
    }
    return fresh;
  }
  async function preflight() {
    if (!selected?.device.id || !board || !watchReady) return;
    invalidate();
    const reviewing = selected.usb;
    reviewingUsb.current = reviewing;
    const files = await inspectFiles();
    const fresh = await checkTarget(selected.device);
    if (!sameUsb(selected.usb, fresh.usb)) {
      setSelected(null);
      throw new Error('实际接口已变化，请重新选择设备。');
    }
    if (
      candidate &&
      (candidate.deviceId !== fresh.device.id || candidate.boardUid !== fresh.device.boardUid)
    )
      throw new Error('当前设备与板型资源绑定的单板不一致。');
    const ticket = candidate
      ? await invokeCommand('workbench_firmware_prepare', {
          deviceId: selected.device.id,
          candidateId: candidate.id,
          postAction: 'reboot',
        })
      : null;
    if (reviewingUsb.current !== reviewing) throw new Error('目标 USB 连接已变化，请重新预检。');
    reviewingUsb.current = fresh.usb;
    setTarget({ ...fresh, files, ticket });
    setConfirmed(false);
    setStep(4);
  }
  async function startFlash() {
    if (!board || !confirmed || !target?.device.id || taskId) return;
    const deviceId = target.device.id;
    try {
      const files = await inspectFiles();
      if (
        files.image.sha256 !== target.files.image.sha256 ||
        files.loader?.sha256 !== target.files.loader?.sha256
      )
        throw new Error('确认后文件已变化，请重新预检。');
      const fresh = await checkTarget(target.device);
      if (!sameUsb(fresh.usb, target.usb))
        throw new Error('USB 设备标识或工作模式已变化，请重新检查接口。');
      if (!watchReady || reviewingUsb.current !== target.usb)
        throw new Error('目标 USB 连接已变化，请重新预检。');
      // The backend owns device transitions after the explicit start request.
      reviewingUsb.current = null;
      setConfirmed(false);
      const id = target.ticket
        ? await invokeCommand('workbench_firmware_start', {
            deviceId,
            confirmationId: target.ticket.id,
            confirmed: true,
          })
        : await invokeCommand('workbench_start_flash', {
            deviceId,
            firmwarePath: imagePath,
            options: flashOptions(board.route, board.chipVendor, loaderPath),
          });
      setTaskId(id);
    } catch (e) {
      const started =
        target.ticket &&
        /FIRMWARE_TASK_STARTED_RECORD_FAILED:([^:\s]+):DO_NOT_RETRY/.exec(formatErrorMessage(e));
      if (started) {
        setTaskId(started[1]);
        setError('任务已经启动，发布记录未完整保存。');
        return;
      }
      setSelected(null);
      setTarget(null);
      setConfirmed(false);
      setStep(3);
      throw e;
    }
  }
  function retry() {
    if (!terminal) return;
    setTaskId('');
    invalidate();
    setSelected(null);
    setCount(null);
    setStep(3);
  }
  const route =
    board?.route === 'mainline_spinor'
      ? '全志 SPI NOR 物理写入'
      : board?.route === 'rockchip_raw_emmc'
        ? 'Rockchip eMMC 完整镜像'
        : board?.chipVendor === 'allwinner'
          ? '全志厂家镜像'
          : '已保存的厂商烧录配置';
  const usesRecipe = board?.route === 'vendor' && board.chipVendor !== 'allwinner';
  const verify =
    board?.route === 'mainline_spinor'
      ? '独立物理回读校验'
      : usesRecipe
        ? '由当前厂商配置决定'
        : '启用后端写入校验';
  const imageError =
    image?.error || (candidate && image?.value && !imageValid ? '镜像 SHA-256 与清单不符。' : '');
  const enabled =
    step === 1
      ? hasBoard
      : step === 2
        ? imageValid && (source !== 'catalog' || !!candidate)
        : watchReady && !!selected && imageValid && (!board?.needsLoader || loaderValid);
  const stepDetails = [
    boardName,
    baseName(imagePath),
    selected?.usb.mode,
    taskId ? stateName(task.status) : '确认并执行烧录',
  ];
  const hints =
    step === 1
      ? ['选择具体的开发板型号。', '无需连接 USB 即可继续。', '板型决定可用镜像与烧录路线。']
      : [boardName || '未选择', baseName(imagePath) || '未选择', selected?.usb.mode || '未配置'];
  return (
    <section className="v2-basic" aria-label="烧录系统" data-step={step}>
      <aside className="v2-rail">
        <div className="v2-rail-title">
          <span className="v2-chip-icon">
            <DeveloperBoard24Regular />
            <Flash24Regular />
          </span>
          <div>
            <h2>烧录系统</h2>
            <p>四步完成系统烧录</p>
          </div>
        </div>
        <nav aria-label="烧录步骤">
          <ol>
            {STEPS.map((label, i) => {
              const n = (i + 1) as BasicStep;
              const done = n < step;
              return (
                <li key={label} data-state={done ? 'done' : n === step ? 'active' : 'pending'}>
                  <button
                    type="button"
                    id={'v2-step-' + n}
                    disabled={locked || !!taskId || n > step}
                    aria-current={n === step ? 'step' : undefined}
                    onClick={() => back(n)}
                  >
                    <span className="v2-step-number">{n}</span>
                    <span className="v2-step-label">
                      <strong>{label}</strong>
                      {step > 1 && !!stepDetails[i] && n <= step && <small>{stepDetails[i]}</small>}
                    </span>
                    {done && <CheckmarkCircle20Regular />}
                  </button>
                </li>
              );
            })}
          </ol>
        </nav>
        <section className="v2-rail-note">
          <h3>
            {step === 1 ? <Info20Regular /> : <ClipboardTaskListLtr24Regular />}
            {step === 1 ? '操作提示' : '当前任务'}
          </h3>
          {step === 1 ? (
            <ul>
              {hints.map((s) => (
                <li key={s}>{s}</li>
              ))}
            </ul>
          ) : (
            <dl>
              {['板型', '镜像', '接口'].map((t, i) => (
                <div key={t}>
                  <dt>{t}</dt>
                  <dd title={hints[i]}>{hints[i]}</dd>
                </div>
              ))}
            </dl>
          )}
        </section>
      </aside>
      <div className="v2-content">
        <fieldset className="v2-control-area" disabled={!!busy}>
          {step === 1 && (
            <V2BoardPicker
              boards={BOARD_PRESETS}
              selectedId={boardId}
              onSelect={selectBoard}
              rkBoard={rkBoard}
              onRkBoard={selectVariant}
            />
          )}
          {step === 2 && board && (
            <V2SystemPicker
              board={board}
              boardName={boardName}
              source={source}
              onSource={changeSource}
              imagePath={imagePath}
              identity={image?.value ?? null}
              reading={!!imagePath && !image}
              onBrowse={() => void run('选择镜像', () => chooseFile('image'))}
              onImagePath={loadImage}
              dropRef={dropRef}
              devices={devices}
              catalogDevice={devices.find((d) => d.id === catalogId)}
              onCatalogDevice={(id) => {
                setCatalogId(id);
                clearFiles();
                invalidate();
              }}
              onCandidate={selectCandidate}
              candidate={candidate}
              fileError={imageError}
            />
          )}
          {step === 3 && board && (
            <V2Interface
              board={board}
              boardName={boardName}
              imagePath={imagePath}
              imageSize={image?.value?.sizeBytes ?? null}
              imageSha256={image?.value?.sha256}
              imageVerified={!!candidate && imageValid}
              loaderPath={loaderPath}
              loaderReady={loaderValid}
              onChooseLoader={() => void run('选择 Loader', () => chooseFile('loader'))}
              onDropLoader={(path) => {
                if (!lock.current) loadLoader(path);
              }}
              selectedDeviceId={selected?.device.id ?? null}
              onTargetChange={selectTarget}
              onDetected={setCount}
              fixedDeviceId={candidate?.deviceId}
              active={active}
              onOpenAdvanced={onOpenAdvancedWorkbench}
            />
          )}
          {step === 4 && board && target && (
            <V2Write
              snapshot={{
                board: boardName,
                boardArt: <V2BoardArt id={board.id} name={boardName} />,
                chip: board.chip,
                vendor: linuxVendorLabel(board.chipVendor),
                storage: board.storage,
                route,
                imagePath,
                imageSize: target.files.image.sizeBytes,
                imageSha256: target.files.image.sha256,
                loaderPath,
                loaderSha256: target.files.loader?.sha256,
                device: target.device.name,
                usb:
                  target.usb.vid.toString(16).padStart(4, '0') +
                  ':' +
                  target.usb.pid.toString(16).padStart(4, '0') +
                  ' · ' +
                  target.usb.key,
                mode: target.usb.mode,
                verify,
                postAction: usesRecipe ? '由当前厂商配置决定' : '校验成功后发送重启指令',
              }}
              task={taskId ? task : undefined}
              confirmed={confirmed}
              busy={!!busy}
              onConfirmChange={setConfirmed}
              onStart={() => void run('复核并提交', startFlash)}
              onBack={() => back(3)}
              onRetry={retry}
              onOpenRecords={onOpenRecords}
            />
          )}
        </fieldset>
        {(error || (step === 3 && loader?.error)) && (
          <p role="alert" className="v2-engine-error">
            {error || loader?.error}
          </p>
        )}
      </div>
      <footer className="v2-foot">
        <div className="v2-status-facts" role="status">
          <span>
            <i aria-hidden="true" className={statusTone} />
            {statusText}
          </span>
          <span>
            <UsbPlug24Regular />
            {count === null ? 'USB 未检测' : '检测到 ' + count + ' 台'}
          </span>
          <span>
            <Layer24Regular />
            {step === 1 ? '当前任务：未创建' : boardName}
          </span>
        </div>
        {step < 4 && (
          <div className="v2-foot-actions">
            {step > 1 && (
              <button type="button" disabled={locked} onClick={() => back((step - 1) as BasicStep)}>
                ← 上一步
              </button>
            )}
            <button
              type="button"
              className="v2-primary"
              id={step === 3 ? 'v2-preflight' : undefined}
              disabled={locked || !enabled}
              onClick={() => {
                if (step === 3) {
                  void run('只读预检中', preflight);
                } else {
                  invalidate();
                  setStep((step + 1) as BasicStep);
                }
              }}
            >
              {step === 1 ? '下一步：选择系统' : step === 2 ? '下一步：设备连接' : '检查并烧录'}{' '}
              <ArrowRight20Regular />
            </button>
          </div>
        )}
      </footer>
    </section>
  );
}
