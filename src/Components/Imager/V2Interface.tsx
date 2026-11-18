import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { LynxIcon } from '../../CoreUI';
import { invokeCommand, type WorkbenchDevice } from '../../Platform/IPC';
import { subscribeUsbHotplug } from '../../Platform/IPC/Events';
import type { WorkbenchFlasherDevice } from '../../Platform/IPC/contracts/Workbench';
import { formatErrorMessage } from '../../Utils';
import { EMPTY_LINUX_DEVICE, LINUX_CHIP_VENDORS } from '../Workbench/linuxWorkbenchModel';
import { uniqueUsbTarget } from './BasicPlan';
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
  selectedDeviceId: number | null;
  fixedDeviceId?: number | null;
  onTargetChange: (profile: WorkbenchDevice | null, usb: WorkbenchFlasherDevice | null) => void;
  onOpenAdvanced: () => void;
  onDetected?: (count: number | null) => void;
  active?: boolean;
}

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

export function V2Interface({
  board,
  boardName,
  routeLabel,
  vendorTarget,
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
  const sequence = useRef(0);
  const bindLock = useRef(false);
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
  const choices = usb ? deviceMatches(usb).filter(allowed) : [];
  const customPaths = profilePaths(profiles).filter(Boolean);
  const hasTarget = !!profile && !!usb && modeOk(usb);
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
            : '使用开发板的 USB 下载口，按板级说明进入下载模式后匹配实际端口。'}
        </p>
      </header>
      <div className="r3-wire">
        <span>
          <LynxIcon name="desktop" size={24} />
          <strong>运行 LYNX 的电脑</strong>
        </span>
        <span className="r3-wire-path">
          {usb
            ? usb.key
            : detecting
              ? '正在枚举'
              : found.length
                ? found.length + ' 个候选端口'
                : '等待设备'}
        </span>
        <span>
          <LynxIcon name="board" size={24} />
          <strong>
            {boardName} · {board.mode}
          </strong>
        </span>
      </div>
      <div className="r3-connect-columns">
        {renderDevices}
        <section className="r3-connect-facts">
          <h3>{usb ? '连接详情' : '如何连接'}</h3>
          {usb ? (
            <dl>
              {[
                ['物理端口', usb.key],
                ['VID : PID', usbId(usb)],
                ['实际模式', modeLabel(usb.mode)],
                ['板卡配置', profile?.name || '等待绑定'],
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
              <li>选择实际端口。调试串口可用于稍后观察启动。</li>
            </ol>
          )}
        </section>
      </div>
      {vendorTarget}
      {hasTarget && <p className="vi-info">提交前将再次检查文件、接口和已保存的设备绑定。</p>}
    </section>
  );
}
