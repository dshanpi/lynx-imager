import { useEffect, useRef, useState } from 'react';
import { invokeCommand } from '../../../Platform/IPC/Client';
import type {
  LocalArtifactList,
  LocalArtifactMeta,
  LocalCommandDiagnosticScope,
} from '../../../Platform/IPC/Commands';
import { formatLocalLearningError } from './errors';
import './LocalArtifacts.css';

const CHUNK = 65536;
const TYPES = new Set([
  'application/json',
  'text/plain',
  'audio/wav',
  'audio/flac',
  'video/mp4',
  'application/octet-stream',
]);
const digestPattern = /^[a-f0-9]{64}$/;
function sameScope(a: LocalCommandDiagnosticScope, b: LocalCommandDiagnosticScope) {
  return (
    a.sessionId === b.sessionId &&
    a.stepId === b.stepId &&
    a.attempt === b.attempt &&
    a.digest === b.digest
  );
}
function valid(meta: LocalArtifactMeta) {
  return (
    Number.isSafeInteger(meta.bytes) &&
    meta.bytes >= 0 &&
    meta.bytes <= 32 * 1024 * 1024 &&
    meta.bytes <= meta.spec.maxBytes &&
    meta.chunkCount === Math.ceil(meta.bytes / CHUNK) &&
    digestPattern.test(meta.sha256) &&
    digestPattern.test(meta.chunkRoot) &&
    TYPES.has(meta.spec.mimeType)
  );
}

