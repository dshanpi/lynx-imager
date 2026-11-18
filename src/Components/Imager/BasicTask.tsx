import { useEffect, useState } from 'react';
import { invokeCommand, subscribeEvent, type WorkbenchTaskHistory } from '../../Platform/IPC';
import { formatErrorMessage } from '../../Utils';
import { formatLinuxBytes } from '../Workbench/linuxWorkbenchModel';

type Details = NonNullable<WorkbenchTaskHistory['details']>;
type View = {
  id: string;
  status: string;
  phase: string;
  details: Details;
  logs: string[];
  error: string;
  startedAt: number | null;
  finishedAt: number | null;
  observationError: string;
};
const initial = (id: string): View => ({
  id,
  status: 'queued',
  phase: '',
  details: {},
  logs: [],
  error: '',
  startedAt: null,
  finishedAt: null,
  observationError: '',
});
export const ended = (status: string) =>
  ['success', 'failed', 'cancelled', 'terminated'].includes(status);
const labels: Record<string, string> = {
  queued: '已提交',
  running: '执行中',
  pending: '等待',
  not_verified: '未验证',
  not_executed: '未执行',
  not_run: '未执行',
  verified: '校验通过',
  preflight: '预检',
  success: '成功',
  failed: '失败',
  cancelled: '已取消',
  terminated: '已终止',
  completed: '完成',
  skipped: '已跳过',
  writing: '写入中',
  verifying: '校验中',
  preparing: '准备中',
  erasing: '擦除中',
  verify: '校验',
  write: '写入',
  erase: '擦除',
  flash: '烧录',
  reboot: '重启',
  unchanged: '保持原样',
};
export const stateName = (value?: string | null) => (value ? (labels[value] ?? value) : '未报告');

export function useBasicTask(id: string) {
  const [view, setView] = useState<View>(() => initial(''));
  useEffect(() => {
    if (!id) return;
    let alive = true;
    let sequence = 0;
    let finalSeen = false;
    let timer: ReturnType<typeof setTimeout>;
    const listeners: Array<() => void> = [];
    setView(initial(id));
    const update = (change: (old: View) => View) => {
      if (alive) setView((old) => change(old.id === id ? old : initial(id)));
    };
    function keep(unlisten: () => void) {
      if (alive) listeners.push(unlisten);
      else unlisten();
    }
    const failedToListen = (e: unknown) =>
      update((old) => ({ ...old, observationError: formatErrorMessage(e) }));
    void subscribeEvent('workbench-task-event', (event) => {
      if (event.taskId !== id || !alive) return;
      sequence++;
      if (ended(event.status)) finalSeen = true;
      update((old) => {
        const progress =
          event.total && event.current != null
            ? Math.min(100, Math.max(0, (event.current / event.total) * 100))
            : null;
        return {
          ...old,
          status: ended(old.status) ? old.status : event.status || old.status,
          phase: event.phase || old.phase,
          error: event.status === 'failed' ? event.message : old.error,
          logs: event.message ? [...old.logs.slice(-79), event.message] : old.logs,
          details: progress == null ? old.details : { ...old.details, progressPct: progress },
        };
      });
    })
      .then(keep)
      .catch(failedToListen);
    void subscribeEvent('workbench-flash-telemetry', (event) => {
      if (event.taskId === id && alive) {
        sequence++;
        update((old) => ({ ...old, details: { ...old.details, ...event.telemetry } }));
      }
    })
      .then(keep)
      .catch(failedToListen);
    async function refresh() {
      const atRead = sequence;
      let persistedEnd = false;
      try {
        const list = await invokeCommand(
          'workbench_list_tasks',
          { limit: 100 },
          { timeoutMs: 8000 }
        );
        const task = list.find((item) => item.taskId === id);
        if (task) {
          persistedEnd = ended(task.status);
          update((old) => {
            const useRecord = persistedEnd || (!finalSeen && sequence === atRead);
            const recordDetails = {
              ...task.details,
              progressPct: task.details?.progressPct ?? task.progressPct,
            };
            return {
              ...old,
              status: useRecord ? task.status : old.status,
              phase: useRecord ? task.phase : old.phase,
              error: task.error || old.error,
              startedAt: task.startedAt,
              finishedAt: task.finishedAt,
              observationError: '',
              details: useRecord
                ? { ...old.details, ...recordDetails }
                : { ...recordDetails, ...old.details },
              logs:
                (persistedEnd || !old.logs.length) && task.details?.outputTail?.length
                  ? task.details.outputTail.slice(-80)
                  : old.logs,
            };
          });
        }
      } catch (e) {
        failedToListen(e);
      }
      if (alive && !persistedEnd)
        timer = setTimeout(() => {
          void refresh();
        }, 2500);
    }
    void refresh();
    return () => {
      alive = false;
      clearTimeout(timer);
      listeners.forEach((stop) => stop());
    };
  }, [id]);
  return view.id === id ? view : initial(id);
}

export function BasicTask({
  task,
  target,
  usb,
  path,
}: {
  task: View;
  target: string;
  usb: string;
  path: string;
}) {
  const d = task.details;
  const pct = d.progressPct;
  const signals = [
    ['写入', d.writeState],
    ['协议校验', d.verifyState],
    ['物理回读校验', d.physicalVerifyState],
    ['结束动作', d.postFlashState],
    ['系统启动', d.bootVerifyState],
  ];
  return (
    <>
      <section className="basic-section">
        <h3>
          {stateName(task.status)}
          {task.phase && ' · ' + stateName(task.phase)}
        </h3>
        {pct != null && <div className="basic-percent">{Math.round(pct)}%</div>}
        <progress
          aria-label="后端报告的烧录进度"
          max={100}
          value={pct ?? undefined}
          hidden={pct == null && ended(task.status)}
        />
        <p className="basic-note">
          任务 <code>{task.id}</code>
          {d.transferredBytes != null && '　已传输 ' + formatLinuxBytes(d.transferredBytes)}
          {d.totalBytes != null && ' / ' + formatLinuxBytes(d.totalBytes)}
          {d.bytesPerSecond != null && '　' + formatLinuxBytes(d.bytesPerSecond) + '/s'}
          {d.currentPartition && '　分区 ' + d.currentPartition}
        </p>
        <div className="basic-task-identity">
          <span>
            目标 <strong>{target}</strong>
          </span>
          <span>
            接口 <code>{usb}</code>
          </span>
          <span>
            镜像 <code>{path}</code>
          </span>
          {task.startedAt && (
            <span>开始 {new Date(task.startedAt * 1000).toLocaleTimeString()}</span>
          )}
          {task.finishedAt && (
            <span>结束 {new Date(task.finishedAt * 1000).toLocaleTimeString()}</span>
          )}
        </div>
        {task.error && (
          <p role="alert" className="basic-warning">
            {task.error}
          </p>
        )}
        {task.observationError && (
          <p role="status">读取任务状态遇到问题，正在重新读取。{task.observationError}</p>
        )}
      </section>
      <div className="basic-columns">
        <section>
          <h3>执行结果</h3>
          <dl className="basic-props">
            {signals.map(([name, val]) => (
              <div key={name}>
                <dt>{name}</dt>
                <dd>{stateName(val)}</dd>
              </div>
            ))}
          </dl>
        </section>
        <section>
          <h3>任务日志</h3>
          <pre className="basic-log" tabIndex={0} aria-label="当前任务日志">
            {task.logs.join('\n') || '等待后端日志…'}
          </pre>
        </section>
      </div>
    </>
  );
}
