import { useEffect, useRef, useState } from 'react';
import { invokeCommand } from '../../../Platform/IPC/Client';
import type { LearnActionRequest, LearnBoardObservation } from '../../../Platform/IPC/Commands';
import { learnServiceError } from './serviceStatus';

const labels = {
  unknown: '采集状态未知',
  observed: '已采集本地观察',
  failed: '采集失败',
  stale: '采集期间绑定或账号发生变化',
};

export function LearnGenericBoard({
  active,
  request,
}: {
  active: boolean;
  request: LearnActionRequest;
}) {
  const [records, setRecords] = useState<LearnBoardObservation[]>([]);
  const [busy, setBusy] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState('');
  const [reload, setReload] = useState(0);
  const sequence = useRef(0);
  const key = JSON.stringify(request);
  useEffect(() => {
    const seq = ++sequence.current;
    setRecords([]);
    setError('');
    setLoaded(false);
    setBusy(false);
    if (!active) return;
    setBusy(true);
    void invokeCommand('learn_list_board_observations', {
      request: JSON.parse(key) as LearnActionRequest,
    })
      .then((value) => {
        if (seq === sequence.current) {
          setRecords(value);
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
  }, [active, key, reload]);
  async function observe() {
    if (!active || busy || !loaded) return;
    const seq = ++sequence.current;
    setBusy(true);
    setError('');
    try {
      const value = await invokeCommand('learn_observe_board', { request });
      if (seq === sequence.current)
        setRecords((previous) => [
          value,
          ...previous.filter((r) => r.observationId !== value.observationId),
        ]);
    } catch (e) {
      if (seq === sequence.current) setError(learnServiceError(e));
    } finally {
      if (seq === sequence.current) setBusy(false);
    }
  }
  return (
    <section className="learn-panel learn-generic-board" aria-label="课程板卡观察">
      <h3>课程板卡观察</h3>
      <p>
        读取此课程工程已绑定的 Linux 板卡。请先在工作台确认 ADB 连接，或打开对应串口并进入 Shell。
      </p>
      <p>这里保存系统与运行状态的本地观察；不代表部署、实机验收或云端评测通过。</p>
      {error && (
        <p role="alert" className="learn-notice">
          {error}
        </p>
      )}
      <div className="learn-hub-actions">
        <button
          className="learning-primary"
          disabled={!active || busy || !loaded}
          onClick={() => void observe()}
        >
          {busy ? '处理中…' : '读取已绑定板卡状态'}
        </button>
        <button
          className="learning-secondary"
          disabled={!active || busy}
          onClick={() => setReload((n) => n + 1)}
        >
          刷新本地观察记录
        </button>
      </div>
      {records.map((record) => (
        <section className="learn-action-attempt" key={record.observationId}>
          <h4>
            {labels[record.state]} · {record.boardName}
          </h4>
          <p>
            {record.projectName} · {record.startedAt}
          </p>
          {record.state === 'unknown' && (
            <p role="status">上次采集未确认结束；记录不会自动重放。需要时可主动重新读取。</p>
          )}
          {record.state === 'stale' && (
            <p role="status">
              以下是旧绑定下的观察，不能作为当前目标的结果；请核对绑定后重新读取。
            </p>
          )}
          {record.errorCode && (
            <p role="alert">{record.errorCode}：请检查板卡绑定、连接及硬件占用，再主动重试。</p>
          )}
          {record.snapshot && (
            <dl>
              <dt>采集时间</dt>
              <dd>{new Date(record.snapshot.observedAtMs).toLocaleString()}</dd>
              <dt>通道</dt>
              <dd>{record.snapshot.transport}</dd>
              <dt>系统 / 内核</dt>
              <dd>
                {record.snapshot.osName || '未提供'} · {record.snapshot.kernel || '未提供'}
              </dd>
              <dt>架构 / CPU</dt>
              <dd>
                {record.snapshot.architecture || '未提供'} · {record.snapshot.cpuModel || '未提供'}
              </dd>
              <dt>已保存板卡</dt>
              <dd>
                <code>{record.boardUid}</code>
              </dd>
            </dl>
          )}
          {record.evidenceSha256 && (
            <details>
              <summary>本地证据摘要</summary>
              <code>{record.evidenceSha256}</code>
            </details>
          )}
          <small>仅本机保存 · 尚未获云端接受 · 观察结果不等于评测通过</small>
        </section>
      ))}
    </section>
  );
}
