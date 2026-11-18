import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { confirm, open, save } from '@tauri-apps/plugin-dialog';
import { LynxIcon } from '../../CoreUI';
import { invokeCommand, subscribeEvent } from '../../Platform/IPC';
import type { AdbTransferSnapshot, WorkbenchDevice } from '../../Platform/IPC/Commands';
import { formatErrorMessage } from '../../Utils';
import './AdbFileTransfer.css';
import { listDirectory } from '../../Library/ADB/AdbLibrary';
import type { AdbDirectoryListing } from '../../Library/ADB/types';

interface Props {
  device: WorkbenchDevice;
  isActive: boolean;
  compact?: boolean;
  online?: boolean;
  onOpenSettings?: () => void;
  onDetect?: () => void;
}

function fileName(path: string) {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? '';
}

function bytes(value: number) {
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KiB`;
  return `${(value / 1024 / 1024).toFixed(1)} MiB`;
}

function statusLabel(status: string) {
  return (
    {
      queued: '等待中',
      running: '传输中',
      success: '已完成',
      failed: '失败',
      cancelled: '已取消',
    }[status] ?? status
  );
}

export const AdbFileTransfer: React.FC<Props> = ({
  device,
  isActive,
  compact = false,
  online,
  onOpenSettings,
  onDetect,
}) => {
  const [uploadLocal, setUploadLocal] = useState('');
  const [uploadRemote, setUploadRemote] = useState('/data/local/tmp/');
  const [downloadRemote, setDownloadRemote] = useState('');
  const [overwrite, setOverwrite] = useState(false);
  const [tasks, setTasks] = useState<AdbTransferSnapshot[]>([]);
  const [error, setError] = useState('');
  const [path, setPath] = useState('/');
  const [listing, setListing] = useState<AdbDirectoryListing | null>(null);
  const [listError, setListError] = useState('');
  const [revision, setRevision] = useState(0);
  const [transferForm, setTransferForm] = useState<'upload' | 'download' | null>(null);
  const transferPending = useRef(false);
  const [submitting, setSubmitting] = useState(false);
  const mounted = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  useEffect(() => {
    let cancelled = false;
    setListing(null);
    setListError('');
    if (online && isActive && device.adbSerial) {
      void listDirectory(device.adbSerial, path)
        .then((result) => {
          if (!cancelled) setListing(result);
        })
        .catch((cause) => {
          if (!cancelled) setListError(formatErrorMessage(cause));
        });
    }
    return () => {
      cancelled = true;
    };
  }, [device.adbSerial, isActive, online, path, revision]);

  const refresh = useCallback(async () => {
    if (!device.id) return;
    try {
      setTasks(await invokeCommand('adb_transfer_list', { deviceId: device.id }));
    } catch (value) {
      setError(formatErrorMessage(value));
    }
  }, [device.id]);

  useEffect(() => {
    if (isActive) void refresh();
  }, [isActive, refresh]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void subscribeEvent('adb-transfer-progress', (snapshot) => {
      if (disposed || snapshot.deviceId !== device.id) return;
      setTasks((current) => {
        const next = current.filter((task) => task.taskId !== snapshot.taskId);
        return [snapshot, ...next].sort((a, b) => b.startedAtMs - a.startedAtMs);
      });
    }).then((value) => {
      if (disposed) value();
      else unlisten = value;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [device.id]);

  const running = useMemo(
    () => tasks.some((task) => task.status === 'queued' || task.status === 'running'),
    [tasks]
  );
  const visibleTasks = tasks.slice(0, compact ? 3 : 8);

  const chooseUpload = async () => {
    const selected = await open({ multiple: false, directory: false, title: '选择要上传的文件' });
    if (typeof selected !== 'string' || !mounted.current) return;
    setUploadLocal(selected);
    if (uploadRemote.endsWith('/')) setUploadRemote(`${uploadRemote}${fileName(selected)}`);
  };

  const start = async (direction: 'upload' | 'download', localPath: string, remotePath: string) => {
    if (
      !device.id ||
      !device.adbSerial ||
      online === false ||
      running ||
      transferPending.current ||
      !mounted.current
    )
      return;
    transferPending.current = true;
    setSubmitting(true);
    setError('');
    try {
      const snapshot = await invokeCommand('adb_transfer_start', {
        request: { deviceId: device.id, direction, localPath, remotePath, overwrite },
      });
      setTasks((current) =>
        current.some((t) => t.taskId === snapshot.taskId) ? current : [snapshot, ...current]
      );
    } catch (value) {
      const detail = formatErrorMessage(value);
      if (/ADB_(REMOTE|LOCAL)_EXISTS/.test(detail) && !overwrite) {
        const approved = await confirm('目标文件已经存在，是否允许本次传输覆盖？', {
          title: '确认覆盖文件',
          kind: 'warning',
        });
        if (approved && mounted.current) {
          try {
            const snapshot = await invokeCommand('adb_transfer_start', {
              request: { deviceId: device.id, direction, localPath, remotePath, overwrite: true },
            });
            setTasks((current) =>
              current.some((t) => t.taskId === snapshot.taskId) ? current : [snapshot, ...current]
            );
            return;
          } catch (retryError) {
            setError(formatErrorMessage(retryError));
            return;
          }
        }
      }
      setError(detail);
    } finally {
      transferPending.current = false;
      if (mounted.current) setSubmitting(false);
    }
  };

  const chooseDownload = async () => {
    if (!downloadRemote.trim()) return;
    const target = await save({
      title: '保存从设备下载的文件',
      defaultPath: fileName(downloadRemote) || 'download.bin',
    });
    if (target) await start('download', target, downloadRemote.trim());
  };

  if (online !== undefined) {
    const blocked = !online || !device.adbSerial || running || submitting;
    const crumbs = [
      '/',
      ...path
        .split('/')
        .filter(Boolean)
        .map((_, index, folders) => '/' + folders.slice(0, index + 1).join('/')),
    ];
    const browse = (next: string) => {
      setPath(next);
      setDownloadRemote('');
    };
    const run = (action: () => Promise<void>) => {
      void action().catch((e) => setError(formatErrorMessage(e)));
    };
    return (
      <section className="adv-files" data-testid="adb-file-transfer" data-device-id={device.id}>
        <div className="adv-page-heading">
          <h2>文件传输</h2>
          <span>{device.name} · ADB</span>
        </div>
        <h3>ADB 文件传输</h3>
        {!online && (
          <div className="adv-warning" role="status">
            {device.adbSerial ? `未检测到已绑定设备 ${device.adbSerial}` : '尚未绑定 ADB 设备。'}
            {device.adbSerial ? (
              <button onClick={onDetect}>重新检测接口</button>
            ) : (
              <button onClick={onOpenSettings}>设备设置</button>
            )}
          </div>
        )}
        <div className="adv-file-buttons">
          <button
            className="adv-primary"
            disabled={blocked}
            onClick={() => {
              setTransferForm('upload');
              setUploadLocal('');
              setUploadRemote(path.replace(/\/?$/, '/'));
            }}
          >
            发送文件至设备
          </button>
          <button disabled={blocked} onClick={() => setTransferForm('download')}>
            从设备接收
          </button>
          <button disabled={!online} onClick={() => setRevision((n) => n + 1)}>
            刷新目录
          </button>
        </div>
        {transferForm && (
          <form
            className="adv-transfer-form"
            onSubmit={(e) => {
              e.preventDefault();
              run(
                transferForm === 'upload'
                  ? () => start('upload', uploadLocal, uploadRemote.trim())
                  : chooseDownload
              );
            }}
          >
            {transferForm === 'upload' ? (
              <>
                <label>
                  本机文件{' '}
                  <button type="button" disabled={blocked} onClick={() => run(chooseUpload)}>
                    {uploadLocal || '选择文件'}
                  </button>
                </label>
                <label>
                  设备保存路径{' '}
                  <input value={uploadRemote} onChange={(e) => setUploadRemote(e.target.value)} />
                </label>
              </>
            ) : (
              <label>
                设备文件路径{' '}
                <input
                  value={downloadRemote}
                  onChange={(e) => setDownloadRemote(e.target.value)}
                  placeholder="/var/log/boot.log"
                />
              </label>
            )}
            <label>
              <input
                type="checkbox"
                checked={overwrite}
                onChange={(e) => setOverwrite(e.target.checked)}
              />
              允许覆盖目标文件
            </label>
            <div>
              <button
                className="adv-primary"
                disabled={
                  blocked ||
                  (transferForm === 'upload'
                    ? !uploadLocal || !uploadRemote.trim()
                    : !downloadRemote.trim())
                }
              >
                {' '}
                {transferForm === 'upload' ? '确认发送' : '选择保存位置并接收'}
              </button>
              <button type="button" onClick={() => setTransferForm(null)}>
                收起
              </button>
            </div>
          </form>
        )}
        {online && (
          <div className="adv-directory">
            <nav aria-label="目录路径">
              {crumbs.map((folder, i) => (
                <button
                  key={folder}
                  aria-current={folder === path ? 'location' : undefined}
                  onClick={() => browse(folder)}
                  style={{ paddingInlineStart: Math.min(i, 6) * 8 + 8 + 'px' }}
                >
                  <LynxIcon name="folder" size={16} />
                  {fileName(folder) || '/'}
                </button>
              ))}
            </nav>
            <div>
              <h3>{path}</h3>
              <table>
                <thead>
                  <tr>
                    <th>文件名</th>
                    <th>大小</th>
                    <th>修改时间</th>
                  </tr>
                </thead>
                <tbody>
                  {listing?.items.map((item) => (
                    <tr key={item.path}>
                      <td>
                        <button
                          type="button"
                          aria-pressed={!item.is_directory && downloadRemote === item.path}
                          onClick={() => {
                            if (item.is_directory) browse(item.path);
                            else {
                              setDownloadRemote(item.path);
                              setTransferForm('download');
                            }
                          }}
                        >
                          <LynxIcon name={item.is_directory ? 'folder' : 'document'} size={16} />
                          {item.name}
                        </button>
                      </td>
                      <td>{item.is_directory ? '—' : bytes(item.size)}</td>
                      <td>
                        {item.modified_time
                          ? new Date(item.modified_time * 1000).toLocaleString()
                          : '—'}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
              {listError ? (
                <p role="alert">{listError}</p>
              ) : !listing || !listing.items.length ? (
                <p>{listing ? '目录为空。' : '正在读取目录…'}</p>
              ) : null}
            </div>
          </div>
        )}
        <h3>传输队列</h3>
        {error && (
          <p className="adv-warning" role="alert">
            {error}
          </p>
        )}
        <table>
          <thead>
            <tr>
              <th>方向</th>
              <th>文件</th>
              <th>大小</th>
              <th>进度</th>
            </tr>
          </thead>
          <tbody>
            {visibleTasks.map((task) => (
              <tr key={task.taskId}>
                <td>{task.direction === 'upload' ? '本机 → 设备' : '设备 → 本机'}</td>
                <td>{task.fileName}</td>
                <td>
                  {bytes(task.transferredBytes)}
                  {task.totalBytes != null && ' / ' + bytes(task.totalBytes)}
                </td>
                <td>
                  {statusLabel(task.status)} {task.errorCode}
                  {(task.status === 'queued' || task.status === 'running') && (
                    <button
                      onClick={() =>
                        run(async () => {
                          await invokeCommand('adb_transfer_cancel', { taskId: task.taskId });
                          await refresh();
                        })
                      }
                    >
                      取消
                    </button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {!visibleTasks.length && <p className="adv-empty-message">当前设备暂无传输记录。</p>}
      </section>
    );
  }

  return (
    <section
      className={`adb-file-transfer ${compact ? 'is-compact' : ''}`}
      data-testid="adb-file-transfer"
      data-device-id={device.id}
    >
      <header className="adb-transfer-heading">
        <div className="adb-transfer-title">
          <span className="adb-transfer-title-icon">
            <LynxIcon name="phone" size={20} />
          </span>
          <div>
            <small>ADB FILE TRANSFER</small>
            <strong>设备文件传输</strong>
            <p>在本机与当前 Linux 单板之间安全传送单个文件</p>
          </div>
        </div>
        <div className="adb-transfer-binding">
          <span className={device.adbSerial ? 'is-online' : ''}>
            <i /> {device.adbSerial ? 'ADB 已绑定' : 'ADB 未绑定'}
          </span>
          <strong title={device.adbSerial}>{device.adbSerial || device.name}</strong>
        </div>
      </header>

      {!device.adbSerial ? (
        <div className="adb-transfer-unbound">
          <LynxIcon name="disconnect" size={20} />
          <div>
            <strong>当前单板尚未绑定 ADB</strong>
            <span>请在“设备设置”中选择这块板对应的 ADB 序列号。</span>
          </div>
        </div>
      ) : (
        <div className="adb-transfer-layout">
          <div className="adb-transfer-actions">
            <article className="adb-transfer-action is-upload">
              <header>
                <span>
                  <LynxIcon name="upload" size={20} />
                </span>
                <div>
                  <strong>上传到设备</strong>
                  <small>本机文件 → {device.name}</small>
                </div>
              </header>
              <label>
                <span>本机文件</span>
                <button
                  className="adb-transfer-file-picker"
                  type="button"
                  onClick={chooseUpload}
                  disabled={running}
                  title={uploadLocal}
                >
                  <LynxIcon name="document" size={16} />
                  <b>{uploadLocal ? fileName(uploadLocal) : '选择一个文件'}</b>
                  <em>{uploadLocal ? '重新选择' : '浏览'}</em>
                </button>
              </label>
              <label>
                <span>设备保存路径</span>
                <input
                  aria-label="远端上传路径"
                  value={uploadRemote}
                  onChange={(event) => setUploadRemote(event.target.value)}
                  placeholder="/data/local/tmp/file.bin"
                />
              </label>
              <button
                className="adb-transfer-primary"
                type="button"
                onClick={() => void start('upload', uploadLocal, uploadRemote.trim())}
                disabled={running || !uploadLocal || !uploadRemote.trim()}
              >
                <LynxIcon name="upload" size={16} /> 开始上传
              </button>
            </article>

            <article className="adb-transfer-action is-download">
              <header>
                <span>
                  <LynxIcon name="download" size={20} />
                </span>
                <div>
                  <strong>从设备下载</strong>
                  <small>{device.name} → 本机文件</small>
                </div>
              </header>
              <label>
                <span>设备文件路径</span>
                <input
                  aria-label="远端下载文件"
                  value={downloadRemote}
                  onChange={(event) => setDownloadRemote(event.target.value)}
                  placeholder="/data/local/tmp/file.bin"
                />
              </label>
              <div className="adb-transfer-download-hint">
                <LynxIcon name="info" size={16} />
                <span>开始时再选择本机保存位置，避免把路径保存到配置或记录中。</span>
              </div>
              <button
                className="adb-transfer-primary"
                type="button"
                onClick={() => void chooseDownload()}
                disabled={running || !downloadRemote.trim()}
              >
                <LynxIcon name="download" size={16} /> 选择位置并下载
              </button>
            </article>

            <label className="adb-transfer-overwrite">
              <input
                type="checkbox"
                checked={overwrite}
                onChange={(event) => setOverwrite(event.target.checked)}
              />
              <span>
                <strong>允许覆盖目标文件</strong>
                <small>默认关闭；发现同名文件时也会再次向你确认。</small>
              </span>
            </label>
          </div>

          <aside className="adb-transfer-history">
            <header>
              <div>
                <small>TRANSFER QUEUE</small>
                <strong>传输进度</strong>
              </div>
              <span>{running ? '正在传输' : `${visibleTasks.length} 条记录`}</span>
            </header>
            {error && <p className="adb-transfer-error">{error}</p>}
            <div className="adb-transfer-tasks">
              {visibleTasks.map((task) => {
                const percent = task.totalBytes
                  ? Math.min(100, (task.transferredBytes / task.totalBytes) * 100)
                  : 0;
                return (
                  <article key={task.taskId} className={`is-${task.status}`}>
                    <div className="adb-transfer-task-heading">
                      <span className="adb-transfer-task-icon">
                        <LynxIcon
                          name={task.direction === 'upload' ? 'upload' : 'download'}
                          size={16}
                        />
                      </span>
                      <div>
                        <strong title={task.fileName}>{task.fileName}</strong>
                        <small>{task.direction === 'upload' ? '上传到设备' : '下载到本机'}</small>
                      </div>
                      <em>{statusLabel(task.status)}</em>
                    </div>
                    <div className="adb-transfer-progress-row">
                      <progress
                        max={100}
                        value={percent}
                        aria-label={`${task.fileName} 传输进度`}
                      />
                      <b>{Math.round(percent)}%</b>
                    </div>
                    <div className="adb-transfer-task-meta">
                      <small>
                        {bytes(task.transferredBytes)} /{' '}
                        {task.totalBytes ? bytes(task.totalBytes) : '计算中'}
                        {task.bytesPerSecond ? ` · ${bytes(task.bytesPerSecond)}/s` : ''}
                      </small>
                      {(task.status === 'queued' || task.status === 'running') && (
                        <button
                          type="button"
                          onClick={() =>
                            void invokeCommand('adb_transfer_cancel', { taskId: task.taskId })
                          }
                        >
                          取消
                        </button>
                      )}
                    </div>
                    {task.errorCode && <p>{task.errorCode}</p>}
                  </article>
                );
              })}
              {!visibleTasks.length && (
                <div className="adb-transfer-empty">
                  <LynxIcon name="history" size={20} />
                  <strong>还没有传输任务</strong>
                  <span>选择左侧上传或下载，进度会显示在这里。</span>
                </div>
              )}
            </div>
          </aside>
        </div>
      )}
      {error && !device.adbSerial && <p className="adb-transfer-error">{error}</p>}
    </section>
  );
};

export default AdbFileTransfer;
