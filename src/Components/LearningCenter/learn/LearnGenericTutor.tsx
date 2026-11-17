import { useEffect, useRef, useState } from 'react';
import { invokeCommand } from '../../../Platform/IPC/Client';
import type { LearnCoursePin, LearnVerifiedCourse } from '../../../Platform/IPC/Commands';
import { SafeMarkdown } from './LearnMarkdown';

const labels: Record<string, string> = {
  explain: '解释原理',
  hint: '提示方向',
  example: '局部示例',
  solution: '完整示范',
};
export function LearnGenericTutor({
  active,
  pin,
  unitId,
  policy,
  engineeringActivityId,
}: {
  active: boolean;
  pin: LearnCoursePin;
  unitId: string;
  policy: LearnVerifiedCourse['agentPolicy'];
  engineeringActivityId?: string;
}) {
  const [question, setQuestion] = useState('');
  const [level, setLevel] = useState(policy.hintLevels[0] || 'explain');
  const [answer, setAnswer] = useState('');
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [includeEngineering, setIncludeEngineering] = useState(false);
  const [sourcePaths, setSourcePaths] = useState('');
  const generation = useRef(0);
  useEffect(() => {
    generation.current++;
    setBusy(false);
    return () => {
      generation.current++;
    };
  }, [active]);
  async function ask() {
    if (!active || busy || !question.trim()) return;
    const paths = sourcePaths
      .split('\n')
      .map((path) => path.trim())
      .filter(Boolean);
    if (
      includeEngineering &&
      (!paths.length || paths.length > 8 || new Set(paths).size !== paths.length)
    ) {
      setError('请填写 1 到 8 个不同的源码文件路径，每行一个。');
      return;
    }
    const current = ++generation.current;
    setBusy(true);
    setError('');
    setAnswer('');
    try {
      const result = await invokeCommand('learn_ask_tutor', {
        request: {
          ...pin,
          unitId,
          question,
          hintLevel: level,
          ...(includeEngineering && engineeringActivityId
            ? { engineering: { activityId: engineeringActivityId, sourcePaths: paths } }
            : {}),
        },
      });
      if (current === generation.current) setAnswer(result.answer);
    } catch (e) {
      if (current === generation.current)
        setError(
          typeof e === 'string'
            ? e.split(':').slice(1).join(':') || 'AI 指导暂不可用，请检查 Provider 配置。'
            : 'AI 指导暂不可用，请检查 Provider 配置。'
        );
    } finally {
      if (current === generation.current) setBusy(false);
    }
  }
  if (!policy.enabled || !policy.hintLevels.length)
    return <p>本课程尚未启用 AI 指导，可先按教材步骤学习。</p>;
  return (
    <section aria-label="当前课程 AI 指导">
      <h3>问 AI 老师</h3>
      <label>
        提示深度
        <select value={level} onChange={(e) => setLevel(e.target.value)} disabled={busy}>
          {policy.hintLevels.map((value) => (
            <option key={value} value={value}>
              {labels[value] || value}
            </option>
          ))}
        </select>
      </label>
      <label>
        你的问题
        <textarea
          value={question}
          onChange={(e) => setQuestion(e.target.value)}
          maxLength={1000}
          rows={4}
          placeholder="这一步为什么这样做？我应该先检查什么？"
          disabled={busy}
        />
      </label>
      {engineeringActivityId && (
        <fieldset disabled={busy}>
          <label>
            <input
              type="checkbox"
              checked={includeEngineering}
              onChange={(e) => setIncludeEngineering(e.target.checked)}
            />
            附带当前工程差异和实践记录
          </label>
          {includeEngineering && (
            <label>
              允许读取的源码文件（工程内相对路径，每行一个）
              <textarea
                value={sourcePaths}
                onChange={(e) => setSourcePaths(e.target.value)}
                maxLength={2056}
                rows={3}
                placeholder={'src/main.c\ninclude/main.h'}
              />
            </label>
          )}
          <small>
            需要已绑定的 Ubuntu
            工程和课程固定提交。仅读取所选已跟踪源码的差异，附带环境与本活动最近三次本地任务记录；内容脱敏并限制大小。未跟踪文件不发送。
          </small>
        </fieldset>
      )}
      <button
        className="learning-secondary"
        disabled={!active || busy || !question.trim()}
        onClick={() => {
          void ask();
        }}
      >
        {busy ? '正在分析本课教材…' : '结合本课教材回答'}
      </button>
      <p className="learn-muted">
        发送范围：当前课节中课程策略允许的教材与脱敏问题
        {includeEngineering ? '，以及上面明确选择的工程差异和本地实践记录' : ''}。
        历史任务结果不代表当前源码仍然通过。回答供学习参考，不能作为设备验证证据。
      </p>
      {error && <p role="alert">{error}</p>}
      {answer && (
        <div role="status">
          <SafeMarkdown source={answer} />
        </div>
      )}
    </section>
  );
}
