import type { LearnPreviewList } from '../../../Platform/IPC/Commands';

export function LearnPreviewShelf({
  list,
  query,
  busy,
  onOpen,
}: {
  list: LearnPreviewList | null;
  query: string;
  busy: boolean;
  onOpen: (id: string) => void;
}) {
  if (!list) return null;
  const items = list.items.filter((p) =>
    p.title.toLowerCase().includes(query.trim().toLowerCase())
  );
  return (
    <section className="learn-panel learn-preview-shelf" aria-label="官网试学课程">
      <h2>官网试学课程</h2>
      <p>仅你获邀的测试课程 · 在线核验后阅读 · 不计入正式课程成绩</p>
      {!list.discoveryAvailable && (
        <p role="status">官网暂未提供新邀请自动发现接口。已接收的试学会在此自动恢复并重新核验。</p>
      )}
      {list.warnings.map((message, i) => (
        <p role="alert" key={i}>
          {message}
        </p>
      ))}
      {items.map((p) => (
        <article className="learn-panel" key={p.previewId}>
          <h3>{p.title}</h3>
          <p>
            <span className="learn-chip">获邀试学</span> 修订 {p.draftRevision} · {p.chapterCount}{' '}
            章 · {p.unitCount} 节
          </p>
          <p>有效至 {new Date(p.expiresAt).toLocaleString()}</p>
          <button
            disabled={busy || Date.parse(p.expiresAt) <= Date.now()}
            onClick={() => onOpen(p.previewId)}
          >
            {p.currentUnitId ? '继续在线试学' : '在线试学'}
          </button>
        </article>
      ))}
      {!items.length && list.discoveryAvailable && !list.warnings.length && (
        <p>当前账号没有有效试学邀请。</p>
      )}
    </section>
  );
}
