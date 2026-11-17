import { useEffect, useRef, useState } from 'react';
import { invokeCommand } from '../../../Platform/IPC/Client';
import type {
  LearnActionEvidencePreview,
  LearnActionRequest,
  LearnActionRun,
  LearnCourseManifest,
  LearnCoursePin,
  LearnCourseUnit,
  LearnPracticeSyncBatch,
  CommandResult,
} from '../../../Platform/IPC/Commands';
import { LearnActionEvidence } from './LearnActionEvidence';
import { learnServiceError } from './serviceStatus';
import { readPendingWrite } from './writeRecovery';
import './LearnPracticeSync.css';

const terminal = new Set(['succeeded', 'failed', 'cancelled', 'timed_out', 'output_limit']);
const results: Record<string, string> = {
  succeeded: '任务完成',
  failed: '执行失败',
  cancelled: '已取消',
  timed_out: '已超时',
  output_limit: '输出超限',
};
const stages: Record<LearnPracticeSyncBatch['stage'], string> = {
  session: '创建实践尝试',
  events: '同步执行事件',
  evidence: '登记证据',
  grant: '获取上传授权',
  upload: '上传已选文件',
  completion: '确认文件核验',
  done: '文件已核验',
};
export function LearnPracticeSync({
  active,
  accountId,
  pin,
  unit,
  profileId,
  actions,
}: {
  active: boolean;
  accountId: number;
  pin: LearnCoursePin;
  unit: LearnCourseUnit;
  profileId: string;
  actions: LearnCourseManifest['actions'];
}) {
  const [runs, setRuns] = useState<LearnActionRun[]>([]);
  const [batches, setBatches] = useState<LearnPracticeSyncBatch[]>([]);
  const [previews, setPreviews] = useState<Record<string, LearnActionEvidencePreview>>({});
  const [freshness, setFreshness] = useState<
    Record<string, CommandResult<'learn_inspect_practice_freshness'>>
  >({});
  const [selected, setSelected] = useState<string[]>([]);
  const [consent, setConsent] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [reload, setReload] = useState(0);
  const sequence = useRef(0);
  const scope = { ...pin, profileId, unitId: unit.unitId };
  const requests = unit.activities
    .filter((a) => a.environmentRef === profileId)
    .flatMap((a) =>
      a.actionRefs
        .filter((id) =>
          actions.some(
            (action) => action.actionId === id && action.capabilityId === 'learn.project.task'
          )
        )
        .map((actionId) => ({ ...scope, activityId: a.activityId, actionId }))
    );
  const key = JSON.stringify({ accountId, scope, requests });
  const sameScope = (batch: LearnPracticeSyncBatch) =>
    Object.entries(scope).every(
      ([name, value]) => batch[name as keyof LearnPracticeSyncBatch] === value
    );
  function checked(batch: LearnPracticeSyncBatch) {
    if (
      !sameScope(batch) ||
      !batch.batchId ||
      !Array.isArray(batch.runIds) ||
      !['pending', 'conflict', 'accepted'].includes(batch.status) ||
      !(batch.stage in stages) ||
      !Number.isSafeInteger(batch.revision) ||
      batch.revision < 0
    )
      throw { machineCode: 'LEARN_SYNC_RECEIPT_INVALID' };
    return batch;
  }
  async function refresh(generation: number) {
    const values = await invokeCommand('learn_list_practice_sync', { request: scope });
    if (generation !== sequence.current) return;
    setBatches(values.map(checked));
  }
  useEffect(() => {
    const generation = ++sequence.current;
    setFreshness({});
    setRuns([]);
    setBatches([]);
    setPreviews({});
    setSelected([]);
    setConsent(false);
    setError('');
    setBusy(false);
    if (!active || accountId <= 0) return;
    const saved = JSON.parse(key) as { scope: typeof scope; requests: LearnActionRequest[] };
    setBusy(true);
    void Promise.all([
      invokeCommand('learn_list_practice_sync', { request: saved.scope }),
      Promise.all(
        saved.requests.map(async (request) => {
          const rows = await invokeCommand('learn_list_actions', { request });
          return rows.filter(
            (run) =>
              terminal.has(run.state) &&
              Object.entries(request).every(
                ([name, value]) => run.request[name as keyof LearnActionRequest] === value
              )
          );
        })
      ),
    ])
      .then(([history, rows]) => {
        if (generation !== sequence.current) return;
        setBatches(history.map(checked));
        setRuns([...new Map(rows.flat().map((run) => [run.runId, run])).values()]);
      })
      .catch((e) => {
        if (generation === sequence.current) setError(learnServiceError(e));
      })
      .finally(() => {
        if (generation === sequence.current) setBusy(false);
      });
    return () => {
      sequence.current = generation + 1;
    };
    // The serialized scope includes all account, pin and action inputs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, key, reload]);
  const claimed = new Set(batches.flatMap((b) => b.runIds));
  const chosen = selected.filter((id) => previews[id]?.executionContext && !claimed.has(id));
  const bytes = chosen.reduce(
    (sum, id) =>
      sum +
      previews[id].items
        .filter((item) =>
          actions
            .find((a) => a.actionId === previews[id].request.actionId)
            ?.evidenceOutputs.includes(item.kind)
        )
        .reduce((n, item) => n + item.size, 0),
    0
  );
  function select(run: LearnActionRun, enabled: boolean) {
    setConsent(false);
    setSelected((previous) =>
      enabled
        ? [
            ...previous.filter(
              (id) => id !== run.runId && previews[id]?.request.actionId !== run.request.actionId
            ),
            run.runId,
          ]
        : previous.filter((id) => id !== run.runId)
    );
  }
  async function advance(batch: LearnPracticeSyncBatch, generation: number) {
    let current = batch;
    for (let i = 0; i < 512 && current.status !== 'accepted'; i++) {
      if (generation !== sequence.current) return;
      const next = checked(
        await invokeCommand('learn_advance_practice_sync', { batchId: current.batchId })
      );
      if (generation !== sequence.current) return;
      if (next.batchId !== current.batchId || next.revision < current.revision)
        throw { machineCode: 'LEARN_SYNC_RECEIPT_INVALID' };
      setBatches((previous) => [next, ...previous.filter((b) => b.batchId !== next.batchId)]);
      if (next.errorCode) {
        setError(learnServiceError({ machineCode: next.errorCode }));
        return;
      }
      if (next.revision === current.revision && next.status !== 'accepted')
        throw { machineCode: 'LEARN_SYNC_RECEIPT_INVALID' };
      current = next;
    }
  }
  async function synchronize(existing?: LearnPracticeSyncBatch) {
    if (!active || busy || (!existing && (!consent || chosen.length === 0 || chosen.length > 8)))
      return;
    const generation = sequence.current;
    setBusy(true);
    setError('');
    try {
      let batch = existing;
      if (!batch) {
        if (readPendingWrite(localStorage, { accountId, pin, unitId: unit.unitId, kind: 'review' }))
          throw { machineCode: 'LEARN_REVIEW_PENDING' };
        const result = await invokeCommand('learn_prepare_practice_sync', {
          request: {
            ...scope,
            batchId: `sync_${crypto.randomUUID()}`,
            runs: chosen.map((id) => ({ runId: id, snapshotSha256: previews[id].snapshotSha256 })),
          },
        });
        if (generation !== sequence.current) return;
        batch = checked(result);
        setBatches((previous) => [batch!, ...previous.filter((b) => b.batchId !== batch!.batchId)]);
        setSelected([]);
        setConsent(false);
      }
      await advance(batch, generation);
    } catch (e) {
      if (generation !== sequence.current) return;
      setError(learnServiceError(e));
      // A lost prepare reply may already have committed the batch. Recover by GET/local read only.
      await refresh(generation).catch(() => {});
    } finally {
      if (generation === sequence.current) setBusy(false);
    }
  }
  async function inspect(batchId: string) {
    if (!active || busy) return;
    const generation = sequence.current;
    setBusy(true);
    setError('');
    setFreshness((previous) => {
      const next = { ...previous };
      delete next[batchId];
      return next;
    });
    try {
      const value = await invokeCommand('learn_inspect_practice_freshness', { batchId });
      if (generation !== sequence.current) return;
      if (
        value.batchId !== batchId ||
        !['current', 'stale'].includes(value.status) ||
        !Number.isFinite(Date.parse(value.checkedAt))
      )
        throw { machineCode: 'LEARN_EVIDENCE_CHECK_UNAVAILABLE' };
      setFreshness((previous) => ({ ...previous, [batchId]: value }));
    } catch (e) {
      if (generation === sequence.current) setError(learnServiceError(e));
    } finally {
      if (generation === sequence.current) setBusy(false);
    }
  }
  if (requests.length === 0) return null;
  return (
    <section className="learn-practice-sync" aria-label="本课时实践证据同步">
      <h3>本课时实践证据同步</h3>
      <p>
        选择本课时中属于同一工程和执行环境的记录。每个动作选择一次执行，所选动作进入同一个云端尝试。
      </p>
      <button
        className="learning-secondary"
        disabled={!active || busy}
        onClick={() => setReload((v) => v + 1)}
      >
        刷新执行与同步记录
      </button>
      {error && (
        <p role="alert" className="learn-notice">
          {error}
        </p>
      )}
      {runs.length === 0 && !busy && <p>暂无已结束的工程任务。先完成任务，再查看和选择其证据。</p>}
      {runs.map((run) => (
        <details key={run.runId} data-sync-run={run.runId} open={selected.includes(run.runId)}>
          <summary>
            {unit.activities.find((a) => a.activityId === run.request.activityId)?.title ||
              run.request.actionId}{' '}
            · {results[run.state]} · {run.runId}
          </summary>
          <LearnActionEvidence
            active={active && !busy}
            accountId={accountId}
            run={run}
            onPreview={(value) => setPreviews((previous) => ({ ...previous, [run.runId]: value }))}
          />
          <label>
            <input
              type="checkbox"
              checked={selected.includes(run.runId)}
              disabled={
                !active || busy || claimed.has(run.runId) || !previews[run.runId]?.executionContext
              }
              onChange={(event) => select(run, event.target.checked)}
            />
            将此执行加入本批次
          </label>
          {claimed.has(run.runId) && <p>此执行已归属下面的同步批次，请继续原批次。</p>}
        </details>
      ))}
      <p>
        已选 {chosen.length} 次执行 · 计划上传 {bytes}{' '}
        字节。上传内容包括动作声明的标准输出、诊断日志或退出码，以及执行时的环境观察。
      </p>
      {chosen.length > 8 && <p className="learn-notice">一次最多选择 8 个动作。</p>}
      <label className="learn-practice-consent">
        <input
          type="checkbox"
          checked={consent}
          disabled={!active || busy || chosen.length === 0}
          onChange={(event) => setConsent(event.target.checked)}
        />
        我已检查所选输出，同意将这些证据上传到官网课程记录。
      </label>
      <button
        className="learning-secondary"
        disabled={!active || busy || !consent || chosen.length === 0 || chosen.length > 8}
        onClick={() => void synchronize()}
      >
        同步已选证据
      </button>
      {batches.map((batch) => (
        <article key={batch.batchId} className="learn-practice-batch">
          <p>
            <strong>
              {batch.status === 'accepted'
                ? '云端已接收'
                : batch.status === 'conflict'
                  ? '需要核对原批次'
                  : '待确认同步'}
            </strong>{' '}
            · {stages[batch.stage]}
          </p>
          <p>
            文件核验 {batch.verifiedCount}/{batch.evidenceCount} · 执行 {batch.runIds.length} 次
          </p>
          <small>
            批次：{batch.batchId}
            {batch.attemptId && <> · 尝试：{batch.attemptId}</>}
          </small>
          {batch.errorCode && (
            <p className="learn-notice">{learnServiceError({ machineCode: batch.errorCode })}</p>
          )}
          {batch.status === 'accepted' ? (
            <>
              <p>
                文件核验不代表课程通过。来源仍为用户提供；请在下方点击“同步评测结果”，核对当前尝试后提交章节评测。
              </p>
              <p role="status">
                {!freshness[batch.batchId]
                  ? '历史文件已核验；尚未核验当前工程。'
                  : freshness[batch.batchId].status === 'stale'
                    ? '当前输入已变化，请在上方重新执行相关动作，再预览并同步新证据。历史记录仍保留。'
                    : '当前工程在本次检查时与证据输入一致；修改后需重新核验。'}
              </p>
              {freshness[batch.batchId] && (
                <small>检查时间：{freshness[batch.batchId].checkedAt}</small>
              )}
              <button
                className="learning-secondary"
                disabled={!active || busy}
                onClick={() => void inspect(batch.batchId)}
              >
                核验当前工程
              </button>
            </>
          ) : (
            <button
              className="learning-secondary"
              disabled={!active || busy}
              onClick={() => void synchronize(batch)}
            >
              继续确认原批次
            </button>
          )}
        </article>
      ))}
      {busy && <p role="status">正在读取记录或确认同步阶段…</p>}
    </section>
  );
}
