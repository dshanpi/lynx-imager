import { useEffect, useRef, useState } from 'react';
import { invokeCommand, subscribeEvent } from '../../Platform/IPC';
import { formatErrorMessage } from '../../Utils';

type Target = { vendor: string; key: string; pid: number; loader: boolean };
export function EmmcErase({ model }: { model: string }) {
  const [erased, setErased] = useState(0);
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
  const dialog = useRef<HTMLDialogElement>(null);
  const [targets, setTargets] = useState<Target[]>([]);
  const [key, setKey] = useState('');
  const [phase, setPhase] = useState<'idle' | 'scan' | 'check' | 'erase'>('idle');
  const [message, setMessage] = useState('');
  const [preview, setPreview] = useState<{ token: string; bytes: number } | null>(null);
  const busy = useRef(false);
  useEffect(
    () => () => {
      if (preview)
        void invokeCommand('imager_emmc_cancel', { token: preview.token }).catch(() => {});
    },
    [preview]
  );

  async function run(stage: typeof phase, work: () => Promise<void>) {
    if (busy.current) return;
    if (stage === 'erase') setErased(0);
    busy.current = true;
    setPhase(stage);
    setMessage('');
    try {
      await work();
    } catch (e) {
      setMessage(formatErrorMessage(e));
    } finally {
      setPhase('idle');
      busy.current = false;
    }
  }
  function scan() {
    void run('scan', async () => {
      setPreview(null);
      setTargets([]);
      setKey('');
      const devices = (await invokeCommand('imager_usb_targets', {})).filter(
        (d) => d.vendor === 'rockchip'
      );
      setTargets(devices);
      if (!devices.some((d) => d.loader))
        setMessage('请连接处于 RockUSB Loader 模式的设备，再重新检测。');
    });
  }
  const selected = targets.find((d) => d.key === key && d.loader);
  return (
    <div className="emmc-maintenance">
      <button
        type="button"
        onClick={() => {
          dialog.current?.showModal();
          scan();
        }}
      >
        擦除 eMMC…
      </button>
      <dialog
        ref={dialog}
        aria-labelledby="erase-title"
        onCancel={(e) => {
          if (busy.current) e.preventDefault();
        }}
        onClose={() => {
          setPreview(null);
          setKey('');
          setMessage('');
        }}
      >
        <h3 id="erase-title">擦除 eMMC 用户区</h3>
        <p>当前选择 {model}。请按 USB 端口核对设备。此操作会清空系统、分区表和用户数据。</p>
        <label>
          目标 USB 端口
          <select
            value={key}
            disabled={phase !== 'idle' || !!preview}
            onChange={(e) => {
              setKey(e.target.value);
              setMessage('');
            }}
          >
            <option value="">请选择设备</option>
            {targets.map((d) => (
              <option key={d.key} value={d.key} disabled={!d.loader}>
                USB {d.key} · 2207:{d.pid.toString(16).padStart(4, '0')} ·{' '}
                {d.loader ? 'Loader' : 'Maskrom'}
              </option>
            ))}
          </select>
        </label>
        {preview && (
          <p>
            将清空 USB {key} 的整个 eMMC 用户区，共 {preview.bytes.toLocaleString()} 字节。
          </p>
        )}
        <p role="status">
          {phase === 'erase'
            ? `已擦除 ${(erased / 1024 ** 2).toFixed(1)} MiB，请保持供电和 USB 连接…`
            : phase === 'check'
              ? '正在读取介质和容量…'
              : phase === 'scan'
                ? '正在检测设备…'
                : message}
        </p>
        <footer>
          <button type="button" disabled={phase !== 'idle'} onClick={() => dialog.current?.close()}>
            关闭
          </button>
          {!preview && (
            <button type="button" disabled={phase !== 'idle'} onClick={scan}>
              重新检测
            </button>
          )}
          {preview ? (
            <button
              className="erase-confirm"
              type="button"
              disabled={phase !== 'idle'}
              onClick={() =>
                void run('erase', async () => {
                  try {
                    const bytes = await invokeCommand('imager_emmc_erase', {
                      token: preview.token,
                    });
                    setMessage('擦除完成 · ' + bytes.toLocaleString() + ' 字节');
                  } finally {
                    setPreview(null);
                    setKey('');
                  }
                })
              }
            >
              确认擦除
            </button>
          ) : (
            <button
              type="button"
              disabled={!selected || phase !== 'idle'}
              onClick={() =>
                selected &&
                void run('check', async () => {
                  setPreview(
                    await invokeCommand('imager_emmc_prepare', {
                      key: selected.key,
                      pid: selected.pid,
                    })
                  );
                })
              }
            >
              检查设备容量
            </button>
          )}
        </footer>
      </dialog>
    </div>
  );
}
