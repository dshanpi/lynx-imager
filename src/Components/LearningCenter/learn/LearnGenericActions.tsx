import { useEffect, useRef, useState } from 'react';
import { invokeCommand } from '../../../Platform/IPC/Client';
import type { LearnActionRequest, LearnActionRun } from '../../../Platform/IPC/Commands';
import { LearnActionEvidence } from './LearnActionEvidence';
import { learnServiceError } from './serviceStatus';

const labels: Record<string, string> = {
  prepared: '待确认',
  starting: '正在启动',
  running: '执行中',
  cancelling: '正在取消',
  succeeded: '任务完成',
  failed: '执行失败',
  cancelled: '已取消',
  timed_out: '已超时',
  output_limit: '日志超出上限，已终止',
  busy: '工程被占用',
  unknown: '需查询确认',
};
const pending = (run: LearnActionRun) =>
  ['starting', 'running', 'cancelling', 'unknown'].includes(run.state);

export function LearnGenericActions({
  active,
  request,
  accountId = 0,
}: {
  accountId?: number;
  active: boolean;
  request: LearnActionRequest;
}) {
  const [runs, setRuns] = useState<LearnActionRun[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [confirmed, setConfirmed] = useState('');
  const [loaded, setLoaded] = useState(false);
  const [reload, setReload] = useState(0);
  const sequence = useRef(0);
  const key = JSON.stringify(request);
  useEffect(() => {
    const seq = ++sequence.current;
    setRuns([]);
    setConfirmed('');
    setError('');
    setBusy(false);
    setLoaded(false);
    if (!active) return;
    setBusy(true);
    void invokeCommand('learn_list_actions', { request: JSON.parse(key) as LearnActionRequest })
      .then((result) => {
        if (seq === sequence.current) {
          setRuns(result);
          setLoaded(true);
        }
      })
      .catch((e) => {
        if (seq === sequence.current) setError(learnServiceError(e));
      })
      .finally(() => {
        if (seq === sequence.current) setBusy(false);
      });
    return () => {
      sequence.current++;
    };
  }, [active, key, reload, accountId]);
  useEffect(() => {
    const run = runs.find((r) => ['starting', 'running', 'cancelling'].includes(r.state));
    if (!active || busy || error || !run) return;
    const seq = sequence.current;
    let disposed = false;
    const timer = window.setTimeout(() => {
      void invokeCommand('learn_query_action', { runId: run.runId })
        .then((result) => {
          if (!disposed && seq === sequence.current)
            setRuns((previous) => previous.map((r) => (r.runId === result.runId ? result : r)));
        })
        .catch((e) => {
          if (!disposed && seq === sequence.current) setError(learnServiceError(e));
        });
    }, 2000);
    return () => {
      disposed = true;
      window.clearTimeout(timer);
    };
  }, [active, busy, error, runs]);
  async function perform(
    operation: 'prepare' | 'start' | 'query' | 'cancel',
    run?: LearnActionRun
  ) {
    if (!active || busy) return;
    if (operation === 'prepare' && !loaded) return;
    const seq = ++sequence.current;
    setBusy(true);
    setError('');
    setConfirmed('');
    try {
      const result =
        operation === 'prepare'
          ? await invokeCommand('learn_prepare_action', { request })
          : operation === 'start'
            ? await invokeCommand('learn_start_action', {
                runId: run!.runId,
                confirmation: run!.result.confirmation!,
              })
            : await invokeCommand(
                operation === 'cancel' ? 'learn_cancel_action' : 'learn_query_action',
                {
                  runId: run!.runId,
                }
              );
      if (seq === sequence.current)
        setRuns((previous) => [result, ...previous.filter((r) => r.runId !== result.runId)]);
    } catch (e) {
      if (seq === sequence.current) {
        setError(learnServiceError(e));
        if (operation === 'start' && run)
          setRuns((previous) =>
            previous.map((r) => (r.runId === run.runId ? { ...r, state: 'unknown' } : r))
          );
      }
    } finally {
      if (seq === sequence.current) setBusy(false);
    }
  }
  return (
    <section className="learn-panel learn-generic-actions" aria-label="课程工程任务">
      <h3>课程工程任务</h3>
      <p>先核验固定版本的任务、工程和操作范围，再确认执行。运行结果不等于课程评测通过。</p>
      {error && (
        <p role="alert" className="learn-notice">
          {error}
        </p>
      )}
      {!loaded && !busy && active && (
        <button className="learning-secondary" onClick={() => setReload((value) => value + 1)}>
          重新加载任务记录
        </button>
      )}
      <button
        className="learning-primary"
        disabled={!active || busy || !loaded || runs.some(pending)}
        onClick={() => void perform('prepare')}
      >
        准备并核验任务
      </button>
      {runs.some(pending) && (
        <p role="status">已有任务尚未确认结束。请查询原任务，避免重复操作。</p>
      )}
      {runs.map((run) => (
        <section key={run.runId} className="learn-action-attempt">
          <h4>
            {labels[run.state] || '需确认'} · {run.projectName}
          </h4>
          <small>
            {run.hostName} · {run.updatedAt}
          </small>
          {run.result.task && (
            <dl>
              <dt>工作目录</dt>
              <dd>
                <code>{run.result.task.cwd}</code>
              </dd>
              <dt>固定源码提交</dt>
              <dd>
                <code>{run.result.task.commit}</code>
              </dd>
              <dt>执行参数</dt>
              <dd>
                <code>{JSON.stringify(run.result.task.argv)}</code>
              </dd>
              <dt>超时</dt>
              <dd>{run.result.task.timeoutSeconds} 秒</dd>
            </dl>
          )}
          {run.state === 'prepared' && (
            <>
              <label>
                <input
                  type="checkbox"
                  checked={confirmed === run.runId}
                  disabled={!active || busy}
                  onChange={(e) => setConfirmed(e.target.checked ? run.runId : '')}
                />
                我已核对工程、任务和参数，允许执行本次工程操作
              </label>
              <button
                className="learning-primary"
                disabled={
                  !active ||
                  busy ||
                  confirmed !== run.runId ||
                  !run.result.confirmation ||
                  runs.some(pending)
                }
                onClick={() => void perform('start', run)}
              >
                确认执行
              </button>
            </>
          )}
          <div className="learn-hub-actions">
            <button
              className="learning-secondary"
              disabled={!active || busy}
              onClick={() => void perform('query', run)}
            >
              查询实际状态
            </button>
            {['prepared', 'starting', 'running', 'cancelling', 'unknown'].includes(run.state) && (
              <button
                className="learning-secondary"
                disabled={!active || busy}
                onClick={() => void perform('cancel', run)}
              >
                取消此任务
              </button>
            )}
          </div>
          {run.result.reason && <p role="status">{run.result.reason}</p>}
          {run.result.log && (
            <details open={pending(run)}>
              <summary>Ubuntu 任务日志（已脱敏，最多显示末尾 32 KiB）</summary>
              <pre>{run.result.log}</pre>
            </details>
          )}
          {run.result.exitCode != null && <p>退出码：{run.result.exitCode}</p>}
          {run.result.artifacts?.map((a) => (
            <p key={a.path}>
              {a.path} · {a.size} 字节
              <br />
              <code>{a.sha256}</code>
            </p>
          ))}
          <LearnActionEvidence active={active} accountId={accountId} run={run} />
          <small>
            任务 {run.runId} ·{' '}
            {run.syncStatus === 'local_only' ? '结果已保存在本机，尚未同步评测' : run.syncStatus}
          </small>
        </section>
      ))}
    </section>
  );
}
