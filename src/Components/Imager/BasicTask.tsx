import { useEffect, useState } from 'react';
import { invokeCommand, subscribeEvent, type WorkbenchTaskHistory } from '../../Platform/IPC';
import { formatErrorMessage } from '../../Utils';

type Details = NonNullable<WorkbenchTaskHistory['details']>;
type View = {
  id: string;
  status: string;
  phase: string;
  details: Details;
  logs: string[];
  logVersion: number;
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
  logVersion: 0,
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
          logVersion: old.logVersion + (event.message ? 1 : 0),
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
            const tail = task.details?.outputTail;
            const logs =
              (persistedEnd || !old.logs.length) && tail?.length ? tail.slice(-80) : old.logs;
            let overlap = Math.min(old.logs.length, logs.length);
            while (
              logs !== old.logs &&
              overlap > 0 &&
              !logs
                .slice(0, overlap)
                .every((line, index) => line === old.logs[old.logs.length - overlap + index])
            ) {
              overlap--;
            }
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
              logs,
              logVersion: old.logVersion + logs.length - overlap,
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
