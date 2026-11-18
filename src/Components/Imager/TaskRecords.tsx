import { useEffect, useRef, useState } from 'react';
import { save } from '@tauri-apps/plugin-dialog';
import { writeTextFile } from '@tauri-apps/plugin-fs';
import { formatErrorMessage } from '../../Utils/Error';
import { LynxIcon } from '../../CoreUI/LynxIcon/LynxIcon';
import { loadRecordedBoot } from './BootEvidence';
import type { BootReport } from './writeEvidence';
import {
  invokeCommand,
  type WorkbenchDevice,
  type WorkbenchTaskHistory,
  type WorkbenchWorkflowStep,
} from '../../Platform/IPC';

const statusNames: Record<string, string> = {
  queued: '等待',
  running: '进行中',
  failed: '失败',
  success: '完成',
  succeeded: '完成',
  completed: '完成',
  cancelled: '已取消',
  interrupted: '中断',
  pending: '等待',
};
const statusText = (value: string) => statusNames[value] ?? value;
const date = (value: number) => new Date(value * 1000).toLocaleString();
export function TaskRecords({ onClose }: { onClose: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [rows, setRows] = useState<WorkbenchTaskHistory[]>([]);
  const [devices, setDevices] = useState<WorkbenchDevice[]>([]);
  const [steps, setSteps] = useState<WorkbenchWorkflowStep[]>([]);
  const [status, setStatus] = useState('');
  const [deviceId, setDeviceId] = useState('');
  const [taskId, setTaskId] = useState('');
  const [version, refresh] = useState(0);
  const [ready, setReady] = useState(false);
  const [loadError, setLoadError] = useState('');
  const [detailError, setDetailError] = useState('');
  const [bootReport, setBootReport] = useState<BootReport | null>(null);
  useEffect(() => {
    const el = dialog.current;
    el?.showModal();
    return () => el?.close();
  }, []);
  useEffect(() => {
    let current = true;
    setReady(false);
    setLoadError('');
    Promise.all([
      invokeCommand('workbench_list_tasks', { limit: 200 }),
      invokeCommand('workbench_list_devices'),
    ])
      .then(([tasks, boards]) => {
        if (current) {
          setRows(tasks);
          setDevices(boards);
        }
      })
      .catch((error) => {
        if (current) setLoadError(String(error));
      })
      .finally(() => {
        if (current) setReady(true);
      });
    return () => {
      current = false;
    };
  }, [version]);
  const visible = rows.filter(
    (row) => (!status || row.status === status) && (!deviceId || String(row.deviceId) === deviceId)
  );
  const selected = visible.find((row) => row.taskId === taskId) ?? visible[0];
  const selectedId = selected?.taskId;
  useEffect(() => {
    let current = true;
    setSteps([]);
    setDetailError('');
    setBootReport(null);
    if (selectedId && selected) {
      loadRecordedBoot(selected)
        .then((boot) => {
          if (current) setBootReport(boot);
        })
        .catch((e) => {
          if (current) setDetailError('读取启动验收失败：' + String(e));
        });
      invokeCommand('workbench_list_workflow_steps', { taskId: selectedId })
        .then((result) => {
          if (current) setSteps(result);
        })
        .catch((error) => {
          if (current) setDetailError(String(error));
        });
    }
    return () => {
      current = false;
    };
  }, [selectedId, selected, version]);
  async function exportRecord() {
    if (!selected) return;
    try {
      const [detail, boot] = await Promise.all([
        invokeCommand('workbench_list_workflow_steps', { taskId: selected.taskId }),
        loadRecordedBoot(selected).catch((e) => ({ readError: formatErrorMessage(e) })),
      ]);
      const path = await save({
        defaultPath: 'lynx-task.json',
        filters: [{ name: 'JSON', extensions: ['json'] }],
      });
      if (path)
        await writeTextFile(
          path,
          JSON.stringify(
            {
              task: selected,
              steps: detail,
              manualBootAcceptance: boot,
            },
            null,
            2
          )
        );
    } catch (error) {
      setDetailError(String(error));
    }
  }
  const deviceName = (id: number | null) =>
    devices.find((d) => d.id === id)?.name ?? (id == null ? '—' : '设备 ' + id);
  return (
    <dialog
      ref={dialog}
      className="imager-records"
      aria-labelledby="imager-records-title"
      onCancel={onClose}
    >
      <header>
        <h2 id="imager-records-title">任务记录</h2>
        <button aria-label="关闭任务记录" onClick={onClose}>
          <LynxIcon name="dismiss" size={16} />
        </button>
      </header>
      <div className="imager-records-content">
        <div className="imager-records-filters">
          <label>
            设备
            <select value={deviceId} onChange={(e) => setDeviceId(e.target.value)}>
              <option value="">全部设备</option>
              {[...new Set(rows.map((r) => r.deviceId).filter((id) => id != null))].map((id) => (
                <option key={id} value={id}>
                  {deviceName(id)}
                </option>
              ))}
            </select>
          </label>
          <label>
            结果
            <select value={status} onChange={(e) => setStatus(e.target.value)}>
              <option value="">全部结果</option>
              {[...new Set(rows.map((r) => r.status))].map((s) => (
                <option key={s} value={s}>
                  {statusText(s)}
                </option>
              ))}
            </select>
          </label>
          <button onClick={() => refresh((v) => v + 1)}>刷新</button>
        </div>
        {loadError && <p role="alert">{loadError}</p>}
        <table>
          <thead>
            <tr>
              <th>开始时间</th>
              <th>设备</th>
              <th>任务</th>
              <th>阶段</th>
              <th>结果</th>
            </tr>
          </thead>
          <tbody>
            {visible.map((row) => (
              <tr key={row.taskId} className={selectedId === row.taskId ? 'is-selected' : ''}>
                <td>
                  <button onClick={() => setTaskId(row.taskId)}>{date(row.startedAt)}</button>
                </td>
                <td>{deviceName(row.deviceId)}</td>
                <td>{row.kind}</td>
                <td>{row.phase || '—'}</td>
                <td>{statusText(row.status)}</td>
              </tr>
            ))}
          </tbody>
        </table>
        {!loadError && !visible.length && (
          <p role="status">
            {!ready
              ? '正在读取任务记录…'
              : rows.length
                ? '当前筛选下没有任务。'
                : '尚无任务记录。执行烧录或设备任务后，可在这里查看结果。'}
          </p>
        )}
        {selected && (
          <section>
            <h3>阶段记录</h3>
            <p>
              <code>{selected.taskId}</code>
            </p>
            {selected.error && <p>{selected.error}</p>}
            {bootReport?.taskId === selected.taskId && (
              <div>
                <h3>
                  {bootReport.coldBoot ? '人工断电上电验收' : '人工启动验收'} ·{' '}
                  {bootReport.result === 'pass' ? '通过' : '失败'}
                </h3>
                <p>
                  {bootReport.operator} · {date(bootReport.recordedAt)}
                </p>
                <code>{bootReport.evidence}</code>
              </div>
            )}
            {steps.length ? (
              <ol>
                {steps.map((s) => (
                  <li key={s.id}>
                    {date(s.startedAt)} · {s.stepName} · {statusText(s.status)} {s.error}
                  </li>
                ))}
              </ol>
            ) : (
              <p>暂无已记录的阶段明细</p>
            )}
          </section>
        )}
        {detailError && <p role="alert">{detailError}</p>}
      </div>
      <footer>
        <button onClick={exportRecord} disabled={!selected}>
          导出选中记录
        </button>
        <button onClick={onClose}>关闭</button>
      </footer>
    </dialog>
  );
}
