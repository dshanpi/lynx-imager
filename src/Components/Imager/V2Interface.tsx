import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { LynxIcon } from '../../CoreUI';
import { invokeCommand, type WorkbenchDevice } from '../../Platform/IPC';
import { subscribeUsbHotplug } from '../../Platform/IPC/Events';
import type { WorkbenchFlasherDevice } from '../../Platform/IPC/contracts/Workbench';
import { formatErrorMessage } from '../../Utils';
import { EMPTY_LINUX_DEVICE, LINUX_CHIP_VENDORS } from '../Workbench/linuxWorkbenchModel';
import { uniqueUsbTarget } from './BasicPlan';
import { DriverSetup } from './DriverSetup';
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
  routeLabel: string;
  vendorTarget?: ReactNode;
  fixedDeviceId?: number | null;
  onTargetChange: (profile: WorkbenchDevice | null, usb: WorkbenchFlasherDevice | null) => void;
  onOpenAdvanced: () => void;
  onDetected?: (count: number | null) => void;
  active?: boolean;
  busy?: boolean;
}

const usbId = (usb: WorkbenchFlasherDevice) =>
  [usb.vid, usb.pid].map((n) => n.toString(16).padStart(4, '0')).join(':');
function portLabel(usb: WorkbenchFlasherDevice) {
  const chain = Array.from(usb.key.matchAll(/#usb\((\d+)\)/gi), (match) => match[1]).join('.');
  return chain ? `USB 端口 ${chain}` : `USB 总线 ${usb.bus} · 端口 ${usb.port}`;
}
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

export function V2Interface({
  board,
  boardName,
  routeLabel,
  vendorTarget,
  fixedDeviceId,
  onTargetChange,
  onOpenAdvanced,
  onDetected,
  active = true,
  busy = false,
}: V2InterfaceProps) {
  const platform = LINUX_CHIP_VENDORS.find((p) => p.value === board.chipVendor)!;
  const [found, setFound] = useState<WorkbenchFlasherDevice[]>([]);
  const [profiles, setProfiles] = useState<WorkbenchDevice[]>([]);
  const [requestedPath, setRequestedPath] = useState<string | null>(null);
  const [probedPath, setProbedPath] = useState('');
  const [picked, setPicked] = useState('');
  const [loading, setLoading] = useState(false);
  const [scanned, setScanned] = useState(false);
  const [error, setError] = useState('');
  const [help, setHelp] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [driverMessage, setDriverMessage] = useState('');
  const installLock = useRef(false);
  const currentView = useRef(0);
  const resumeScan = useRef<(() => Promise<void>) | null>(null);
  const sequence = useRef(0);
  const callbacks = useRef({ onTargetChange, onDetected });
  useEffect(() => {
    callbacks.current = { onTargetChange, onDetected };
  }, [onTargetChange, onDetected]);
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
    if (installLock.current) return;
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
    const view = ++currentView.current;
    setHelp(false);
    setDriverMessage('');
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
    resumeScan.current = refresh;
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
      if (currentView.current === view) currentView.current++;
      resumeScan.current = null;
      generation.current++;
      cancelled = true;
      release?.();
    };
  }, [active, refresh, board.id, boardName]);

  async function installDriver(infPath?: string) {
    if (installLock.current || busy) return;
    const view = currentView.current;
    installLock.current = true;
    setInstalling(true);
    setDriverMessage('');
    sequence.current++;
    setFound([]);
    setPicked('');
    setLoading(false);
    setScanned(true);
    setError('');
    callbacks.current.onTargetChange(null, null);
    callbacks.current.onDetected?.(null);
    try {
      const message = await invokeCommand('imager_install_driver', {
        vendor: board.chipVendor,
        infPath,
      });
      if (view === currentView.current) setDriverMessage(message);
    } catch (e) {
      if (view === currentView.current) setDriverMessage(formatErrorMessage(e));
    } finally {
      installLock.current = false;
      setInstalling(false);
      void resumeScan.current?.();
    }
  }
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
  function chooseUsb(d: WorkbenchFlasherDevice) {
    setError('');
    setPicked(d.key);
    const options = deviceMatches(d).filter(allowed);
    if (!modeOk(d) || (fixedDeviceId != null && !options.length)) {
      callbacks.current.onTargetChange(null, null);
      return;
    }
    // Reuse a unique matching setup, otherwise select just this physical port.
    const profile =
      options.length === 1
        ? options[0]
        : {
            ...EMPTY_LINUX_DEVICE,
            name: boardName,
            chipVendor: board.chipVendor,
            flasherKind: platform.flasher,
            flasherPath: probedPath,
            usbBinding: d.key,
          };
    callbacks.current.onTargetChange(profile, d);
  }

  const rk = board.chipVendor === 'rockchip';
  const customPaths = profilePaths(profiles).filter(Boolean);
  const hasTarget = !!usb && modeOk(usb);
  const renderDevices = (
    <section className={'vi-box vi-devices' + (rk ? ' vi-device-rk' : '')} aria-label="检测设备">
      <div className="vi-bar">
        <h3>{scanned ? `可用设备（${found.length}）` : '检测设备'}</h3>
        {customPaths.length > 0 && fixedDeviceId == null && (
          <select
            aria-label="检测与烧录使用的工具"
            title={probedPath || '内置烧录工具'}
            style={{ maxWidth: 'min(100%, 28ch)' }}
            disabled={detecting || busy || installing}
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
        <button
          type="button"
          disabled={detecting || busy || installing}
          onClick={() => void refresh()}
        >
          <LynxIcon name="refresh" size={16} />
          {detecting ? '检测中…' : '重新检测'}
        </button>
        <button
          type="button"
          aria-expanded={help}
          disabled={installing || busy}
          onClick={() => setHelp(!help)}
        >
          <LynxIcon name="download" size={16} /> 安装驱动
        </button>
      </div>
      <div className="vi-device-list" aria-live="polite">
        {found.map((d) => {
          const saved = deviceMatches(d).filter(allowed);
          const selectable = (!fixedDeviceId || saved.length > 0) && modeOk(d);
          return (
            <label key={d.key} className={'vi-device' + (picked === d.key ? ' chosen' : '')}>
              <input
                type="radio"
                name="vi-usb-target"
                checked={picked === d.key}
                onChange={() => chooseUsb(d)}
                disabled={busy || installing || !selectable}
              />
              <span className="vi-usb-identity">
                <strong>{saved.length === 1 ? saved[0].name : platform.label + ' USB 设备'}</strong>
                <small title={d.key}>{portLabel(d)}</small>
              </span>
              <span className="vi-chip">{modeLabel(d.mode)}</span>
              {!modeOk(d) && <small>需要 Maskrom 或 Loader 模式</small>}
            </label>
          );
        })}
        {found.length === 0 && (
          <p className="vi-empty">
            {installing
              ? '正在处理驱动安装…'
              : detecting
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
      {error && (
        <p role="alert" className="vi-error">
          {error}
        </p>
      )}
      {help && (
        <DriverSetup
          vendor={board.chipVendor}
          mode={board.mode}
          installing={installing}
          onInstall={(infPath) => void installDriver(infPath)}
        />
      )}
      {driverMessage && (
        <p className="vi-help" role="status">
          {driverMessage}
        </p>
      )}
    </section>
  );

  const title = hasTarget
    ? '核对已连接的下载设备'
    : found.length > 1
      ? '选定这次要写入的物理端口'
      : '连接开发板的 USB 下载口';
  return (
    <section className="r3-connect" aria-label="设备连接">
      <header className="r3-heading">
        <h2>
          <LynxIcon name="plug" size={20} />
          {title}
        </h2>
        <p>
          {found.length > 1
            ? '请按实际接线选择目标；相同的 VID 和 PID 可以对应不同开发板。'
            : '用数据线连接开发板的下载口，进入下载模式后选择设备。'}
        </p>
      </header>
      <div className="r3-connect-columns">
        {renderDevices}
        <section className="r3-connect-facts">
          <h3>{usb ? '连接详情' : '如何连接'}</h3>
          {usb ? (
            <dl>
              {[
                ['连接位置', portLabel(usb)],
                ['VID : PID', usbId(usb)],
                ['实际模式', modeLabel(usb.mode)],
                ['目标板型', boardName],
                ['本次路线', routeLabel],
                ['目标存储', board.storage + ' · 配置值'],
              ].map(([label, value]) => (
                <div key={label}>
                  <dt>{label}</dt>
                  <dd>{value}</dd>
                </div>
              ))}
            </dl>
          ) : (
            <ol>
              <li>使用数据线连接开发板的 USB 下载口。</li>
              <li>按板级说明进入 {board.mode} 模式并重新检测。</li>
              <li>点击要烧写的设备，然后进入下一步。</li>
            </ol>
          )}
          {usb && (
            <details className="vi-port-path">
              <summary>完整 USB 路径</summary>
              <code>{usb.key}</code>
            </details>
          )}
        </section>
      </div>
      {vendorTarget}
    </section>
  );
}
