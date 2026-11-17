import { useEffect, useRef, useState } from 'react';
import './LocalCommandDiagnostics.css';
import { invokeCommand } from '../../../Platform/IPC/Client';
import type {
  LocalCommandDiagnostic,
  LocalCommandDiagnosticScope,
} from '../../../Platform/IPC/Commands';
import { formatErrorMessage } from '../../../Utils';

export function LocalCommandDiagnostics({
  active,
  scope,
  onSelectForTutor,
}: {
  active: boolean;
  scope: LocalCommandDiagnosticScope;
  onSelectForTutor?: (value: LocalCommandDiagnostic) => void;
}) {
  const [value, setValue] = useState<LocalCommandDiagnostic | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const generation = useRef(0);
  const key = JSON.stringify(scope);
  useEffect(() => {
    generation.current++;
    setValue(null);
    setBusy(false);
    setError('');
    return () => {
      generation.current++;
    };
  }, [active, key]);
  async function read() {
    if (!active || busy) return;
    const current = ++generation.current;
    setBusy(true);
    setError('');
    setValue(null);
    try {
      const result = await invokeCommand('learning_get_local_command_diagnostic', {
        request: scope,
      });
      if (current !== generation.current) return;
      if (
        Object.entries(scope).some(
          ([field, expected]) => result[field as keyof LocalCommandDiagnosticScope] !== expected
        )
      ) {
        throw new Error('诊断作用域不匹配，请重新打开当前记录。');
      }
      setValue(result);
    } catch (reason) {
      if (current === generation.current) setError(formatErrorMessage(reason));
    } finally {
      if (current === generation.current) setBusy(false);
    }
  }
  if (!active) return null;
  return (
    <section className="local-command-diagnostics" aria-label="本地命令诊断">
      <button className="learning-secondary" disabled={busy} onClick={() => void read()}>
        {busy ? '正在读取诊断…' : value ? '重新读取命令诊断' : '查看命令诊断'}
      </button>
      {error && (
        <p role="alert" className="learn-notice">
          {error}
        </p>
      )}
      {value && (
        <div>
          <p>
            记录结果：{value.status} · {new Date(value.createdAt * 1000).toLocaleString()}
          </p>
          {value.availability === 'legacy_unavailable' ? (
            <p role="status">旧记录未保存可核验的诊断内容；保留原结果，不补造日志。</p>
          ) : (
            <>
              <p>
                实际退出码：{value.exitCode ?? '未取得'} · 课程期望：
                {value.expectedExitCode ?? '未取得'}
              </p>
              <p>已保存的脱敏合并输出；只在本机读取，不重新执行或上传。</p>
              {value.truncated && (
                <p className="learn-notice">
                  输出已截断，仅保留脱敏后的末尾 2048 个字符；这不是完整日志。
                </p>
              )}
              {value.text === '' ? (
                <p role="status">本次命令没有输出（0 字节）。</p>
              ) : (
                <pre className="learn-log-output" aria-label="已保存的命令诊断">
                  {value.text}
                </pre>
              )}
              <small>自动脱敏不能识别所有敏感信息。诊断内容不代表设备验证或权威评测通过。</small>
              {onSelectForTutor && (
                <button className="learning-secondary" onClick={() => onSelectForTutor(value)}>
                  附到本次 AI 提问
                </button>
              )}
            </>
          )}
        </div>
      )}
    </section>
  );
}
