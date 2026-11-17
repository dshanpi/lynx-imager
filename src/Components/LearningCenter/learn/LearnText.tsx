import { useEffect, useState } from 'react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { invokeCommand } from '../../../Platform/IPC/Client';
import { formatErrorMessage } from '../../../Utils';
import { LynxIcon } from '../../../CoreUI';
import { SafeMarkdown } from './LearnMarkdown';

export function LearnText({
  active,
  sessionId,
  stepId,
  title,
}: {
  active: boolean;
  sessionId: string;
  stepId: string;
  title: string;
}) {
  const [document, setDocument] = useState<{
    markdown: string;
    sourceUrl: string;
    sha256: string;
  } | null>(null);
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  useEffect(() => {
    if (!active) return;
    let cancelled = false;
    setDocument(null);
    setError('');
    setLoading(true);
    void invokeCommand('learning_read_document', { sessionId, stepId })
      .then((value) => {
        if (!cancelled) setDocument(value);
      })
      .catch((reason) => {
        if (!cancelled) setError(formatErrorMessage(reason));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [active, sessionId, stepId]);
  return (
    <section className="learn-text">
      {!document && <h1>{title}</h1>}
      <p className="learn-muted">
        {loading
          ? '正在核对教材快照…'
          : error ||
            (!document
              ? '当前课程包未声明完整教材快照，下方展示该签名版本中的任务讲义。'
              : '教材已按课程声明的摘要核验。')}
      </p>
      {document && (
        <>
          <button
            className="learning-text-button"
            onClick={() =>
              void openUrl(document.sourceUrl).catch((reason) =>
                setError(formatErrorMessage(reason))
              )
            }
          >
            查看在线原文 ↗
          </button>
          <SafeMarkdown source={document.markdown} />
          {/开发主机|目标开发板|开发环境/.test(document.markdown) && (
            <details className="learn-environment-guide" open>
              <summary>开发环境关系示意</summary>
              <div className="learn-environment-diagram">
                <div>
                  <LynxIcon name="desktop" size={48} />
                  <strong>Ubuntu 开发主机</strong>
                  <small>编写代码 · 构建程序</small>
                </div>
                <span>构建与部署 →</span>
                <div>
                  <LynxIcon name="board" size={48} />
                  <strong>目标开发板</strong>
                  <small>运行程序 · 输出日志</small>
                </div>
                <span>观察与验证 →</span>
                <div>
                  <LynxIcon name="desktop" size={48} />
                  <strong>LYNX 工作台</strong>
                  <small>组织任务 · 查看证据</small>
                </div>
              </div>
              <small>概念示意；具体接线、运行环境和步骤以本课教材为准。</small>
            </details>
          )}
          <details>
            <summary>教材来源与版本摘要</summary>
            <p>{document.sourceUrl}</p>
            <code>{document.sha256}</code>
          </details>
        </>
      )}
    </section>
  );
}
