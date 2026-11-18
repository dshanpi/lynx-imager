import { useEffect, useRef, useState } from 'react';
import { invokeCommand, type WorkbenchTaskHistory } from '../../Platform/IPC';
import { formatErrorMessage } from '../../Utils';
import {
  inspectSerialEvidence,
  selectBootAcceptance,
  sameBootScope,
  type BootReport,
  type BootScope,
} from './writeEvidence';

export const bootRecordKey = (taskId: string) => 'lynx.imager.bootEvidence.' + taskId;
export function readLocalBoot(taskId: string, deviceId: number | null): BootReport | null {
  const raw: unknown = JSON.parse(localStorage.getItem(bootRecordKey(taskId)) || 'null');
  if (!raw || typeof raw !== 'object') return null;
  const r = raw as Partial<BootReport>;
  return r.taskId === taskId &&
    r.deviceId === deviceId &&
    typeof r.imageSha256 === 'string' &&
    typeof r.evidence === 'string' &&
    typeof r.operator === 'string' &&
    typeof r.recordedAt === 'number' &&
    (r.result === 'pass' || r.result === 'fail') &&
    typeof r.coldBoot === 'boolean'
    ? (r as BootReport)
    : null;
}

export async function loadRecordedBoot(task: WorkbenchTaskHistory) {
  if (task.status !== 'success' || task.kind !== 'flash' || task.deviceId == null) return null;
  const expected = task.details?.physicalImageSha256;
  const local = readLocalBoot(task.taskId, task.deviceId);
  if (local && (!expected || local.imageSha256.toLowerCase() === expected.toLowerCase()))
    return local;
  const ws = await invokeCommand('workbench_firmware_workspace', { deviceId: task.deviceId });
  return selectBootAcceptance(ws.acceptance, task.taskId, task.deviceId, expected);
}

export function useBootEvidence(scope: BootScope | null, candidateId?: string) {
  const [report, setReport] = useState<BootReport | null>(null);
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  const { taskId, deviceId, imageSha256 } = scope ?? {};
  const [version, refresh] = useState(0);
  useEffect(() => {
    let active = true;
    setReport(null);
    setError('');
    setLoading(false);
    if (!taskId || deviceId == null || !imageSha256) return;
    async function load(s: BootScope) {
      if (!candidateId) {
        const local = readLocalBoot(s.taskId, s.deviceId);
        return local && sameBootScope(local, s) ? local : null;
      }
      const ws = await invokeCommand('workbench_firmware_workspace', { deviceId: s.deviceId });
      return selectBootAcceptance(ws.acceptance, s.taskId, s.deviceId, s.imageSha256, candidateId);
    }
    setLoading(true);
    load({ taskId, deviceId, imageSha256 })
      .then((result) => {
        if (active) setReport(result);
      })
      .catch((e) => {
        if (active) setError(formatErrorMessage(e));
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => {
      active = false;
    };
  }, [taskId, deviceId, imageSha256, candidateId, version]);
  return {
    report: scope && report && sameBootScope(report, scope) ? report : null,
    error,
    loading,
    refresh: () => refresh((i) => i + 1),
  };
}

export function BootEvidence({
  scope,
  candidateId,
  onSaved,
}: {
  scope: BootScope;
  candidateId?: string;
  onSaved: () => void;
}) {
  const [file, setFile] = useState<File | null>(null);
  const [text, setText] = useState('');
  const [marker, setMarker] = useState('');
  const [operator, setOperator] = useState('');
  const [attested, setAttested] = useState(false);
  const [cold, setCold] = useState(false);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const submitting = useRef(false);
  useEffect(() => {
    let active = true;
    setError('');
    if (file && file.size > 2 * 1024 * 1024)
      setError('日志超过 2 MiB，请选择包含本次启动标识的串口片段。');
    else if (file)
      file
        .text()
        .then((value) => {
          if (active) setText(value);
        })
        .catch((e) => {
          if (active) setError(formatErrorMessage(e));
        });
    return () => {
      active = false;
    };
  }, [file]);
  const match = inspectSerialEvidence(text, marker);
  async function record() {
    if (!file || !match || !attested || !operator.trim() || submitting.current) return;
    submitting.current = true;
    setBusy(true);
    setError('');
    try {
      const sample = [match.first, match.last].filter(Boolean).join(' ｜ ');
      const evidence = file.name.slice(0, 180) + ' · 匹配 ' + match.count + ' 行 · ' + sample;
      const report: BootReport = {
        ...scope,
        result: 'pass',
        recordedAt: Math.floor(Date.now() / 1000),
        operator: operator.trim(),
        evidence,
        coldBoot: cold,
      };
      if (candidateId) {
        await invokeCommand('workbench_firmware_acceptance', {
          deviceId: scope.deviceId,
          candidateId,
          taskId: scope.taskId,
          operator: report.operator,
          items: [
            { item: 'boot', result: 'pass', evidence },
            ...(cold ? [{ item: 'power_cycle', result: 'pass' as const, evidence }] : []),
          ],
        });
      } else {
        localStorage.setItem(bootRecordKey(scope.taskId), JSON.stringify(report));
      }
      onSaved();
    } catch (e) {
      setError(formatErrorMessage(e));
    } finally {
      submitting.current = false;
      setBusy(false);
    }
  }
  return (
    <details className="v2w-boot-form">
      <summary>登记启动证据</summary>
      <fieldset disabled={busy}>
        <label>
          本次启动的串口日志
          <input
            type="file"
            accept=".txt,.log,text/plain"
            onChange={(e) => {
              setText('');
              setAttested(false);
              setCold(false);
              setFile(e.target.files?.[0] ?? null);
            }}
          />
        </label>
        <label>
          预期应用或构建标识
          <input
            placeholder="输入镜像预期输出的标识"
            maxLength={120}
            value={marker}
            onChange={(e) => {
              setMarker(e.target.value);
              setAttested(false);
            }}
          />
        </label>
        {text && marker.trim().length >= 4 && (
          <p role="status">{match ? '匹配 ' + match.count + ' 行' : '串口输出中未找到该标识。'}</p>
        )}
        {match && <pre>{[match.first, match.last].filter(Boolean).join('\n')}</pre>}
        <label>
          验收人
          <input value={operator} maxLength={80} onChange={(e) => setOperator(e.target.value)} />
        </label>
        <label>
          <input
            type="checkbox"
            checked={attested}
            onChange={(e) => setAttested(e.target.checked)}
          />
          我已核对日志来自该目标在本次烧写后的启动
        </label>
        <label>
          <input type="checkbox" checked={cold} onChange={(e) => setCold(e.target.checked)} />
          此日志在独立断电上电后采集
        </label>
        <button
          type="button"
          className="v2w-button"
          disabled={!match || !attested || !operator.trim() || busy}
          onClick={() => void record()}
        >
          {busy ? '保存中…' : '保存本次验收'}
        </button>
      </fieldset>
      {error && <p role="alert">{error}</p>}
    </details>
  );
}
