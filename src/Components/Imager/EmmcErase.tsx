import { useEffect, useRef, useState } from 'react';
import { LynxIcon } from '../../CoreUI';
import { invokeCommand, subscribeEvent } from '../../Platform/IPC';
import { formatErrorMessage } from '../../Utils';
import './EmmcErase.css';

type Target = { vendor: string; key: string; pid: number; loader: boolean };
type Preview = { bytes: number; token: string };
type State =
  | { phase: 'pick' | 'scan'; targets: Target[]; error?: string }
  | { phase: 'check'; device: Target }
  | { phase: 'ready' | 'erase'; device: Target; preview: Preview }
  | { phase: 'done'; device: Target; bytes: number }
  | { phase: 'failed'; device: Target; error: string };

function capacity(bytes: number) {
  const gib = bytes >= 1024 ** 3;
  return {
    number: (bytes / (gib ? 1024 ** 3 : 1024 ** 2)).toLocaleString(undefined, {
      maximumFractionDigits: 2,
    }),
    unit: gib ? 'GiB' : 'MiB',
  };
}
function usbId(device: Target) {
  return '2207:' + device.pid.toString(16).padStart(4, '0');
}
export function EmmcErase({ model }: { model: string }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const heading = useRef<HTMLHeadingElement>(null);
  const ticket = useRef('');
  const busy = useRef(false);
  const [state, setState] = useState<State>({ phase: 'pick', targets: [] });
  const [erased, setErased] = useState(0);
  const { phase } = state;
  function release() {
    if (!ticket.current) return;
    void invokeCommand('imager_emmc_cancel', { token: ticket.current }).catch(() => {});
    ticket.current = '';
  }
  useEffect(() => release, []);
  useEffect(() => {
    let active = true;
    const off = subscribeEvent('imager-emmc-progress', (bytes) => {
      if (active) setErased(bytes);
    }).catch(() => () => {});
    return () => {
      active = false;
      void off.then((stop) => stop());
    };
  }, []);

  useEffect(() => {
    if (phase === 'ready' || phase === 'done' || phase === 'failed') heading.current?.focus();
  }, [phase]);
  async function scan() {
    if (busy.current) return;
    busy.current = true;
    release();
    setState({ phase: 'scan', targets: [] });
    try {
      const targets = (await invokeCommand('imager_usb_targets', {})).filter(
        (d) => d.vendor === 'rockchip'
      );
      setState({ phase: 'pick', targets });
    } catch (e) {
      setState({ phase: 'pick', targets: [], error: formatErrorMessage(e) });
    } finally {
      busy.current = false;
    }
  }
  async function check(device: Target) {
    if (busy.current || !device.loader) return;
    busy.current = true;
    setState({ phase: 'check', device });
    try {
      const preview = await invokeCommand('imager_emmc_prepare', {
        key: device.key,
        pid: device.pid,
      });
      ticket.current = preview.token;
      if (!dialog.current?.open) {
        release();
        return;
      }
      setState({ phase: 'ready', device, preview });
    } catch (e) {
      setState({ phase: 'failed', device, error: formatErrorMessage(e) });
    } finally {
      busy.current = false;
    }
  }
  async function erase() {
    if (busy.current || state.phase !== 'ready' || ticket.current !== state.preview.token) return;
    busy.current = true;
    setErased(0);
    setState({ ...state, phase: 'erase' });
    try {
      const bytes = await invokeCommand('imager_emmc_erase', { token: state.preview.token });
      setState({ phase: 'done', device: state.device, bytes });
    } catch (e) {
      setState({ phase: 'failed', device: state.device, error: formatErrorMessage(e) });
    } finally {
      release();
      busy.current = false;
    }
  }

  const pending = phase === 'scan' || phase === 'check' || phase === 'erase';
  const device = 'device' in state ? state.device : null;
  const total = 'preview' in state ? state.preview.bytes : state.phase === 'done' ? state.bytes : 0;
  const amount = capacity(total);
  const title =
    phase === 'done'
      ? '擦除完成'
      : phase === 'failed'
        ? '操作未完成'
        : phase === 'erase'
          ? '正在擦除 eMMC'
          : '擦除 eMMC';
  return (
    <div className="emmc-maintenance">
      <button
        type="button"
        className="emmc-launch"
        onClick={() => {
          dialog.current?.showModal();
          void scan();
        }}
      >
        <LynxIcon name="delete" size={16} /> 擦除 eMMC…
      </button>
      <dialog
        className="emmc-dialog"
        ref={dialog}
        aria-labelledby="erase-title"
        onCancel={(e) => {
          if (busy.current) e.preventDefault();
        }}
        onClose={() => {
          release();
          setState({ phase: 'pick', targets: [] });
        }}
      >
        <header className="emmc-head">
          <div className="emmc-symbol" data-complete={phase === 'done'}>
            <LynxIcon name={phase === 'done' ? 'check-circle' : 'database'} size={24} />
          </div>
          <div>
            <h2 id="erase-title" ref={heading} tabIndex={-1}>
              {title}
            </h2>
            <p>{model} · 板载存储</p>
          </div>
        </header>
        <div className="emmc-body">
          {'targets' in state && (
            <>
              <div className="emmc-section-title">
                <h3>选择目标 USB 端口</h3>
                <button type="button" className="emmc-link" disabled={pending} onClick={scan}>
                  <LynxIcon name="refresh" size={16} />
                  重新检测
                </button>
              </div>
              <div className="emmc-ports">
                {state.targets.map((d) => (
                  <button
                    type="button"
                    className="emmc-port"
                    key={d.key}
                    disabled={!d.loader}
                    onClick={() => void check(d)}
                  >
                    <LynxIcon name="plug" size={20} />
                    <span>
                      <strong>USB {d.key}</strong>
                      <small>
                        {usbId(d)} · {d.loader ? 'Loader' : 'Maskrom · 需要先加载 Loader'}
                      </small>
                    </span>
                    {d.loader && <LynxIcon name="chevron-right" size={16} />}
                  </button>
                ))}
                {!state.targets.length && (
                  <p className="emmc-empty" role="status">
                    {state.error ||
                      (phase === 'scan'
                        ? '正在检测 USB 设备…'
                        : '未发现设备。请连接处于 Loader 模式的瑞芯微开发板。')}
                  </p>
                )}
              </div>
              {state.targets.some((d) => d.loader) && (
                <p className="emmc-hint">选择端口后读取介质类型和容量。</p>
              )}
            </>
          )}
          {device && (
            <div className="emmc-device">
              <LynxIcon name="plug" size={20} />
              <div>
                <small>目标设备</small>
                <strong>USB {device.key}</strong>
              </div>
              <span className="emmc-usb-id">{usbId(device)} · Loader</span>
              {phase === 'ready' && (
                <button type="button" className="emmc-link" onClick={scan}>
                  更换设备
                </button>
              )}
            </div>
          )}
          {total > 0 && (
            <section className="emmc-range" aria-label="擦除范围" data-done={phase === 'done'}>
              <div className="emmc-range-title">
                <div>
                  <small>{phase === 'done' ? '已擦除范围' : '擦除范围'}</small>
                  <h3>整个 eMMC 用户区</h3>
                </div>
                <span>
                  <b>{amount.number}</b> {amount.unit}
                </span>
              </div>
              {phase === 'erase' ? (
                <progress aria-label="擦除进度" value={erased} max={total} />
              ) : (
                <div className="emmc-range-track" aria-hidden="true" />
              )}
              <div className="emmc-range-scale">
                <span>0 B</span>
                <span>{total.toLocaleString()} B</span>
              </div>
            </section>
          )}
          {phase === 'ready' && (
            <p className="emmc-warning">
              <LynxIcon name="warning" size={20} />
              <span>系统、分区表和用户数据将被永久清除。</span>
            </p>
          )}
          {phase === 'check' && (
            <p className="emmc-hint" role="status">
              正在读取设备容量…
            </p>
          )}
          {phase === 'erase' && (
            <p className="emmc-hint" role="status">
              已擦除 {(erased / 1024 ** 2).toFixed(1)} MiB，请保持供电和 USB 连接。
            </p>
          )}
          {state.phase === 'failed' && (
            <p className="emmc-warning" role="alert">
              <LynxIcon name="error" size={20} />
              {state.error}
            </p>
          )}
        </div>
        <footer className="emmc-actions">
          <button
            type="button"
            className="emmc-secondary"
            disabled={pending}
            onClick={() => dialog.current?.close()}
          >
            {phase === 'ready' ? '取消' : '关闭'}
          </button>
          {(phase === 'ready' || phase === 'erase') && (
            <button type="button" className="emmc-destructive" disabled={pending} onClick={erase}>
              {phase === 'erase' ? '正在擦除…' : '确认擦除'}
            </button>
          )}
          {phase === 'failed' && (
            <button type="button" className="emmc-secondary" onClick={scan}>
              重新检测
            </button>
          )}
        </footer>
      </dialog>
    </div>
  );
}
