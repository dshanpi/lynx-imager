import { useMemo, useState, type ReactNode } from 'react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { markdownBlocks, safeCourseLink } from './markdownModel';
export function SourceImage({
  url,
  alt,
  enabled = false,
  compact = false,
}: {
  url: string;
  alt: string;
  enabled?: boolean;
  compact?: boolean;
}) {
  const [loaded, setLoaded] = useState(false);
  const [failed, setFailed] = useState(false);
  return (
    <span className={`learn-source-image${compact ? ' learn-source-image-compact' : ''}`}>
      {(loaded || enabled) && !failed ? (
        <img
          src={url}
          alt={alt || '教材插图'}
          loading="lazy"
          referrerPolicy="no-referrer"
          onError={() => setFailed(true)}
        />
      ) : (
        <button
          className="learning-secondary"
          onClick={() => {
            setFailed(false);
            setLoaded(true);
          }}
        >
          {failed ? '插图加载失败，重试' : `加载教材插图${alt ? `：${alt}` : ''}`}
        </button>
      )}
      {compact ? (
        (loaded || enabled) && !failed && alt && <small>{alt}</small>
      ) : (
        <small>来源插图 · 外部内容，不作为实验证据</small>
      )}
    </span>
  );
}
export function SourceLink({
  url,
  label,
  children,
}: {
  url: string;
  label: string;
  children?: ReactNode;
}) {
  const [failed, setFailed] = useState(false);
  return (
    <span>
      <a
        href={url}
        onClick={(e) => {
          e.preventDefault();
          setFailed(false);
          void openUrl(url).catch(() => setFailed(true));
        }}
      >
        {children || label || url} ↗
      </a>
      {failed && <span role="alert"> 无法打开外部链接，请重试或复制链接地址。</span>}
    </span>
  );
}
function inline(text: string): ReactNode[] {
  const output: ReactNode[] = [];
  const pattern = /(!?)\[([^\]\n]*)\]\((https:\/\/[^\s)]+)\)|`([^`\n]+)`|\*\*([^*\n]+)\*\*/g;
  let last = 0;
  let match: RegExpExecArray | null;
  while ((match = pattern.exec(text))) {
    output.push(text.slice(last, match.index));
    const key = `${match.index}:${match[0]}`;
    if (match[4]) output.push(<code key={key}>{match[4]}</code>);
    else if (match[5]) output.push(<strong key={key}>{match[5]}</strong>);
    else {
      const url = safeCourseLink(match[3]);
      if (!url) output.push(match[0]);
      else if (match[1]) output.push(<SourceImage key={key} url={url} alt={match[2]} />);
      else output.push(<SourceLink key={key} url={url} label={match[2]} />);
    }
    last = pattern.lastIndex;
  }
  output.push(text.slice(last));
  return output;
}
export function SafeMarkdown({ source }: { source: string }) {
  const blocks = useMemo(() => markdownBlocks(source), [source]);
  if (!blocks.length)
    return <div className="learn-document-document-loading">本节文档快照为空</div>;
  return (
    <div className="learn-document-markdown">
      {blocks.map((block, index) => {
        const key = `${block.kind}:${index}`;
        if (block.kind === 'code')
          return (
            <pre key={key}>
              <code>{block.text}</code>
            </pre>
          );
        const text = inline(block.text);
        if (block.kind === 'h1') return <h1 key={key}>{text}</h1>;
        if (block.kind === 'h2') return <h2 key={key}>{text}</h2>;
        if (block.kind === 'h3') return <h3 key={key}>{text}</h3>;
        if (block.kind === 'li')
          return (
            <div className="md-list" key={key}>
              • {text}
            </div>
          );
        return <p key={key}>{text}</p>;
      })}
    </div>
  );
}
