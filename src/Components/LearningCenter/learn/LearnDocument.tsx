import { createElement, useId, useMemo, useState, type ReactNode } from 'react';
import { documentView, documentUrl, type DocumentNode } from './documentModel';
import { SafeMarkdown, SourceImage, SourceLink } from './LearnMarkdown';

/** Display verified snapshots without changing their original bytes or evidence digest. */
export function LearnDocument({ source, sourceUrl }: { source: string; sourceUrl?: string }) {
  const prefix = useId();
  const [imagesEnabled, setImagesEnabled] = useState(false);
  const view = useMemo(() => documentView(source, sourceUrl), [source, sourceUrl]);
  if (!view) return <SafeMarkdown source={source} />;
  if (view.error) return <p role="alert">{view.error}</p>;
  if (!view.nodes.length) return <p role="status">本节教材没有可显示的正文。</p>;
  const flatten = (nodes: DocumentNode[]): DocumentNode[] =>
    nodes.flatMap((n) => [n, ...flatten(n.children)]);
  const all = flatten(view.nodes);
  const headings = all.filter((n) => /^h[1-3]$/.test(n.tag));
  const images = all.filter((n) => n.tag === 'img').length;
  const sourceLink = sourceUrl && documentUrl(sourceUrl);
  const label = (node: DocumentNode): string => node.text || node.children.map(label).join('');
  const locate = (id: string) =>
    document.getElementById(`${prefix}-${id}`)?.scrollIntoView({ block: 'start' });
  function render(node: DocumentNode, key: string): ReactNode {
    if (node.tag === '#text') return node.text;
    const children = node.children.map((n, i) => render(n, `${key}.${i}`));
    const id = node.id ? `${prefix}-${node.id}` : undefined;
    if (node.tag === 'img')
      return (
        <SourceImage
          key={key}
          url={node.url!}
          alt={node.text || ''}
          enabled={imagesEnabled}
          compact
        />
      );
    if (node.tag === 'a') {
      if (node.fragment)
        return (
          <a
            key={key}
            id={id}
            href={`#${prefix}-${node.fragment}`}
            onClick={(e) => {
              e.preventDefault();
              e.currentTarget
                .closest('[data-testid="learn-document"]')
                ?.querySelectorAll('[id]')
                .forEach((target) => {
                  if (target.id === `${prefix}-${node.fragment}`)
                    target.scrollIntoView({ block: 'nearest' });
                });
            }}
          >
            {children}
          </a>
        );
      return (
        <SourceLink key={key} url={node.url!} label="">
          {children}
        </SourceLink>
      );
    }
    if (node.tag === 'br' || node.tag === 'hr') return createElement(node.tag, { key, id });
    if (node.tag === 'table')
      return (
        <div key={key} className="learn-document-table">
          {createElement('table', { id }, children)}
        </div>
      );
    return createElement(node.tag, { key, id }, children);
  }
  return (
    <section className="learn-document-reader" aria-label="课程教材">
      <div className="learn-reader-tools">
        <strong>课程教材</strong>
        {sourceLink && <SourceLink url={sourceLink} label="查看来源" />}
        {headings.length > 1 && (
          <details className="learn-reader-outline">
            <summary>本文目录</summary>
            <nav aria-label="本文目录">
              {headings.map((n, i) => (
                <button
                  key={i}
                  type="button"
                  className="learning-secondary"
                  onClick={(e) => {
                    e.currentTarget.closest('details')?.removeAttribute('open');
                    locate(n.id!);
                  }}
                >
                  {label(n)}
                </button>
              ))}
            </nav>
          </details>
        )}
        {images > 0 && (
          <button
            type="button"
            className="learning-secondary"
            disabled={imagesEnabled}
            onClick={() => setImagesEnabled(true)}
          >
            {imagesEnabled ? '本节图片已启用' : `加载本节图片（${images}）`}
          </button>
        )}
      </div>
      {images > 0 && (
        <p className="learn-reader-image-note">
          插图来自教材站点，点击后加载；实验取证请使用实测记录。
        </p>
      )}
      <div className="learn-document-markdown learn-document-html" data-testid="learn-document">
        {view.nodes.map((n, i) => render(n, String(i)))}
      </div>
    </section>
  );
}
