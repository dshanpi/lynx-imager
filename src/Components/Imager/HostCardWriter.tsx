import { useEffect, useRef, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { invokeCommand, subscribeEvent } from '../../Platform/IPC';
import type { ImagerCard, ImagerCardImage } from '../../Platform/IPC/contracts/Workbench';

const size = (n: number) =>
  n < 1024 ** 3 ? (n / 1024 ** 2).toFixed(1) + ' MiB' : (n / 1024 ** 3).toFixed(2) + ' GiB';
interface Props {
  model: string;
  step: number;
  onStep: (step: 2 | 3 | 4) => void;
  run: (label: string, work: () => Promise<void>) => Promise<void>;
  onPath: (path: string) => void;
  busy: string;
}
export function HostCardWriter({ model, step, onStep, run, onPath, busy }: Props) {
  const [image, setImage] = useState<ImagerCardImage | null>(null);
  const [cards, setCards] = useState<ImagerCard[]>([]);
  const [card, setCard] = useState<ImagerCard | null>(null);
  const [finished, setFinished] = useState(false);
  const [progress, setProgress] = useState({ phase: '', bytes: 0 });
  const [issue, setIssue] = useState('');
  const generation = useRef(0);
  const historyKey = 'imager.sd-image.' + model;
  useEffect(() => {
    let last: string | null = null;
    try {
      last = localStorage.getItem(historyKey);
    } catch {
      return;
    }
    if (!last) return;
    const version = ++generation.current;
    setIssue('正在检查上次使用的镜像…');
    invokeCommand('imager_card_image', { path: last })
      .then((file) => {
        if (generation.current === version) {
          setImage(file);
          onPath(file.path);
          setIssue('');
        }
      })
      .catch(() => {
        if (generation.current === version) setIssue('上次使用的镜像已不可用，请重新选择。');
      });
    return () => {
      generation.current++;
    };
  }, [historyKey, onPath]);
  useEffect(() => {
    let active = true;
    const off = subscribeEvent('imager-card-progress', (p) => {
      if (active) setProgress(p);
    }).catch(() => () => {});
    return () => {
      active = false;
      void off.then((stop) => stop());
    };
  }, []);
  async function list() {
    setIssue('');
    setCard(null);
    setCards([]);
    const found = await invokeCommand('imager_card_list', {});
    setCards(found);
    if (!found.length) setIssue('未发现可写的 TF / SD 卡，请插入读卡器后重新检测。');
  }
  useEffect(() => {
    if (step === 2 || step === 3) {
      setFinished(false);
      setProgress({ phase: '', bytes: 0 });
    }
  }, [step]);
  return (
    <section className="card-writer">
      <header className="image-desk-title">
        <h2>
          {step === 2
            ? '选择 SD 整盘镜像'
            : step === 3
              ? '选择读卡器'
              : finished
                ? '写卡完成'
                : '确认写入 TF / SD 卡'}
        </h2>
        <div className="image-target-summary">
          <strong>{model}</strong>
          <span>TF / SD</span>
          <span>电脑读卡器</span>
        </div>
      </header>
      {step === 2 && (
        <>
          <p>
            选择板卡可启动的 .img、.raw 或 .wic 整盘镜像，也支持 .img 和 .wic 的 gzip、xz 压缩包。
          </p>
          <button
            onClick={() =>
              void run('分析卡镜像', async () => {
                const path = await open({
                  multiple: false,
                  directory: false,
                  filters: [{ name: 'SD 整盘镜像', extensions: ['img', 'raw', 'wic', 'gz', 'xz'] }],
                });
                if (typeof path !== 'string') return;
                ++generation.current;
                setIssue('');
                setImage(null);
                setCard(null);
                onPath('');
                const file = await invokeCommand('imager_card_image', { path });
                setImage(file);
                onPath(path);
                try {
                  localStorage.setItem(historyKey, path);
                } catch {
                  /* The selected file still works for this session. */
                }
              })
            }
          >
            {image ? '更换镜像' : '选择镜像'}
          </button>
          {issue && <p role="status">{issue}</p>}
          {image && (
            <p>
              {image.path.split(/[\\/]/).pop()} · 解压后 {size(image.bytes)}
            </p>
          )}
          <footer>
            <button
              className="v2-primary"
              disabled={!image || !!busy}
              onClick={() => {
                onStep(3);
                void run('检测读卡器', list);
              }}
            >
              下一步：选择读卡器
            </button>
          </footer>
        </>
      )}
      {step === 3 && (
        <>
          <p>选择插入 TF / SD 卡的读卡器。</p>
          <button onClick={() => void run('检测读卡器', list)}>重新检测</button>
          {cards.map((d) => (
            <label className="reader-row" key={d.key}>
              <input
                type="radio"
                name="host-card"
                checked={card?.key === d.key}
                disabled={!image || image.bytes > d.bytes || image.bytes % d.sector !== 0}
                onChange={() => setCard(d)}
              />
              <strong>{d.name}</strong>
              <span>
                {size(d.bytes)} · 磁盘 {d.key}
              </span>
              {image && image.bytes > d.bytes && <small>容量不足</small>}
            </label>
          ))}
          <p role="status">{issue}</p>
          <footer>
            <button onClick={() => onStep(2)}>上一步</button>
            <button className="v2-primary" disabled={!card || !image} onClick={() => onStep(4)}>
              下一步：确认写卡
            </button>
          </footer>
        </>
      )}
      {step === 4 && image && card && (
        <>
          <p>
            {card.name} · 磁盘 {card.key} · {size(card.bytes)}
          </p>
          <p>
            {image.path.split(/[\\/]/).pop()} · {size(image.bytes)}
          </p>
          {finished ? (
            <p role="status">已回读写入范围，SHA-256 与源镜像一致。</p>
          ) : (
            <>
              <p>写入将覆盖这张卡的分区表和数据。</p>
              {busy && (
                <div role="status">
                  {progress.phase || '正在申请写卡权限'}{' '}
                  <progress max={image.bytes} value={progress.bytes} />
                </div>
              )}
              <footer>
                <button onClick={() => onStep(3)}>上一步</button>
                <button
                  className="v2-primary"
                  disabled={!!busy}
                  onClick={() =>
                    void run('写卡和回读校验', async () => {
                      setProgress({ phase: '', bytes: 0 });
                      await invokeCommand('imager_card_write', { card, image });
                      setFinished(true);
                    })
                  }
                >
                  擦写所选卡
                </button>
              </footer>
            </>
          )}
          {finished && (
            <button
              onClick={() => {
                onStep(3);
                void run('检测读卡器', list);
              }}
            >
              选择另一张卡
            </button>
          )}
        </>
      )}
      {busy && step !== 4 && <p role="status">{busy}…</p>}
    </section>
  );
}
