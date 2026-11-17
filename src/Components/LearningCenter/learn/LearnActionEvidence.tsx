import './LearnActionEvidence.css';
import { useEffect, useRef, useState } from 'react';
import { invokeCommand } from '../../../Platform/IPC/Client';
import type { LearnActionEvidencePreview, LearnActionRun } from '../../../Platform/IPC/Commands';
import { learnServiceError } from './serviceStatus';

const titles = { stdout: '标准输出', build_log: '合并诊断日志', exit_status: '退出码' };
export function LearnActionEvidence({
  active,
  accountId,
  run,
  onPreview,
}: {
  active: boolean;
  accountId: number;
  run: LearnActionRun;
  onPreview?: (value: LearnActionEvidencePreview) => void;
}) {
  const [preview, setPreview] = useState<LearnActionEvidencePreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const sequence = useRef(0);
  const key = JSON.stringify([accountId, run.runId, run.request]);
  const terminal = ['succeeded', 'failed', 'cancelled', 'timed_out', 'output_limit'].includes(
    run.state
  );
  useEffect(() => {
    sequence.current++;
    setPreview(null);
    setBusy(false);
    setError('');
    return () => {
      sequence.current++;
    };
  }, [active, key]);
  async function collect() {
    if (!active || busy || !terminal) return;
    const seq = ++sequence.current;
    setBusy(true);
    setError('');
    try {
      const value = await invokeCommand('learn_collect_action_evidence', { runId: run.runId });
      if (seq !== sequence.current) return;
      if (
        value.runId !== run.runId ||
        value.syncStatus !== 'local_only' ||
        value.trust !== 'client_observation' ||
        Object.entries(run.request).some(
          ([name, expected]) => value.request[name as keyof typeof run.request] !== expected
        )
      ) {
        throw { machineCode: 'LEARN_EVIDENCE_INVALID' };
      }
      setPreview(value);
      onPreview?.(value);
    } catch (e) {
      if (seq === sequence.current) setError(learnServiceError(e));
    } finally {
      if (seq === sequence.current) setBusy(false);
    }
  }
  if (!terminal) return null;
  return (
    <section className="learn-action-evidence" aria-label="本地实践证据">
      <button
        className="learning-secondary"
        disabled={!active || busy}
        onClick={() => void collect()}
      >
        {busy ? '正在校验完整证据…' : preview ? '重新打开已保存证据' : '查看并保存本地证据'}
      </button>
      <p>首次收集读取此任务的完整脱敏输出，全部校验成功后保存到本机。再次打开使用已保存快照。</p>
      {error && (
        <p role="alert" className="learn-notice">
          {error}
        </p>
      )}
      {preview && (
        <div>
          <p role="status">完整证据已保存在本机；云端状态请查看本课时的同步记录。</p>
          <p>自动脱敏不能识别所有敏感信息，请检查内容。本地记录不代表课程评测通过。</p>
          <small>收集时间：{preview.collectedAt}</small>
          {preview.executionContext ? (
            <details>
              <summary>执行时记录的环境与源码</summary>
              <p>
                {preview.executionContext.facts.os || '系统名称未知'} ·{' '}
                {preview.executionContext.facts.architecture}
              </p>
              <p>观察时间：{preview.executionContext.observedAt}</p>
              <p>
                固定源码提交：<code>{preview.executionContext.sourceCommit}</code>
              </p>
              <p>
                源码输入摘要：<code>{preview.executionContext.sourceFingerprint}</code>
              </p>
              <p>这是执行时的本地观察；没有据此确认板卡或工具链版本。</p>
            </details>
          ) : (
            <p>此旧记录没有执行时环境快照，保留为本地历史；不能补造用于同步的环境。</p>
          )}
          {preview.items.map((item) => (
            <details key={item.kind}>
              <summary>
                {titles[item.kind]} · {item.size} 字节{item.redacted ? ' · 已脱敏' : ''}
                {item.outputTruncated ? ' · 原执行输出超限' : ''}
              </summary>
              {item.outputTruncated && (
                <p className="learn-notice">
                  执行时已达到输出上限，以下是当时保留的全部内容，不能作为完整程序输出。
                </p>
              )}
              <p>
                执行时间：{item.startedAt} 至 {item.finishedAt}
              </p>
              <textarea
                aria-label={`${titles[item.kind]}完整内容`}
                readOnly
                rows={8}
                value={item.text}
              />
              {item.size === 0 && <p>标准输出为空（0 字节）。</p>}
              <details>
                <summary>查看校验信息</summary>
                <p>
                  已保存内容：{item.size} 字节
                  <br />
                  <code>{item.sha256}</code>
                </p>
                <p>
                  脱敏前内容：{item.rawBytes} 字节
                  <br />
                  <code>{item.rawSha256}</code>
                </p>
              </details>
            </details>
          ))}
          <details>
            <summary>本地快照校验信息</summary>
            <code>{preview.snapshotSha256}</code>
          </details>
        </div>
      )}
    </section>
  );
}
