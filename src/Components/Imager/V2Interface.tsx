import { useCallback, useEffect, useRef, useState } from 'react';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { LynxIcon, type LynxIconName } from '../../CoreUI';
import { invokeCommand, type WorkbenchDevice } from '../../Platform/IPC';
import { subscribeUsbHotplug } from '../../Platform/IPC/Events';
import type { WorkbenchFlasherDevice } from '../../Platform/IPC/contracts/Workbench';
import { formatErrorMessage } from '../../Utils';
import {
  EMPTY_LINUX_DEVICE,
  LINUX_CHIP_VENDORS,
  formatLinuxBytes,
} from '../Workbench/linuxWorkbenchModel';
import { uniqueUsbTarget } from './BasicPlan';
import { V2BoardArt } from './V2Catalog';
import './V2Interface.css';

export interface V2InterfaceProps {
  board: {
    id: string;
    name: string;
    chipVendor: WorkbenchDevice['chipVendor'];
    storage: string;
    mode: string;
    needsLoader: boolean;
    loaderLabel?: string;
  };
  boardName: string;
  imagePath: string;
  imageSize: number | null;
  imageVerified: boolean;
  imageSha256?: string;
  loaderPath: string;
  loaderReady: boolean;
  onChooseLoader: () => void;
  onDropLoader: (path: string) => void;
  selectedDeviceId: number | null;
  fixedDeviceId?: number | null;
  onTargetChange: (profile: WorkbenchDevice | null, usb: WorkbenchFlasherDevice | null) => void;
  onOpenAdvanced: () => void;
  onDetected?: (count: number | null) => void;
  active?: boolean;
}

const fileName = (path: string) => path.split(/[\\/]/).pop() || path;
const usbId = (usb: WorkbenchFlasherDevice) =>
  [usb.vid, usb.pid].map((n) => n.toString(16).padStart(4, '0')).join(':');
const usbLocation = (usb: WorkbenchFlasherDevice) => `USB ${usbId(usb)} · ${usb.key}`;
const modeLabel = (mode: string) =>
  ({ 'fel/fes': 'FEL / FES', loader: 'Loader', maskrom: 'Maskrom' })[mode.toLowerCase()] ?? mode;
const scannerPath = (profile: WorkbenchDevice) =>
  profile.flasherPath?.trim() ? profile.flasherPath : '';
const matches = (
  profile: WorkbenchDevice,
  usb: WorkbenchFlasherDevice,
  found: WorkbenchFlasherDevice[]
) => {
  if (!profile.usbBinding || profile.id == null) return false;
  try {
    return uniqueUsbTarget(profile.usbBinding, profile.flasherKind, found).key === usb.key;
  } catch {
    return false;
  }
};

function StageStrip({ board, image }: { board: string; image: string }) {
  const names = ['选择板子', '选择系统', '配置接口', '烧写系统'];
  const details = [board, image, '连接设备与模式', '执行写入与验证'];
  return (
    <ol className="vi-steps" aria-label="当前进度">
      {names.map((name, i) => (
        <li key={name} className={i < 2 ? 'done' : i === 2 ? 'now' : ''}>
          <span>{i < 2 ? <LynxIcon name="check" size={16} /> : i + 1}</span>
          <div>
            <strong>{name}</strong>
            <small title={details[i]}>{details[i]}</small>
          </div>
        </li>
      ))}
    </ol>
  );
}

function SummaryRow({ icon, label, value }: { icon: LynxIconName; label: string; value: string }) {
  return (
    <div className="vi-fact">
      <dt>
        <LynxIcon name={icon} size={16} />
        {label}
      </dt>
      <dd title={value}>{value}</dd>
    </div>
  );
}

function CheckRow({
  icon,
  title,
  help,
  state,
  value,
}: {
  icon: LynxIconName;
  title: string;
  help: string;
  state: string;
  value: string;
}) {
  return (
    <li>
      <span className="vi-check-icon">
        <LynxIcon name={icon} size={20} />
      </span>
      <span>
        <strong>{title}</strong>
        <small>{help}</small>
      </span>
      <span className={'vi-verdict' + (state === '通过' ? ' ok' : '')}>
        <LynxIcon name={state === '通过' ? 'check-circle' : 'clock'} size={16} />
        {state}
      </span>
      <span className="vi-value" title={value}>
        {value}
      </span>
    </li>
  );
}