export function LocalArtifacts({
  active,
  scope,
}: {
  active: boolean;
  scope: LocalCommandDiagnosticScope;
}) {
  const [list, setList] = useState<LocalArtifactList | null>(null);
  const [preview, setPreview] = useState<{
    meta: LocalArtifactMeta;
    url: string;
    text: string | null;
    truncated: boolean;
  } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const sequence = useRef(0);
  const objectUrl = useRef<string | null>(null);
  const key = JSON.stringify(scope);
  function clearPreview() {
    if (objectUrl.current) URL.revokeObjectURL(objectUrl.current);
    objectUrl.current = null;
    setPreview(null);
  }
  useEffect(() => {
    sequence.current++;
    if (objectUrl.current) URL.revokeObjectURL(objectUrl.current);
    objectUrl.current = null;
    setList(null);
    setPreview(null);
    setBusy(false);
    setError('');
    return () => {
      sequence.current++;
      if (objectUrl.current) URL.revokeObjectURL(objectUrl.current);
      objectUrl.current = null;
    };
  }, [active, key]);
  async function loadList() {
    if (!active || busy) return;
    const current = ++sequence.current;
    setBusy(true);
    setError('');
    setList(null);
    clearPreview();
    try {
      const result = await invokeCommand('learning_list_local_artifacts', { request: scope });
      if (current !== sequence.current) return;
      if (
        !sameScope(result.scope, scope) ||
        result.items.length > 8 ||
        result.items.some((m) => !valid(m)) ||
        new Set(result.items.map((m) => m.spec.artifactId)).size !== result.items.length ||
        result.items.reduce((sum, m) => sum + m.bytes, 0) > 64 * 1024 * 1024
      ) {
        throw new Error('产物列表范围或格式不匹配，已阻止显示。');
      }
      setList(result);
    } catch (reason) {
      if (current === sequence.current) setError(formatLocalLearningError(reason));
    } finally {
      if (current === sequence.current) setBusy(false);
    }
  }
  async function open(meta: LocalArtifactMeta) {
    if (!active || busy || !list || !sameScope(list.scope, scope) || !valid(meta)) return;
    const current = ++sequence.current;
    setBusy(true);
    setError('');
    clearPreview();
    try {
      const bytes = new Uint8Array(meta.bytes);
      let offset = 0;
      while (offset < meta.bytes) {
        const result = await invokeCommand('learning_read_local_artifact_chunk', {
          request: { scope, artifactId: meta.spec.artifactId, sha256: meta.sha256, offset },
        });
        if (current !== sequence.current) return;
        if (
          !sameScope(result.scope, scope) ||
          result.artifactId !== meta.spec.artifactId ||
          result.sha256 !== meta.sha256 ||
          result.offset !== offset ||
          result.bytes !== meta.bytes ||
          result.dataBase64.length > 4 * Math.ceil(CHUNK / 3)
        )
          throw new Error('产物分块范围不匹配。');
        const part = Uint8Array.from(atob(result.dataBase64), (c) => c.charCodeAt(0));
        if (
          part.length !== Math.min(CHUNK, meta.bytes - offset) ||
          result.nextOffset !== offset + part.length
        ) {
          throw new Error('产物分块长度不匹配。');
        }
        bytes.set(part, offset);
        offset = result.nextOffset;
      }
      const hash = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', bytes)), (n) =>
        n.toString(16).padStart(2, '0')
      ).join('');
      if (current !== sequence.current) return;
      if (hash !== meta.sha256) throw new Error('产物内容摘要不匹配，未创建预览或下载。');
      const isText =
        meta.spec.mimeType === 'text/plain' || meta.spec.mimeType === 'application/json';
      const truncated = isText && bytes.length > CHUNK;
      const text = isText
        ? new TextDecoder('utf-8', { fatal: true }).decode(bytes.subarray(0, CHUNK), {
            stream: truncated,
          })
        : null;
      const url = URL.createObjectURL(new Blob([bytes], { type: meta.spec.mimeType }));
      objectUrl.current = url;
      setPreview({ meta, url, text, truncated });
    } catch (reason) {
      if (current === sequence.current) setError(formatLocalLearningError(reason));
    } finally {
      if (current === sequence.current) setBusy(false);
    }
  }
  if (!active) return null;
  return (
    <section
      className="local-artifacts"
      aria-label="本地执行产物"
      data-artifact-digest={scope.digest}
    >
      <button className="learning-secondary" disabled={busy} onClick={() => void loadList()}>
        {busy ? '正在读取产物…' : list ? '重新读取产物列表' : '查看执行产物'}
      </button>
      {error && (
        <p role="alert" className="learn-notice">
          {error}
        </p>
      )}
      {list && sameScope(list.scope, scope) && (
        <div>
          <p>
            执行当时保存的文件 · {new Date(list.createdAt * 1000).toLocaleString()}
            。只读本机快照，不重新执行或访问远端。
          </p>
          <p>历史产物可读不代表当前工程仍有效，也不代表实机或权威评测通过。</p>
          {!list.items.length && (
            <p role="status">此记录没有产物快照；不会从当前工程补造历史文件。</p>
          )}
          {list.items.map((meta) => (
            <div
              className="local-artifact-item"
              key={meta.spec.artifactId}
              data-artifact-id={meta.spec.artifactId}
            >
              <strong>{meta.spec.title}</strong>
              <code>{meta.spec.path}</code>
              <small>
                {meta.bytes.toLocaleString()} 字节 · {meta.spec.mimeType}
              </small>
              <button
                className="learning-secondary"
                disabled={busy}
                onClick={() => void open(meta)}
              >
                读取并核验：{meta.spec.title}
              </button>
            </div>
          ))}
          {preview && (
            <div className="local-artifact-preview">
              <p>已核验完整文件 SHA-256：</p>
              <code>{preview.meta.sha256}</code>
              {preview.text !== null && (
                <>
                  {preview.truncated && <p role="status">预览仅显示前 64 KiB，完整文件可下载。</p>}
                  <pre aria-label="产物文本预览">{preview.text || '（空文件）'}</pre>
                </>
              )}
              {preview.meta.spec.mimeType.startsWith('audio/') && (
                <audio
                  aria-label="产物音频预览"
                  controls
                  preload="metadata"
                  src={preview.url}
                  onError={() => setError('当前系统无法播放此格式，可下载文件查看。')}
                />
              )}
              {preview.meta.spec.mimeType.startsWith('video/') && (
                <video
                  aria-label="产物视频预览"
                  controls
                  preload="metadata"
                  src={preview.url}
                  onError={() => setError('当前系统无法播放此格式，可下载文件查看。')}
                />
              )}
              <a
                className="learning-secondary"
                href={preview.url}
                download={preview.meta.spec.path.split('/').pop()}
              >
                下载已核验文件
              </a>
              <small>文件可能含工程信息；由你决定是否分享。不会自动上传或附给 AI。</small>
            </div>
          )}
        </div>
      )}
    </section>
  );
}
