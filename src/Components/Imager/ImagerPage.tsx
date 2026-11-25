import { HostCardWriter } from './HostCardWriter';
import { V2BoardPicker } from './TargetPicker';
import {
  allwinnerEmmcPreset,
  allwinnerModels,
  rockchipModels,
  profiles,
  type TargetChoice,
} from './targetCatalog';
import { useCallback, useEffect, useRef, useState } from 'react';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { open } from '@tauri-apps/plugin-dialog';
import {
  ArrowRight20Regular,
  CheckmarkCircle20Regular,
  Layer24Regular,
  UsbPlug24Regular,
} from '@fluentui/react-icons';
import { invokeCommand, subscribeUsbHotplug, type WorkbenchDevice } from '../../Platform/IPC';
import type {
  StagedFirmware,
  FirmwareConfirmation,
  WorkbenchFlasherDevice,
} from '../../Platform/IPC/contracts/Workbench';
import { formatErrorMessage } from '../../Utils';
import { linuxVendorLabel } from '../Workbench/linuxWorkbenchModel';
import { ended, stateName, useBasicTask } from './BasicTask';
import { flashOptions, uniqueUsbTarget, type BasicRoute } from './BasicPlan';
import { imagePickerDirectory, V2SystemPicker, type V2ImageSource } from './V2Catalog';
import { V2Interface } from './V2Interface';
import { V2Write, WritePreflightFailure, type PreflightContext } from './V2Write';
import { VendorRoutePanel, VendorTargetPanel } from './VendorRoutePanel';
import { recheckImageEvidence, type ImageReviewEvidence } from './imageReviewEvidence';
import {
  defaultVendorSelection,
  inspectVendorImage,
  vendorNeedsLoader,
  vendorGate,
  vendorFlashOptions,
  type VendorInspection,
  type VendorSelection,
} from './vendorPlan';
import './ImagerPage.css';
import './FlashWorkspace.css';