export function V2Interface({
  board,
  boardName,
  imagePath,
  imageSize,
  imageVerified,
  imageSha256,
  loaderPath,
  loaderReady,
  onChooseLoader,
  onDropLoader,
  selectedDeviceId,
  fixedDeviceId,
  onTargetChange,
  onOpenAdvanced,
  onDetected,
  active = true,
}: V2InterfaceProps) {
  const platform = LINUX_CHIP_VENDORS.find((p) => p.value === board.chipVendor)!;
  const [found, setFound] = useState<WorkbenchFlasherDevice[]>([]);
  const [profiles, setProfiles] = useState<WorkbenchDevice[]>([]);
  const [requestedPath, setRequestedPath] = useState<string | null>(null);
  const [probedPath, setProbedPath] = useState('');
  const [picked, setPicked] = useState('');
  const [loading, setLoading] = useState(false);
  const [scanned, setScanned] = useState(false);
  const [binding, setBinding] = useState(false);
  const [error, setError] = useState('');
  const [help, setHelp] = useState(false);
  const [routeHelp, setRouteHelp] = useState(false);
  const sequence = useRef(0);
  const bindLock = useRef(false);
  const callbacks = useRef({ onTargetChange, onDetected, onDropLoader });
  useEffect(() => {
    callbacks.current = { onTargetChange, onDetected, onDropLoader };
  }, [onTargetChange, onDetected, onDropLoader]);
  const dropZone = useRef<HTMLDivElement>(null);
  const allProfiles = useCallback(
    () => invokeCommand('workbench_list_devices', undefined, { timeoutMs: 8000 }),
    []
  );
  const scanDevices = useCallback(
    (path: string) =>
      invokeCommand('workbench_scan_flasher_devices', { kind: platform.flasher, path }),
    [platform.flasher]
  );
  const profilePaths = useCallback(
    (saved: WorkbenchDevice[]) => [
      ...new Set(
        saved
          .filter(
            (p) =>
              p.chipVendor === board.chipVendor &&
              p.flasherKind === platform.flasher &&
              (fixedDeviceId == null || p.id === fixedDeviceId)
          )
          .map(scannerPath)
      ),
    ],
    [board.chipVendor, platform.flasher, fixedDeviceId]
  );
  const refresh = useCallback(async () => {
    const seq = ++sequence.current;
    setLoading(true);
    setScanned(false);
    setPicked('');
    setFound([]);
    setError('');
    callbacks.current.onTargetChange(null, null);
    callbacks.current.onDetected?.(null);
    try {
      const saved = await allProfiles();
      if (sequence.current !== seq) return;
      setProfiles(saved);
      const paths = profilePaths(saved);
      if (fixedDeviceId != null && !paths.length)
        throw new Error('绑定的设备配置已变化，请重新选择资源。');
      let path = paths.length === 1 ? paths[0] : '';
      if (
        fixedDeviceId == null &&
        requestedPath != null &&
        (requestedPath === '' || paths.includes(requestedPath))
      )
        path = requestedPath;
      setProbedPath(path);
      const devices = await scanDevices(path);
      if (sequence.current !== seq) return;
      setFound(devices);
      setScanned(true);
      callbacks.current.onDetected?.(devices.length);
    } catch (e) {
      if (sequence.current === seq) setError(formatErrorMessage(e));
    } finally {
      if (sequence.current === seq) setLoading(false);
    }
  }, [allProfiles, scanDevices, requestedPath, profilePaths, fixedDeviceId]);
  useEffect(() => {
    if (!active) {
      setFound([]);
      setPicked('');
      setScanned(false);
      setLoading(false);
      setError('');
      callbacks.current.onTargetChange(null, null);
      callbacks.current.onDetected?.(null);
      return;
    }
    const generation = sequence;
    let cancelled = false;
    let release: (() => void) | undefined;
    void refresh();
    void subscribeUsbHotplug(() => {
      if (!cancelled) void refresh();
    })
      .then((unlisten) => {
        if (cancelled) unlisten();
        else release = unlisten;
      })
      .catch(() => {});
    return () => {
      generation.current++;
      cancelled = true;
      release?.();
    };
  }, [active, refresh, board.id, boardName]);

  useEffect(() => {
    if (!active || !board.needsLoader) return;
    let cancelled = false;
    let release: (() => void) | undefined;
    try {
      void getCurrentWebview()
        .onDragDropEvent(({ payload }) => {
          if (cancelled || payload.type !== 'drop' || payload.paths.length !== 1) return;
          const area = dropZone.current?.getBoundingClientRect();
          if (!area || !area.width) return;
          const { x, y } = payload.position;
          const ratio = window.devicePixelRatio;
          if (
            x / ratio >= area.left &&
            x / ratio < area.right &&
            y / ratio >= area.top &&
            y / ratio < area.bottom
          )
            callbacks.current.onDropLoader(payload.paths[0]);
        })
        .then((unlisten) => {
          if (cancelled) unlisten();
          else release = unlisten;
        })
        .catch(() => {});
    } catch {
      /* Native file selection remains available. */
    }
    return () => {
      cancelled = true;
      release?.();
    };
  }, [active, board.needsLoader]);

  if (!active) return null;
  const detecting = loading || (!scanned && !error);
  const usb = found.find((d) => d.key === picked);
  const deviceMatches = (d: WorkbenchFlasherDevice) => profiles.filter((p) => matches(p, d, found));
  const allowed = (p: WorkbenchDevice) =>
    p.chipVendor === board.chipVendor &&
    scannerPath(p) === probedPath &&
    (!fixedDeviceId || p.id === fixedDeviceId);
  const modeOk = (d: WorkbenchFlasherDevice) =>
    board.chipVendor !== 'rockchip' || ['loader', 'maskrom'].includes(d.mode.toLowerCase());
  const profile = usb && deviceMatches(usb).find((p) => allowed(p) && p.id === selectedDeviceId);
  function chooseUsb(d: WorkbenchFlasherDevice) {
    setError('');
    setPicked(d.key);
    const options = deviceMatches(d).filter(allowed);
    if (options.length === 1 && modeOk(d)) callbacks.current.onTargetChange(options[0], d);
    else callbacks.current.onTargetChange(null, null);
  }
  async function bindNew(d: WorkbenchFlasherDevice) {
    if (bindLock.current || fixedDeviceId || loading) return;
    bindLock.current = true;
    setBinding(true);
    setError('');
    const seq = sequence.current;
    try {
      const [current, saved] = await Promise.all([scanDevices(probedPath), allProfiles()]);
      if (seq !== sequence.current) return;
      const live = uniqueUsbTarget(d.key, platform.flasher, current);
      if (live.vid !== d.vid || live.pid !== d.pid || live.mode !== d.mode || live.chip !== d.chip)
        throw new Error('设备身份或模式已变化，请重新检测。');
      if (saved.some((p) => matches(p, live, current)))
        throw new Error('该物理接口已有绑定，请重新检测后选择。');
      const id = await invokeCommand('workbench_save_device', {
        device: {
          ...EMPTY_LINUX_DEVICE,
          name: boardName,
          chipVendor: board.chipVendor,
          flasherKind: platform.flasher,
          flasherPath: probedPath,
          usbBinding: live.key,
        },
      });
      const updated = await allProfiles();
      if (seq !== sequence.current) return;
      setProfiles(updated);
      const created = updated.find(
        (p) => p.id === id && p.chipVendor === board.chipVendor && matches(p, live, current)
      );
      if (!created) throw new Error('已保存绑定，请重新检测并确认目标。');
      callbacks.current.onTargetChange(modeOk(live) ? created : null, modeOk(live) ? live : null);
    } catch (e) {
      if (seq === sequence.current) setError(formatErrorMessage(e));
    } finally {
      bindLock.current = false;
      setBinding(false);
    }
  }

  const rk = board.chipVendor === 'rockchip';
  const loaderLabel = rk ? 'RAM Loader' : 'RAM FES Loader';
  const missingLoader = board.needsLoader && !loaderReady;
  const file = fileName(imagePath);
  const fileSize = imageSize === null ? '尚未读取' : formatLinuxBytes(imageSize);
  const choices = usb ? deviceMatches(usb).filter(allowed) : [];
  const customPaths = profilePaths(profiles).filter(Boolean);
  const hasTarget = !!profile && !!usb && modeOk(usb);
  const status = detecting
    ? '正在检测设备'
    : missingLoader
      ? '等待补齐 Loader'
      : error
        ? '需要检查'
        : usb && !modeOk(usb)
          ? '设备模式不匹配'
          : hasTarget
            ? '设备已就绪'
            : '等待选择设备';
  const hint = missingLoader
    ? `请选择与 ${boardName} 匹配的 ${loaderLabel} 文件。`
    : hasTarget
      ? '可继续核对本次烧录参数。'
      : `请连接目标板，并按板型要求进入 ${board.mode} 模式。`;
  const route = rk
    ? 'Rockchip 完整 eMMC 镜像'
    : board.id === 'f101'
      ? '全志 SPI NOR 物理写入'
      : platform.label + ' 厂商镜像';
  const verify = rk
    ? '完整写入后校验'
    : board.id === 'f101'
      ? '独立物理回读'
      : board.chipVendor === 'allwinner'
        ? '写入后校验'
        : '按已保存的厂商配置';
  const lastAction =
    board.chipVendor === 'allwinner' || rk ? '校验后发送重启指令' : '按已保存的厂商配置';
  const renderDevices = (
    <section className={'vi-box vi-devices' + (rk ? ' vi-device-rk' : '')} aria-label="检测设备">
      <div className="vi-bar">
        <h3>
          {rk ? (
            <i className={'vi-dot' + (found.length ? ' ok' : '')} />
          ) : (
            <b className="vi-number">1</b>
          )}
          {rk ? (scanned ? `已识别 ${found.length} 台设备` : '检测设备') : '检测设备'}
        </h3>
        {customPaths.length > 0 && fixedDeviceId == null && (
          <select
            aria-label="检测与烧录使用的工具"
            title={probedPath || '内置烧录工具'}
            style={{ maxWidth: 'min(100%, 28ch)' }}
            disabled={detecting || binding}
            value={probedPath}
            onChange={(e) => {
              callbacks.current.onTargetChange(null, null);
              setPicked('');
              setRequestedPath(e.target.value);
            }}
          >
            <option value="">内置烧录工具</option>
            {customPaths.map((path) => (
              <option key={path} value={path}>
                {path}
              </option>
            ))}
          </select>
        )}
        <button type="button" disabled={detecting || binding} onClick={() => void refresh()}>
          <LynxIcon name="refresh" size={16} />
          {detecting ? '检测中…' : '重新检测'}
        </button>
        {!rk && (
          <button type="button" aria-expanded={help} onClick={() => setHelp(!help)}>
            <LynxIcon name="info" size={16} />
            驱动帮助
          </button>
        )}
      </div>
      <div className="vi-device-list" aria-live="polite">
        {found.map((d, i) => {
          const saved = deviceMatches(d);
          const selectable = !fixedDeviceId || saved.some(allowed);
          return (
            <label key={d.key} className={'vi-device' + (picked === d.key ? ' chosen' : '')}>
              <input
                type="radio"
                name="vi-usb-target"
                checked={picked === d.key}
                onChange={() => chooseUsb(d)}
                disabled={binding || !selectable}
              />
              {!rk && <LynxIcon name="plug" size={24} />}
              <span className="vi-usb-identity">
                <strong>{saved.find(allowed)?.name || `USB 设备 #${i + 1}`}</strong>
                <small title={usbLocation(d)}>{usbLocation(d)}</small>
              </span>
              <span className={'vi-chip' + (rk ? ' purple' : '')}>{modeLabel(d.mode)}</span>
              {!rk && (
                <span className="vi-owner" title={saved.map((p) => p.name).join('、')}>
                  {saved.length ? '已绑定' : '待绑定'}
                </span>
              )}
              <span className="vi-connected">
                <LynxIcon name="check-circle" size={16} />
                已发现
              </span>
            </label>
          );
        })}
        {found.length === 0 && (
          <p className="vi-empty">
            {detecting
              ? '正在枚举 USB 设备…'
              : error
                ? '检测未能完成，请检查连接或烧录工具。'
                : `未发现设备。请用数据线连接开发板，并进入 ${board.mode} 模式后重新检测。`}
          </p>
        )}
      </div>
      {fixedDeviceId && found.length > 0 && !found.some((d) => deviceMatches(d).some(allowed)) && (
        <div className="vi-help">
          <p>此镜像已绑定到指定单板。请连接该单板，或在设备设置中核对已保存的 USB 接口。</p>
          <button type="button" onClick={onOpenAdvanced}>
            查看设备设置
          </button>
        </div>
      )}
      {usb && !profile && (
        <div className="vi-binding">
          {!choices.length && deviceMatches(usb).length > 0 && (
            <p>此接口已有其他工具或平台的绑定。请选择该配置使用的烧录工具。</p>
          )}
          {choices.length > 1 ? (
            <label>
              已保存的绑定{' '}
              <select
                value=""
                onChange={(e) => {
                  const p = choices.find((item) => item.id === Number(e.target.value));
                  if (p && modeOk(usb)) callbacks.current.onTargetChange(p, usb);
                }}
              >
                <option value="">选择单板配置…</option>
                {choices.map((p) => (
                  <option key={p.id} value={p.id!}>
                    {p.name}
                  </option>
                ))}
              </select>
            </label>
          ) : (
            !deviceMatches(usb).length &&
            !fixedDeviceId && (
              <button type="button" disabled={binding} onClick={() => void bindNew(usb)}>
                {binding ? '正在保存…' : `将此接口绑定到 ${boardName}`}
              </button>
            )
          )}
          {!modeOk(usb) && (
            <span role="status">
              此完整 eMMC 路线需要 Maskrom 或 Loader 模式。当前检测为 {modeLabel(usb.mode)}。
            </span>
          )}
          {!!deviceMatches(usb).length && choices.length === 0 && (
            <span>此接口的已保存绑定与当前目标不符。</span>
          )}
        </div>
      )}
      {error && (
        <p role="alert" className="vi-error">
          {error}
        </p>
      )}
      {help && (
        <div className="vi-help">
          <strong>USB 连接与驱动</strong>
          <p>
            确认 USB 数据线与供电正常，按该板说明进入 {board.mode}
            ，然后重新检测。自定义烧录器和接口绑定可在设备设置检查。
          </p>
          <button type="button" onClick={onOpenAdvanced}>
            查看设备设置
          </button>
        </div>
      )}
    </section>
  );

  return (
    <div className={'vi-grid' + (rk ? ' vi-rk' : '')}>
      <section className="vi-main" aria-label="配置接口">
        {!rk && <StageStrip board={boardName} image={file} />}
        <header className="vi-heading">
          <h2>
            <LynxIcon name={rk ? 'plug' : 'task'} size={24} />
            配置接口
          </h2>
          <p>
            {rk
              ? '连接设备并配置当前路线所需的接口、模式和 Loader 文件。'
              : `连接开发板并选择目标设备，系统将自动识别设备模式（${board.mode}）。`}
          </p>
        </header>
        {renderDevices}
        {board.needsLoader && (
          <>
            <section className="vi-box vi-route">
              <h3>
                <LynxIcon name="box" size={20} />
                烧录路线与 Loader 配置
              </h3>
              <div>
                <label>
                  平台 / 路线<output>{route}</output>
                </label>
                <label>
                  目标存储<output>{board.storage}</output>
                </label>
                <button
                  type="button"
                  onClick={() => setRouteHelp(!routeHelp)}
                  aria-expanded={routeHelp}
                >
                  <LynxIcon name="book" size={16} />
                  路线说明
                </button>
              </div>
              {routeHelp && (
                <p className="vi-route-note">
                  {rk
                    ? '使用板型匹配的完整 eMMC 镜像与 RAM Loader。Maskrom 模式下加载至内存，已在 Loader 模式时沿用当前会话。写入会擦除目标 eMMC。'
                    : '完整镜像从 SPI NOR 物理起始位置写入。独立 FES Loader 仅加载至 RAM，物理回读和启动结果由后端分别报告。'}
                </p>
              )}
            </section>
            <section className={'vi-box vi-loader' + (missingLoader ? ' missing' : '')}>
              <h3>
                <LynxIcon name={missingLoader ? 'warning' : 'check-circle'} size={20} />
                {loaderLabel} <em>（必填）</em>
              </h3>
              <p>
                {missingLoader
                  ? `当前路线需要匹配的 ${loaderLabel}，补齐文件后才能开始预检或烧录。`
                  : `已读取 ${fileName(loaderPath)}`}
              </p>
              <div ref={dropZone} className="vi-drop">
                <LynxIcon name="upload" size={24} />
                <span>
                  {loaderReady ? fileName(loaderPath) : `将 ${loaderLabel} 文件拖拽到此处，或`}
                </span>
                <button type="button" onClick={onChooseLoader}>
                  <LynxIcon name="folder" size={16} />
                  {loaderPath ? '重新选择' : '选择文件'}
                </button>
                <small>
                  {rk ? '选择板型对应的 Rockchip Loader' : '使用与该板匹配的独立 RAM FES Loader'}
                </small>
              </div>
            </section>
          </>
        )}
        {(!rk || loaderReady) && (
          <section className="vi-box vi-checks">
            <h3>
              <b className="vi-number">2</b>预检（只读检查）
            </h3>
            <ul>
              <CheckRow
                icon="document"
                title="镜像读取"
                help="检查文件可读性与实际大小"
                state={imageSize ? '通过' : '待检查'}
                value={`${file} · ${fileSize}`}
              />
              <CheckRow
                icon="board"
                title="板型兼容"
                help="核对镜像与目标板的适配依据"
                state={imageVerified ? '通过' : '待核对'}
                value={
                  imageVerified
                    ? `发布清单 · ${boardName}`
                    : `本地镜像，请核对 ${boardName} 的适配说明`
                }
              />
              <CheckRow
                icon="plug"
                title="目标设备"
                help="检查 USB 身份、模式与设备绑定"
                state={hasTarget ? '通过' : '待选择'}
                value={usb ? usbLocation(usb) : '请选择实际 USB 目标'}
              />
              <CheckRow
                icon="box"
                title="后端路线"
                help="核对存储、接口与必要文件"
                state={missingLoader ? '缺文件' : '待复核'}
                value={route}
              />
            </ul>
            <p className="vi-info">
              <LynxIcon name="info" size={16} />
              {hasTarget && !missingLoader ? '目标已选择，继续后将复核接口与文件。' : hint}
            </p>
          </section>
        )}
      </section>
      <aside className="vi-side">
        <details open className="vi-box vi-summary">
          <summary>
            <LynxIcon name="list-check" size={20} />
            {rk ? '任务详情' : '任务摘要'}
          </summary>
          {!rk && (
            <div className="vi-board">
              <V2BoardArt id={board.id} name={boardName} />
              <div>
                <strong>{boardName}</strong>
                <small>{platform.label}</small>
              </div>
            </div>
          )}
          <dl>
            {rk && (
              <SummaryRow
                icon="board"
                label="目标设备"
                value={usb ? `${boardName} · ${usbId(usb)}` : boardName}
              />
            )}
            <SummaryRow icon="desktop" label={rk ? '镜像文件' : '系统镜像'} value={file} />
            {rk && imageSha256 && <SummaryRow icon="shield" label="SHA-256" value={imageSha256} />}
            {!rk && (
              <>
                <SummaryRow icon="document" label="大小" value={fileSize} />
                <SummaryRow
                  icon="board"
                  label="目标设备"
                  value={profile?.name || (usb ? usb.chip : '等待选择')}
                />
                <SummaryRow
                  icon="plug"
                  label="USB 身份"
                  value={usb ? usbLocation(usb) : '等待选择'}
                />
                <SummaryRow
                  icon="terminal"
                  label="设备模式"
                  value={usb ? modeLabel(usb.mode) : '待检测'}
                />
              </>
            )}
            <SummaryRow icon="database" label="目标存储" value={board.storage} />
            <SummaryRow
              icon={rk ? 'shield' : 'task'}
              label={rk ? '校验策略' : '结束动作'}
              value={rk ? verify : lastAction}
            />
          </dl>
        </details>
        <section className="vi-box vi-tips">
          <h3>
            <LynxIcon name="info" size={20} />
            {rk ? '当前状态' : '提示'}
          </h3>
          {rk ? (
            <>
              <strong className={'vi-badge' + (hasTarget && !missingLoader ? ' ready' : '')}>
                {status}
              </strong>
              <p>{hint}</p>
            </>
          ) : (
            <>
              <ul>
                <li>当前为只读检查，尚未开始擦写。</li>
                <li>烧录将覆盖目标存储内容，请先备份重要数据。</li>
                <li>如检测不到设备，请检查 USB 连接、供电和设备模式。</li>
              </ul>
              <button
                type="button"
                className="vi-text-action"
                onClick={() => setHelp(!help)}
                aria-expanded={help}
              >
                查看连接与模式说明
              </button>
            </>
          )}
        </section>
      </aside>
    </div>
  );
}
