import { useEffect, useRef, useState } from 'react';
import { invokeCommand } from '../../../Platform/IPC/Client';
import type { LearningDeepLinkTarget } from '../types';

export function LearnCourseLink({
  active,
  onOpen,
}: {
  active: boolean;
  onOpen: (target: LearningDeepLinkTarget) => void;
}) {
  const [value, setValue] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const generation = useRef(0);
  const locked = useRef(false);
  useEffect(() => {
    generation.current++;
    locked.current = false;
    setBusy(false);
    return () => {
      generation.current++;
    };
  }, [active]);
  return (
    <details className="learn-course-link">
      <summary>已有官网课程链接？在此打开</summary>
      <p>粘贴官网的 LYNX 课程或章节链接，在当前客户端继续。课程权益由登录账号核验。</p>
      <form
        onSubmit={async (event) => {
          event.preventDefault();
          if (!active || locked.current || !value.trim()) return;
          locked.current = true;
          const request = ++generation.current;
          setBusy(true);
          setError('');
          try {
            const target = await invokeCommand('learn_parse_course_link', { value });
            if (request === generation.current) onOpen(target);
          } catch {
            if (request === generation.current)
              setError(
                '请粘贴 100ask.net/lynx-learn/ 下的课程、章节链接，或官网提供的 lynx://learn/ 链接。不要包含查询参数或登录凭据。'
              );
          } finally {
            if (request === generation.current) {
              locked.current = false;
              setBusy(false);
            }
          }
        }}
      >
        <label>
          <span>官网学习链接</span>
          <input
            aria-label="官网学习链接"
            type="text"
            maxLength={2048}
            autoComplete="off"
            spellCheck={false}
            value={value}
            disabled={busy}
            onChange={(e) => {
              setValue(e.target.value);
              setError('');
            }}
            placeholder="https://www.100ask.net/lynx-learn/class-…"
          />
        </label>
        <button
          type="submit"
          className="learning-secondary"
          disabled={!active || busy || !value.trim()}
        >
          {busy ? '正在解析…' : '在当前 LYNX 打开'}
        </button>
      </form>
      {error && <p role="alert">{error}</p>}
    </details>
  );
}
