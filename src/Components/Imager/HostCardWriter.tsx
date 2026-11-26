import { useEffect, useRef, useState } from 'react';
import { LynxIcon } from '../../CoreUI';
import './HostCardWriter.css';
import { formatErrorMessage } from '../../Utils';
import { open } from '@tauri-apps/plugin-dialog';
import { invokeCommand, subscribeEvent } from '../../Platform/IPC';
import type { ImagerCard, ImagerCardImage } from '../../Platform/IPC/contracts/Workbench';

const size = (n: number) =>
  n < 1024 ** 3 ? (n / 1024 ** 2).toFixed(1) + ' MiB' : (n / 1024 ** 3).toFixed(2) + ' GiB';
interface Props {
  model: string;
  step: number;
  onStep: (step: 1 | 2 | 3 | 4) => void;
  run: (label: string, work: () => Promise<void>) => Promise<void>;
  onPath: (path: string) => void;
  busy: string;
}
export function HostCardWriter({ model, step, onStep, run, onPath, busy }: Props) {
  const [image, setImage] = useState<ImagerCardImage | null>(null);
  const [cards, setCards] = useState<ImagerCard[]>([]);
  const [card, setCard] = useState<ImagerCard | null>(null);
  const [finished, setFinished] = useState(false);
  const [writeError, setWriteError] = useState('');
  const [progress, setProgress] = useState({ phase: '', bytes: 0 });
  const [issue, setIssue] = useState('');
  const generation = useRef(0);
  const [scanRevision, setScanRevision] = useState(0);
  const [scanning, setScanning] = useState(false);
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
        if (generation.current === version) {
          onPath('');
          setIssue('上次使用的镜像已不可用，请重新选择。');
        }
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
  useEffect(() => {
    if (step !== 3) return;
    let current = true;
    setScanning(true);
    setIssue('');
    setCard(null);
    setCards([]);
    invokeCommand('imager_card_list', {})
      .then((found) => {
        if (!current) return;
        setCards(found);
        if (!found.length) setIssue('未发现可写的 TF / SD 卡，请插入读卡器后重新检测。');
      })
      .catch((e) => {
        if (current) setIssue('读卡器检测失败：' + formatErrorMessage(e));
      })
      .finally(() => {
        if (current) setScanning(false);
      });
    return () => {
      current = false;
    };
  }, [step, scanRevision]);
  useEffect(() => {
    if (step === 2 || step === 3) {
      setFinished(false);
      setWriteError('');
      setProgress({ phase: '', bytes: 0 });
    }
  }, [step]);
  async function chooseImage() {
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
  }
  const filename = image?.path.split(/[\\/]/).pop();
  const writing = step === 4 && !!busy;
  const heading =
    step === 2
      ? '选择 SD 整盘镜像'
      : step === 3
        ? '选择读卡器'
        : finished
          ? '写卡完成'
          : writing
            ? '正在写卡与校验'
            : writeError
              ? '写卡未完成'
              : '确认写入 TF / SD 卡';
  return (
    <section className="card-writer">
      <header className="image-desk-title">
        <h2>{heading}</h2>
        <div className="image-target-summary">
          <strong>{model}</strong>
          <span>TF / SD</span>
          <span>电脑读卡器</span>
        </div>
      </header>

      {step === 2 && (
        <div className="card-image-slot">
          <div className="disk-file-symbol" aria-hidden="true">
            <LynxIcon name="document" size={40} />
            {filename?.includes('.') && <b>{filename.split('.').pop()?.toUpperCase()}</b>}
          </div>
          <div>
            <small>整盘镜像</small>
            <h3 title={image?.path}>{filename || '选择要写入卡中的文件'}</h3>
            <p>
              {image
                ? '解压后 ' + size(image.bytes)
                : '支持 IMG、RAW、WIC，以及 IMG / WIC 的 XZ、GZ 压缩包。'}
            </p>
          </div>
          <button disabled={!!busy} onClick={() => void run('分析卡镜像', chooseImage)}>
            <LynxIcon name="folder" size={16} />
            {image ? '更换镜像' : '选择镜像'}
          </button>
        </div>
      )}

      {step === 3 && (
        <div className="reader-list">
          <div className="reader-list-title">
            <h3>可写入的存储卡</h3>
            <button disabled={scanning || !!busy} onClick={() => setScanRevision((i) => i + 1)}>
              <LynxIcon name="refresh" size={16} />
              重新检测
            </button>
          </div>
          {cards.map((d) => {
            const reason =
              image &&
              (image.bytes > d.bytes
                ? '容量不足'
                : image.bytes % d.sector !== 0
                  ? '扇区大小不匹配'
                  : '');
            return (
              <label className="reader-row" key={d.key}>
                <LynxIcon name="plug" size={20} />
                <span>
                  <strong>{d.name}</strong>
                  <small>物理磁盘 {d.key}</small>
                </span>
                <span>{reason || size(d.bytes)}</span>
                <input
                  type="radio"
                  name="host-card"
                  checked={card?.key === d.key}
                  disabled={!!busy || !image || !!reason}
                  onChange={() => setCard(d)}
                />
              </label>
            );
          })}
          {!cards.length && (
            <div className="reader-empty">
              <LynxIcon name="plug" size={24} />
              <p role="status">{issue || '正在查找读卡器…'}</p>
            </div>
          )}
        </div>
      )}

      {step === 4 && card && image && (
        <>
          <div className="card-destination">
            <div className="card-specimen" aria-hidden="true">
              <div className="sd-body">
                <span className="sd-label">SD</span>
                <span className="sd-stripe" />
                <span className="sd-size">{size(card.bytes)}</span>
              </div>
            </div>
            <div className="card-destination-copy">
              <small>目标存储卡</small>
              <h3>{card.name}</h3>
              <dl>
                <div>
                  <dt>物理磁盘</dt>
                  <dd>{card.key}</dd>
                </div>
                <div>
                  <dt>容量</dt>
                  <dd>{size(card.bytes)}</dd>
                </div>
                <div>
                  <dt>逻辑扇区</dt>
                  <dd>{card.sector} B</dd>
                </div>
              </dl>
            </div>
          </div>
          {finished ? (
            <p className="card-notice success" role="status">
              <LynxIcon name="check-circle" size={20} />
              已回读写入范围，SHA-256 与源镜像一致。
            </p>
          ) : writeError ? (
            <div className="card-notice failure" role="alert">
              <LynxIcon name="warning" size={20} />
              <div>
                <strong>写卡未完成，请重新检测存储卡</strong>
                <p>{writeError}</p>
                <small>卡内数据可能已被部分覆盖。检查连接后重新选择目标卡，再确认写入。</small>
              </div>
            </div>
          ) : writing ? (
            <div className="card-progress" role="status">
              <strong>{progress.phase || '正在申请写卡权限'}</strong>
              <progress
                aria-label={progress.phase || '正在申请写卡权限'}
                max={image.bytes}
                value={progress.phase ? progress.bytes : undefined}
              />
            </div>
          ) : (
            <p className="card-notice warning">
              <LynxIcon name="warning" size={20} />
              写入将覆盖这张卡上的分区表和数据。
            </p>
          )}
        </>
      )}

      {image && (
        <dl className="card-file-details">
          {step === 2 && (
            <div>
              <dt>本地路径</dt>
              <dd>{image.path}</dd>
            </div>
          )}
          {step !== 2 && (
            <div>
              <dt>源镜像</dt>
              <dd title={image.path}>{filename}</dd>
            </div>
          )}
          {step !== 2 && (
            <div>
              <dt>写入大小</dt>
              <dd>{size(image.bytes)}</dd>
            </div>
          )}
          <div>
            <dt>SHA-256</dt>
            <dd>
              <code>{image.sha256}</code>
            </dd>
          </div>
        </dl>
      )}
      {step === 2 && (busy || issue) && <p role="status">{busy ? busy + '…' : issue}</p>}

      <footer>
        {!finished && (
          <button disabled={!!busy} onClick={() => onStep(step === 4 ? 3 : step === 3 ? 2 : 1)}>
            上一步
          </button>
        )}
        {step === 2 && (
          <button className="v2-primary" disabled={!image || !!busy} onClick={() => onStep(3)}>
            下一步：选择读卡器
            <LynxIcon name="chevron-right" size={16} />
          </button>
        )}
        {step === 3 && (
          <button
            className="v2-primary"
            disabled={!card || !image || scanning || !!busy}
            onClick={() => onStep(4)}
          >
            下一步：确认写卡
            <LynxIcon name="chevron-right" size={16} />
          </button>
        )}
        {step === 4 &&
          image &&
          card &&
          (finished ? (
            <button onClick={() => onStep(3)}>选择另一张卡</button>
          ) : writeError ? (
            <button className="v2-primary" onClick={() => onStep(3)}>
              重新检测存储卡
            </button>
          ) : (
            <button
              className="v2-primary"
              disabled={!!busy}
              onClick={() =>
                void run('写卡和回读校验', async () => {
                  setProgress({ phase: '', bytes: 0 });
                  setWriteError('');
                  try {
                    await invokeCommand('imager_card_write', { card, image });
                    setFinished(true);
                  } catch (error) {
                    setWriteError(formatErrorMessage(error));
                  }
                })
              }
            >
              <LynxIcon name="download" size={16} />
              擦写所选卡
            </button>
          ))}
      </footer>
    </section>
  );
}
