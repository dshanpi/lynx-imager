import { useEffect, useRef, useState } from 'react';
import { invokeCommand } from '../../../Platform/IPC/Client';
import type {
  LearnCoursePin,
  LearnLearningRecord,
  LearnRecordEntry,
  LearnRecordRequest,
} from '../../../Platform/IPC/Commands';
import { learnServiceError } from './serviceStatus';
import { PendingWriteNotice, useWriteRecovery } from './writeRecovery';

/** A learner statement is not an assessment or hardware result. */
export function LearnGenericProgress({
  accountId,
  active,
  pin,
  unitId,
  chapterId,
  drafts,
  locallySaved = false,
  storageError = '',
  onRetryStorage,
  onDraftChange,
}: {
  accountId: number;
  active: boolean;
  pin: LearnCoursePin;
  unitId: string;
  chapterId: string;
  drafts: Record<string, string>;
  locallySaved?: boolean;
  storageError?: string;
  onRetryStorage?: () => void;
  onDraftChange: (activityId: string, text: string | null) => void;
}) {
  const [record, setRecord] = useState<LearnLearningRecord | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [reload, setReload] = useState(0);
  const sequence = useRef(0);
  const {
    load: loadRecovery,
    reset: resetRecovery,
    ...recovery
  } = useWriteRecovery<LearnRecordRequest>({ accountId, pin, unitId, kind: 'record' });
  const { enrollmentId, releaseId, manifestSha256 } = pin;
  useEffect(() => {
    const seq = ++sequence.current;
    setRecord(null);
    resetRecovery();
    if (!active) return;
    setBusy(true);
    setError('');
    void invokeCommand('learn_get_learning_record', { enrollmentId, releaseId, manifestSha256 })
      .then((value) => {
        if (seq === sequence.current) {
          loadRecovery();
          setRecord(value);
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
  }, [
    active,
    accountId,
    enrollmentId,
    releaseId,
    manifestSha256,
    unitId,
    reload,
    loadRecovery,
    resetRecovery,
  ]);
  async function save(entry: LearnRecordEntry, kind: 'confirm' | 'reflection') {
    if (!active || !record || busy || !recovery.ready || recovery.pending) return;
    const body = {
      ...pin,
      expectedRevision: record.revision,
      unitId: entry.unitId,
      activityId: entry.activityId,
      kind,
      text: kind === 'reflection' ? (drafts[entry.activityId] ?? entry.reflection) : '',
    };
    await send({ ...body, requestId: crypto.randomUUID() });
  }
  async function send(request: LearnRecordRequest) {
    if (!active || !record || busy || !recovery.ready) return;
    const seq = ++sequence.current;
    setBusy(true);
    setError('');
    setNotice('');
    try {
      recovery.begin(request);
      const value = await invokeCommand('learn_save_learning_record', { request });
      if (seq !== sequence.current) return;
      recovery.complete(request);
      setRecord(value);
      // A newer locally edited draft is not the input acknowledged by this response.
      if (request.kind === 'reflection' && drafts[request.activityId] === request.text)
        onDraftChange(request.activityId, null);
      setNotice('已同步到当前课程学习记录。');
    } catch (e) {
      if (seq === sequence.current) {
        if (
          typeof e === 'object' &&
          e !== null &&
          'machineCode' in e &&
          e.machineCode === 'LEARN_RECORD_CONFLICT'
        ) {
          try {
            recovery.complete(request);
          } catch (storageFailure) {
            setError(learnServiceError(storageFailure));
            resetRecovery();
            return;
          }
          resetRecovery();
        }
        setError(`${learnServiceError(e)} 未提交的笔记仍保留；冲突时请先同步，再保存。`);
      }
    } finally {
      if (seq === sequence.current) setBusy(false);
    }
  }
  const rows = record?.entries.filter((e) => e.unitId === unitId) || [];
  const chapterRows = record?.entries.filter((e) => e.chapterId === chapterId) || [];
  return (
    <section className="learn-generic-progress" aria-label="我的学习与复盘">
      <h3>我的学习与复盘</h3>
      <p>
        本章已自述完成 {chapterRows.filter((e) => e.confirmed).length} / {chapterRows.length}{' '}
        个活动。
      </p>
      <small>阅读确认和笔记由本人填写，实操与测验需另行验证。</small>
      {Object.keys(drafts).length > 0 && (
        <p role="status">
          {locallySaved
            ? '本节复盘草稿已保存在本机；返回课程或重启后可恢复，尚未同步到官网。'
            : '本节有未保存的复盘。当前输入仍保留，离开课程前请保存。'}
        </p>
      )}
      {storageError && (
        <div role="alert" className="learn-notice">
          <p>{storageError}</p>
          {onRetryStorage && (
            <button className="learning-secondary" onClick={onRetryStorage}>
              重试本机草稿保存
            </button>
          )}
        </div>
      )}
      <button
        className="learning-secondary"
        disabled={!active || busy}
        onClick={() => setReload((n) => n + 1)}
      >
        {busy ? '正在同步…' : '同步学习记录'}
      </button>
      {error && (
        <p role="alert" className="learn-notice">
          {error}
        </p>
      )}
      <PendingWriteNotice
        request={recovery.pending}
        disabled={!active || busy || !record || !recovery.ready}
        retry={() => {
          if (recovery.pending) void send(recovery.pending);
        }}
      />
      {notice && <p role="status">{notice}</p>}
      {rows.map((entry) => (
        <div key={entry.activityId} className="learn-record-entry">
          <strong>{entry.title}</strong>
          <p>
            {entry.confirmed
              ? '已确认阅读 / 理解（本人自述）'
              : entry.selfConfirmable
                ? '尚未确认阅读 / 理解'
                : '待实操或测验验证'}
          </p>
          {entry.selfConfirmable && (
            <button
              className="learning-secondary"
              disabled={!active || busy || !recovery.ready || !!recovery.pending || entry.confirmed}
              onClick={() => void save(entry, 'confirm')}
            >
              我已阅读并理解
            </button>
          )}
          <label htmlFor={`learn-reflection-${entry.activityId}`}>
            本步理解、遇到的问题与下一步
          </label>
          <textarea
            id={`learn-reflection-${entry.activityId}`}
            rows={3}
            maxLength={4096}
            disabled={!active || busy}
            value={drafts[entry.activityId] ?? entry.reflection}
            onChange={(e) => onDraftChange(entry.activityId, e.target.value)}
          />
          <button
            className="learning-secondary"
            disabled={
              !active ||
              busy ||
              !recovery.ready ||
              !!recovery.pending ||
              !(drafts[entry.activityId] ?? entry.reflection).trim()
            }
            onClick={() => void save(entry, 'reflection')}
          >
            保存复盘
          </button>
          {entry.updatedAt && <small>上次同步 {new Date(entry.updatedAt).toLocaleString()}</small>}
        </div>
      ))}
    </section>
  );
}
