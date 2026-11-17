import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { confirm, open, save } from '@tauri-apps/plugin-dialog';
import { LynxIcon } from '../../CoreUI';
import { invokeCommand, subscribeEvent } from '../../Platform/IPC';
import type { AdbTransferSnapshot, WorkbenchDevice } from '../../Platform/IPC/Commands';
import { formatErrorMessage } from '../../Utils';
import './AdbFileTransfer.css';

interface Props {
  device: WorkbenchDevice;
  isActive: boolean;
  compact?: boolean;
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

export const AdbFileTransfer: React.FC<Props> = ({ device, isActive, compact = false }) => {
  const [uploadLocal, setUploadLocal] = useState('');
  const [uploadRemote, setUploadRemote] = useState('/data/local/tmp/');
  const [downloadRemote, setDownloadRemote] = useState('');
  const [overwrite, setOverwrite] = useState(false);
  const [tasks, setTasks] = useState<AdbTransferSnapshot[]>([]);
  const [error, setError] = useState('');

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
    if (typeof selected !== 'string') return;
    setUploadLocal(selected);
    if (uploadRemote.endsWith('/')) setUploadRemote(`${uploadRemote}${fileName(selected)}`);
  };

  const start = async (direction: 'upload' | 'download', localPath: string, remotePath: string) => {
    if (!device.id || !device.adbSerial || running) return;
    setError('');
    try {
      const snapshot = await invokeCommand('adb_transfer_start', {
        request: { deviceId: device.id, direction, localPath, remotePath, overwrite },
      });
      setTasks((current) => [snapshot, ...current]);
    } catch (value) {
      const detail = formatErrorMessage(value);
      if (/ADB_(REMOTE|LOCAL)_EXISTS/.test(detail) && !overwrite) {
        const approved = await confirm('目标文件已经存在，是否允许本次传输覆盖？', {
          title: '确认覆盖文件',
          kind: 'warning',
        });
        if (approved) {
          setOverwrite(true);
          try {
            const snapshot = await invokeCommand('adb_transfer_start', {
              request: { deviceId: device.id, direction, localPath, remotePath, overwrite: true },
            });
            setTasks((current) => [snapshot, ...current]);
            return;
          } catch (retryError) {
            setError(formatErrorMessage(retryError));
            return;
          }
        }
      }
      setError(detail);
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