type BasicStep = 1 | 2 | 3 | 4;
class PreSubmitError extends Error {}
const STEPS = ['选择烧录目标', '选择系统', '设备连接', '烧写系统'] as const;
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
  ...allwinnerModels.map((model): BoardPreset => ({
    id: allwinnerEmmcPreset(model),
    name: model === 'T153' ? 'T153 Omnigate' : model,
    chipVendor: 'allwinner',
    chip: '全志 · ' + model,
    storage: 'eMMC',
    mode: 'FEL',
    route: 'mainline_emmc',
    needsLoader: true,
  })),
  ...rockchipModels.map((model): BoardPreset => ({
    id: model.toLowerCase(),
    name: model,
    chipVendor: 'rockchip',
    chip: '瑞芯微 · ' + model,
    storage: 'eMMC',
    mode: 'Maskrom / Loader',
    route: 'rockchip_raw_emmc',
    needsLoader: true,
    loaderLabel: 'Rockchip Loader',
  })),
  {
    id: 'nxp-imx6ull',
    name: 'i.MX6ULL',
    chipVendor: 'nxp',
    chip: 'NXP · i.MX6ULL',
    storage: 'eMMC / SD',
    mode: 'SDP / Fastboot',
    route: 'vendor',
    needsLoader: false,
  },

  {
    id: 'nxp-imx',
    name: 'i.MX 系列',
    chipVendor: 'nxp',
    chip: 'NXP · i.MX',
    storage: 'eMMC / SD',
    mode: 'SDP / Fastboot',
    route: 'vendor',
    needsLoader: false,
  },
  {
    id: 'stm32mp157',
    name: 'STM32MP157',
    chipVendor: 'st',
    chip: 'ST · STM32MP157',
    storage: 'eMMC / SD',
    mode: 'USB DFU / DfuSe',
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
type Preflight = Target & {
  files: Files;
  ticket: FirmwareConfirmation | null;
  vendorEvidence: string;
};
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
  let device = selected;
  if (selected.id != null && !selected.boardUid?.startsWith('imager-usb:')) {
    const devices = await invokeCommand('workbench_list_devices', undefined);
    const saved = devices.find((d) => d.id === selected.id);
    if (!saved || signature(saved) !== signature(selected))
      throw new Error('设备配置已变化，请重新选择端口。');
    device = saved;
  }
  const found = await invokeCommand('workbench_scan_flasher_devices', {
    kind: device.flasherKind,
    path: device.flasherPath,
  });
  const usb = uniqueUsbTarget(device.usbBinding, device.flasherKind, found);
  return { device, usb };
}
async function resolveFlashTarget(device: WorkbenchDevice) {
  if (device.id != null) return device;
  const { name, chipVendor, flasherKind, flasherPath, usbBinding } = device;
  return invokeCommand('imager_resolve_flash_target', {
    name,
    chipVendor,
    flasherKind,
    flasherPath,
    usbBinding,
  });
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
  const [choice, setChoice] = useState<TargetChoice | null>(null);
  const [boardVariant, setBoardVariant] = useState('');
  const [routeOverride, setRouteOverride] = useState<BasicRoute | null>(null);
  const [selection, setSelection] = useState<VendorSelection | null>(null);
  const [reviewReady, setReviewReady] = useState(false);
  const reviewEvidence = useRef<ImageReviewEvidence | null>(null);
  const observeReview = useCallback((ready: boolean, evidence?: ImageReviewEvidence) => {
    reviewEvidence.current = ready ? (evidence ?? null) : null;
    setReviewReady(ready);
  }, []);
  const [interfaceRevision, refreshInterface] = useState(0);
  const [source, setSource] = useState<V2ImageSource>('local');
  const [imagePath, setImagePath] = useState('');
  const [loaderPath, setLoaderPath] = useState('');
  const [revision, recheck] = useState(0);
  const [candidate, setCandidate] = useState<StagedFirmware | null>(null);
  const [selected, setSelected] = useState<Target | null>(null);
  const [target, setTarget] = useState<Preflight | null>(null);
  const [confirmed, setConfirmed] = useState(false);
  const [taskId, setTaskId] = useState('');
  const [count, setCount] = useState<number | null>(null);
  const [busy, setBusy] = useState('');
  const [error, setError] = useState('');
  const [failedPreflight, setFailedPreflight] = useState<PreflightContext | null>(null);
  const reviewedContext = useRef<PreflightContext | null>(null);
  const lock = useRef(false);
  const reviewingUsb = useRef<WorkbenchFlasherDevice | null>(null);
  const [watchReady, setWatchReady] = useState(false);
  const [watchError, setWatchError] = useState('');
  const dropRef = useRef<HTMLDivElement>(null);
  const preset = BOARD_PRESETS.find((b) => b.id === boardId);
  const routeKey = selection
    ? selection.route === 'rk-emmc'
      ? 'rockchip_raw_emmc'
      : 'vendor'
    : (routeOverride ?? preset?.route);
  const board =
    preset && routeKey
      ? {
          ...preset,
          aliases:
            choice && preset.id === profiles[choice.model]?.preset
              ? profiles[choice.model]?.aliases
              : undefined,
          route: routeKey,
          needsLoader: selection ? vendorNeedsLoader(selection.route) : routeKey !== 'vendor',
          storage: choice?.medium ?? preset.storage,
        }
      : undefined;
  const boardName =
    boardVariant.trim() || (board?.chipVendor === 'allwinner' ? board.name : choice?.model) || '';
  const vendorRoute = choice?.reader ? undefined : selection?.route;
  useEffect(() => {
    if (
      vendorRoute !== 'rk-emmc' ||
      choice?.model !== 'RK3576' ||
      boardVariant !== 'DshanPI-A1' ||
      step !== 2 ||
      loaderPath
    )
      return;
    let current = true;
    invokeCommand('imager_board_loader', { board: boardVariant })
      .then((path) => {
        if (current) setLoaderPath((previous) => previous || path);
      })
      .catch((e) => {
        if (current) setError(formatErrorMessage(e));
      });
    return () => {
      current = false;
    };
  }, [vendorRoute, choice?.model, boardVariant, step, loaderPath]);
  const [inspected, setInspected] = useState<{
    key: string;
    value: VendorInspection | null;
    error: string;
  } | null>(null);
  const inspectionKey = JSON.stringify([vendorRoute, imagePath, revision]);
  useEffect(() => {
    if (!vendorRoute || !imagePath) return;
    let current = true;
    inspectVendorImage(vendorRoute, imagePath)
      .then((value) => {
        if (current) setInspected({ key: inspectionKey, value, error: '' });
      })
      .catch((error) => {
        if (current)
          setInspected({ key: inspectionKey, value: null, error: formatErrorMessage(error) });
      });
    return () => {
      current = false;
    };
  }, [vendorRoute, imagePath, inspectionKey]);
  const inspection = inspected?.key === inspectionKey ? inspected.value : null;
  const inspectionError = inspected?.key === inspectionKey ? inspected.error : '';
  const image = useFingerprint(choice?.reader ? '' : imagePath, revision);
  const loader = useFingerprint(loaderPath, revision);
  const imageValid =
    !!image?.value && (!candidate || candidate.manifest.image.sha256 === image.value.sha256);
  const loaderValid =
    !!loader?.value && (!candidate || candidate.manifest.loader.sha256 === loader.value.sha256);
  const task = useBasicTask(taskId);
  const terminal = !!taskId && task.id === taskId && ended(task.status);
  const running = !!taskId && !terminal;
  const locked = !!busy || running;
  const statusText = taskId
    ? stateName(task.status)
    : busy || (failedPreflight ? '未提交烧录' : target ? '等待确认' : '就绪');
  const statusTone = terminal
    ? task.status
    : failedPreflight
      ? 'failed'
      : busy || running
        ? 'working'
        : undefined;
  const statusTarget =
    (choice?.reader ? choice.model : selected?.device.name || boardName) || '未选择板子';
  const watchTarget = active && !choice?.reader && step >= 3 && !taskId;
  // Keep the same subscription from interface detection through the confirmation drawer.
  // A second board can reconnect to the same USB port with the same VID/PID.
  useEffect(() => {
    setWatchReady(false);
    setWatchError('');
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
      setFailedPreflight(reviewedContext.current);
    })
      .then((stop) => {
        if (!current) stop();
        else {
          release = stop;
          setWatchReady(true);
        }
      })
      .catch((error) => {
        if (current) setWatchError('无法监听设备连接变化：' + formatErrorMessage(error));
      });
    return () => {
      current = false;
      reviewingUsb.current = null;
      release?.();
    };
  }, [watchTarget, interfaceRevision]);
  useEffect(() => {
    onStatusChange?.({ target: statusTarget, task: statusText });
  }, [statusTarget, statusText, onStatusChange]);
  function invalidate() {
    reviewingUsb.current = null;
    reviewedContext.current = null;
    setTarget(null);
    setConfirmed(false);
    setError('');
    setFailedPreflight(null);
  }
  function clearFiles() {
    setReviewReady(false);
    setImagePath('');
    setLoaderPath('');
    setCandidate(null);
  }
  function chooseHardware(next: TargetChoice | null) {
    if (
      choice?.model === next?.model &&
      choice?.medium === next?.medium &&
      !!choice?.reader === !!next?.reader
    )
      return;
    const id =
      next?.medium === 'eMMC' && allwinnerModels.includes(next.model)
        ? allwinnerEmmcPreset(next.model)
        : next
          ? (profiles[next.model]?.preset ?? null)
          : null;
    setChoice(next);
    invalidate();
    clearFiles();
    setBoardId(id);
    const vendor = BOARD_PRESETS.find((p) => p.id === id)?.chipVendor;
    setSelection(
      !next?.reader && vendor && vendor !== 'allwinner' ? defaultVendorSelection(vendor) : null
    );
    setRouteOverride(null);
    setBoardVariant('');
    setSource('local');
    setSelected(null);
    setCount(null);
  }
  function selectVariant(name: string) {
    if (name === boardVariant) return;
    invalidate();
    clearFiles();
    setSelected(null);
    setBoardVariant(name);
  }
  function changeSource(value: V2ImageSource) {
    if (value === source) return;
    invalidate();
    if (value === 'catalog' || source === 'catalog') clearFiles();
    setSelected(null);
    setSource(value);
  }
  const loadImage = useCallback((path: string) => {
    setReviewReady(false);
    setCandidate(null);
    setSource('local');
    setImagePath(path.trim());
    recheck((r) => r + 1);
    setTarget(null);
    setConfirmed(false);
    setSelected(null);
    setError('');
  }, []);
  const restoreFiles = useCallback(
    (image: string, loader: string) => {
      loadImage(image);
      setLoaderPath(loader);
    },
    [loadImage]
  );
  function loadLoader(path: string) {
    if (candidate) return;
    setLoaderPath(path.trim());
    setReviewReady(false);
    recheck((r) => r + 1);
    invalidate();
  }
  function changeRoute(next: BasicRoute) {
    if (candidate || next === routeKey) return;
    setRouteOverride(next);
    if (source === 'catalog' && next !== 'mainline_spinor') setSource('local');
    clearFiles();
    setSelected(null);
    invalidate();
  }
  function useImageAsLoader(next: BasicRoute) {
    if (candidate || !imagePath) return;
    setRouteOverride(next);
    setLoaderPath(imagePath);
    loadImage('');
  }
  function changeVendor(next: VendorSelection) {
    if (selection?.route !== next.route) {
      clearFiles();
      setSelected(null);
    }
    setSelection(next);
    setTarget(null);
    setConfirmed(false);
    setError('');
  }
  async function chooseFile(which: 'image' | 'loader') {
    const result = await open({
      multiple: false,
      directory: false,
      defaultPath: imagePickerDirectory((which === 'loader' && loaderPath) || imagePath),
      title: which === 'image' ? '选择本地镜像' : '选择匹配的 RAM Loader',
      filters:
        which === 'image'
          ? [
              {
                name: '系统镜像',
                extensions: ['img', 'bin', 'xz', 'gz', 'wic', 'dfu', 'tsv', 'uuu', 'lst', 'kdimg'],
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
    if (!active || choice?.reader || step !== 2 || source !== 'local') return;
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
  }, [active, choice?.reader, step, source, loadImage]);
  function selectCandidate(c: StagedFirmware) {
    invalidate();
    setSelected(null);
    setSource('catalog');
    setRouteOverride('mainline_spinor');
    setReviewReady(false);
    setCandidate(c);
    setImagePath(c.imagePath);
    setLoaderPath(c.loaderPath);
    recheck((r) => r + 1);
  }
  const selectTarget = useCallback(
    (device: WorkbenchDevice | null, usb: WorkbenchFlasherDevice | null) => {
      if (lock.current) return;
      setSelection((previous) =>
        previous?.targetKey && previous.targetKey !== usb?.key
          ? { ...previous, targetKey: '', medium: '', alt: '', address: '', eraseConfirmed: false }
          : previous
      );
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
    setFailedPreflight(null);
    try {
      await fn();
    } catch (e) {
      if (label === '只读预检中' || e instanceof PreSubmitError) {
        reviewingUsb.current = null;
        setFailedPreflight(reviewedContext.current);
        setSelected(null);
        setCount(null);
      }
      setError(formatErrorMessage(e));
      setConfirmed(false);
    } finally {
      setBusy('');
      lock.current = false;
    }
  }
  async function inspectFiles(): Promise<Files> {
    if (!reviewReady || !image?.value || !board || (board.needsLoader && !loader?.value))
      throw new Error('请先核对镜像和所需的 Loader。');
    const fresh = await identities(imagePath, board.needsLoader ? loaderPath : '');
    await recheckImageEvidence(reviewEvidence.current);
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
  async function checkVendor(targetUsb: WorkbenchFlasherDevice, profile: WorkbenchDevice) {
    if (!selection) return '';
    const fresh = await inspectVendorImage(selection.route, imagePath);
    const reason = vendorGate(selection, fresh, targetUsb, 'target', profile, choice?.medium);
    if (reason) throw new Error(reason);
    const signature = JSON.stringify(fresh);
    if (signature !== JSON.stringify(inspection))
      throw new Error('厂商镜像或依赖文件已变化，请返回选择系统并重新分析。');
    return signature;
  }
  async function preflight() {
    if (!selected || !board || !watchReady) return;
    invalidate();
    const reviewing = selected.usb;
    reviewedContext.current = {
      board: boardName,
      device: selected.device.name,
      route,
      storage: board.storage,
      imagePath,
      loaderPath: board.needsLoader ? loaderPath : '',
      usb: reviewing.key,
      usbId: [reviewing.vid, reviewing.pid].map((v) => v.toString(16).padStart(4, '0')).join(':'),
      mode: reviewing.mode,
    };
    reviewingUsb.current = reviewing;
    const files = await inspectFiles();
    const fresh = await checkTarget(selected.device);
    const vendorEvidence = await checkVendor(fresh.usb, fresh.device);
    if (!sameUsb(selected.usb, fresh.usb)) {
      setSelected(null);
      throw new Error('实际接口已变化，请重新选择设备。');
    }
    if (reviewingUsb.current !== reviewing) throw new Error('目标 USB 连接已变化，请重新选择。');
    fresh.device = await resolveFlashTarget(fresh.device);
    if (fresh.device.id == null) throw new Error('无法准备本次烧录目标。');
    let ticket: FirmwareConfirmation | null = null;
    if (candidate) {
      const bound = await invokeCommand(
        'workbench_firmware_download',
        {
          deviceId: fresh.device.id,
          request: {
            releaseId: candidate.releaseId,
            indexSha256: candidate.indexSha256,
            applicationId: candidate.manifest.applicationId,
            version: candidate.manifest.version,
            channel: 'stable',
            loaderPath,
          },
        },
        { timeoutMs: 190000 }
      );
      if (
        bound.deviceId !== fresh.device.id ||
        bound.boardUid !== fresh.device.boardUid ||
        bound.manifest.image.sha256 !== files.image.sha256 ||
        bound.manifest.loader.sha256 !== files.loader?.sha256 ||
        reviewingUsb.current !== reviewing
      )
        throw new Error('设备或发布文件已变化，请重新预检。');
      ticket = await invokeCommand('workbench_firmware_prepare', {
        deviceId: fresh.device.id,
        candidateId: bound.id,
        postAction: 'reboot',
      });
    }
    if (reviewingUsb.current !== reviewing) throw new Error('目标 USB 连接已变化，请重新预检。');
    reviewingUsb.current = fresh.usb;
    setTarget({ ...fresh, files, ticket, vendorEvidence });
    setConfirmed(false);
    setStep(4);
  }
  async function startFlash() {
    if (!board || !confirmed || !target?.device.id || taskId) return;
    const deviceId = target.device.id;
    let submitted = false;
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
      if ((await checkVendor(fresh.usb, fresh.device)) !== target.vendorEvidence)
        throw new Error('厂商目标或镜像依赖已变化，请重新预检。');
      if (!watchReady || reviewingUsb.current !== target.usb)
        throw new Error('目标 USB 连接已变化，请重新预检。');
      // The backend owns device transitions after the explicit start request.
      reviewingUsb.current = null;
      setConfirmed(false);
      submitted = true;
      const id = target.ticket
        ? await invokeCommand('workbench_firmware_start', {
            deviceId,
            confirmationId: target.ticket.id,
            confirmed: true,
          })
        : await invokeCommand('workbench_start_flash', {
            deviceId,
            firmwarePath: imagePath,
            options: selection
              ? vendorFlashOptions(selection, loaderPath, inspection)
              : flashOptions(board.route, board.chipVendor, loaderPath),
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
      if (!submitted) throw new PreSubmitError(formatErrorMessage(e));
      throw new Error('提交未取得任务编号，请先在任务记录核对设备状态。' + formatErrorMessage(e));
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
  const vendorLabels: Record<string, string> = {
    'rk-update': '瑞芯微厂商升级包',
    'rk-emmc': '瑞芯微主线 eMMC 整盘',
    'nxp-uuu': 'NXP UUU 脚本',
    'nxp-configured': 'NXP 板级配置',
    'st-dfuse': 'STM32 DfuSe 容器',
    'st-raw': 'STM32 RAW',
    'k230-kdimage': 'K230 分区包',
    'k230-raw': 'K230 RAW',
  };
  const route = selection
    ? vendorLabels[selection.route]
    : (
        {
          mainline_spinor: '全志 SPI NOR 物理写入',
          mainline_emmc: '全志主线 eMMC',
          mainline_nand: '全志主线 NAND 组件包',
          vendor: '全志原厂系统',
        } as Record<string, string>
      )[routeKey || 'vendor'];
  const usesRecipe = board?.route === 'vendor' && board.chipVendor !== 'allwinner';
  const verify =
    board?.route === 'mainline_spinor'
      ? '独立物理回读校验'
      : usesRecipe
        ? '由当前厂商配置决定'
        : '启用后端写入校验';
  const imageError =
    inspectionError ||
    image?.error ||
    (candidate && image?.value && !imageValid ? '镜像 SHA-256 与清单不符。' : '');
  const vendorFileIssue = selection && inspection ? vendorGate(selection, inspection) : '';
  const targetIssue =
    step === 3 && selection && selected
      ? vendorGate(selection, inspection, selected.usb, 'target', selected.device, choice?.medium)
      : '';
  const filesReady =
    reviewReady &&
    imageValid &&
    (!board?.needsLoader || loaderValid) &&
    (!selection || (!!inspection && !vendorFileIssue));
  const cardReader = !!choice?.reader;
  const enabled =
    step === 1
      ? !!choice && (!!board || cardReader)
      : step === 2
        ? filesReady && (source === 'local' || (source === 'catalog' && !!candidate))
        : watchReady && !!selected && filesReady && !targetIssue;
  const stepDetails = [
    choice?.reader ? choice.model : boardName,
    baseName(imagePath),
    selected?.usb.mode,
    taskId ? stateName(task.status) : cardReader ? '写入与回读校验' : '确认并执行烧录',
  ];
  return (
    <section className="v2-basic" aria-label="烧录系统" data-step={step}>
      <aside className="v2-rail">
        <div className="v2-rail-title">
          <h2>烧录系统</h2>
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
                    <span className="v2-step-number">
                      {done ? <CheckmarkCircle20Regular /> : n}
                    </span>
                    <span className="v2-step-label">
                      <strong>
                        {cardReader && n === 3
                          ? '选择读卡器'
                          : cardReader && n === 4
                            ? '写入存储卡'
                            : label}
                      </strong>
                      {step > 1 && !!stepDetails[i] && n <= step && <small>{stepDetails[i]}</small>}
                    </span>
                  </button>
                </li>
              );
            })}
          </ol>
        </nav>
        <details className="r3-help">
          <summary>使用帮助</summary>
          <p>
            {
              [
                '按板上丝印选定型号，这一步可以离线完成。',
                cardReader
                  ? '选择要写到存储卡的整盘镜像。'
                  : '先选系统文件。使用独立 RAM 引导的路线还需要配套 Loader。',
                cardReader
                  ? '插入读卡器，按磁盘名称和容量选择目标卡。'
                  : '连接 USB 下载口，按实际接线选择物理端口；调试串口用于观察启动。',
                cardReader
                  ? '核对镜像与目标卡，写入后将自动回读校验。'
                  : '检查实际写入范围。结果页分别记录写入、校验、重启和启动证据。',
              ][step - 1]
            }
          </p>
        </details>
      </aside>
      <div className="v2-content">
        <fieldset className="v2-control-area" disabled={!!busy}>
          {step === 1 && (
            <V2BoardPicker
              active={active}
              choice={choice}
              onChoice={chooseHardware}
              boardName={boardVariant}
              onBoardName={selectVariant}
            />
          )}
          {cardReader && step >= 2 && choice && (
            <HostCardWriter
              key={choice.model}
              model={choice.model}
              step={step}
              onStep={(n) => {
                setError('');
                setStep(n);
              }}
              run={run}
              onPath={setImagePath}
              busy={busy}
            />
          )}
          {step === 2 && !cardReader && board && (
            <V2SystemPicker
              memoryScope={JSON.stringify([
                choice?.model,
                board.storage,
                boardName,
                vendorRoute ?? board.route,
              ])}
              onRestore={restoreFiles}
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
              onCandidate={selectCandidate}
              candidate={candidate}
              fileError={imageError}
              route={board.route}
              onRoute={changeRoute}
              onUseImageAsLoader={useImageAsLoader}
              loaderPath={loaderPath}
              loaderIdentity={loader?.value ?? null}
              loaderError={loader?.error}
              onChooseLoader={() => void run('选择引导来源', () => chooseFile('loader'))}
              onReviewReady={observeReview}
              vendorTitle={selection ? route : undefined}
              vendorControls={
                selection && (
                  <VendorRoutePanel
                    dropRef={dropRef}
                    boardName={boardName}
                    mode={board.mode}
                    vendor={board.chipVendor}
                    selection={selection}
                    inspection={inspection}
                    fileIssue={vendorFileIssue}
                    loaderError={loader?.error}
                    onChange={changeVendor}
                    imagePath={imagePath}
                    loaderPath={loaderPath}
                    onChooseImage={() => void run('选择镜像', () => chooseFile('image'))}
                    onChooseLoader={() => void run('选择 Loader', () => chooseFile('loader'))}
                  />
                )
              }
            />
          )}
          {step === 3 && failedPreflight && (
            <WritePreflightFailure
              context={failedPreflight}
              message={error}
              onReconnect={() => {
                invalidate();
                setSelected(null);
                setCount(null);
                refreshInterface((n) => n + 1);
              }}
            />
          )}
          {step === 3 && !cardReader && board && !failedPreflight && (
            <V2Interface
              key={interfaceRevision}
              routeLabel={route}
              vendorTarget={
                selection &&
                selected && (
                  <VendorTargetPanel
                    targetMedium={choice?.medium}
                    selection={selection}
                    inspection={inspection}
                    usb={selected.usb}
                    profile={selected.device}
                    onChange={changeVendor}
                  />
                )
              }
              board={board}
              boardName={boardName}
              busy={locked}
              onTargetChange={selectTarget}
              onDetected={setCount}
              active={active}
              onOpenAdvanced={onOpenAdvancedWorkbench}
            />
          )}
          {step === 4 && !cardReader && board && target && (
            <V2Write
              snapshot={{
                board: boardName,
                deviceId: target.device.id ?? undefined,
                candidateId: target.ticket?.candidateId,
                eraseEmmcUserArea: selection?.route === 'rk-emmc',
                targetDetails: selection
                  ? [
                      ...(selection.medium
                        ? [['目标介质', selection.medium] as [string, string]]
                        : []),
                      ...(selection.alt !== ''
                        ? [['DFU Alternate', selection.alt] as [string, string]]
                        : []),
                      ...(selection.address
                        ? [['目标地址', selection.address] as [string, string]]
                        : []),
                      ...(inspection ? [['输入格式', inspection.format] as [string, string]] : []),
                    ]
                  : undefined,
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
        {targetIssue && !failedPreflight && (
          <p role="status" className="vi-help">
            {targetIssue}
          </p>
        )}
        {!failedPreflight && (error || (step === 3 && (watchError || loader?.error))) && (
          <p role="alert" className="v2-engine-error">
            {error || watchError || loader?.error}
            {step === 3 && !cardReader && (
              <button
                type="button"
                onClick={() => {
                  invalidate();
                  setSelected(null);
                  refreshInterface((n) => n + 1);
                }}
              >
                重新检测设备
              </button>
            )}
          </p>
        )}
      </div>
      {step < 4 && (!cardReader || step === 1) && !failedPreflight && (
        <div className="v2-foot-actions">
          <p className="flash-next-hint">
            {step === 1
              ? choice
                ? choice.model + ' / ' + (choice.medium === 'SD' ? 'TF / SD' : choice.medium)
                : '请选择芯片型号和目标存储'
              : step === 2
                ? '镜像及所需引导文件就绪后继续'
                : '下一步只读预检，不会立即写入'}
          </p>
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
            {step === 1 ? '下一步：选择系统' : step === 2 ? '下一步：设备连接' : '下一步：确认烧写'}{' '}
            <ArrowRight20Regular />
          </button>
        </div>
      )}
      <footer className="v2-foot">
        <div className="v2-status-facts" role="status">
          <span>
            <i aria-hidden="true" className={statusTone} />
            {statusText}
          </span>
          <span>
            <UsbPlug24Regular />
            {cardReader ? '电脑读卡器' : count === null ? 'USB 未检测' : '检测到 ' + count + ' 台'}
          </span>
          <span>
            <Layer24Regular />
            {step === 1
              ? choice
                ? choice.model + ' / ' + choice.medium
                : '选择目标时无需连接 USB'
              : cardReader
                ? choice?.model + ' / TF / SD'
                : boardName}
          </span>
        </div>
      </footer>
    </section>
  );
}
