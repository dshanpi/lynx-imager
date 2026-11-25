import { useEffect, useRef, useState, type ReactNode, type RefObject } from 'react';
import { LynxIcon, type LynxIconName } from '../../CoreUI';
import type { WorkbenchDevice } from '../../Platform/IPC';
import type {
  StagedFirmware,
  FirmwareCatalogVersion,
} from '../../Platform/IPC/contracts/Workbench';
import { formatLinuxBytes } from '../Workbench/linuxWorkbenchModel';
import BasicCatalog from './BasicCatalog';
import { AllwinnerRoute, SystemImageReview } from './SystemImageReview';
import { awRouteText } from './imageReviewModel';
import type { ImageReviewEvidence } from './imageReviewEvidence';
import { copyVerifiedText } from './imagerAIControls';
import type { BasicRoute } from './BasicPlan';
import './V2Catalog.css';

export interface V2Board {
  id: string;
  name: string;
  aliases?: readonly string[];
  chipVendor: WorkbenchDevice['chipVendor'];
  chip: string;
  storage: string;
  mode: string;
  route: BasicRoute;
}
export type V2ImageSource = 'catalog' | 'local' | 'recent';
type Identity = { sizeBytes: number; sha256: string };
const baseName = (path: string) => path.split(/[/\\]/).pop() || path;
function icon(name: LynxIconName, size: 16 | 20 | 24 = 20) {
  return <LynxIcon name={name} size={size} aria-hidden="true" />;
}

function DetailsPane({
  title,
  glyph,
  children,
}: {
  title: string;
  glyph: LynxIconName;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDetailsElement>(null);
  useEffect(() => {
    const media = window.matchMedia('(max-width: 1050px)');
    const update = () => {
      if (ref.current) ref.current.open = !media.matches;
    };
    update();
    media.addEventListener('change', update);
    return () => media.removeEventListener('change', update);
  }, []);
  return (
    <details ref={ref} open className="v2c-detail">
      <summary>
        {icon(glyph)}
        {title}
        {icon('chevron-down', 16)}
      </summary>
      <div className="v2c-detail-content">{children}</div>
    </details>
  );
}
function Facts({ rows }: { rows: Array<[string, ReactNode, LynxIconName?]> }) {
  return (
    <dl className="v2c-facts">
      {rows.map(([name, value, glyph]) => (
        <div key={name}>
          <dt>
            {glyph && icon(glyph, 16)}
            {name}
          </dt>
          <dd>{value}</dd>
        </div>
      ))}
    </dl>
  );
}

function CopyHash({ hash }: { hash: string }) {
  const [message, setMessage] = useState('');
  return (
    <span className="v2c-hash">
      <code title={hash}>{hash}</code>
      <button
        type="button"
        aria-label="复制 SHA-256"
        title="复制 SHA-256"
        onClick={() =>
          void copyVerifiedText(Promise.resolve(hash)).then(
            () => setMessage('已复制'),
            () => setMessage('无法复制，请选择摘要手动复制。')
          )
        }
      >
        {icon('copy', 16)}
      </button>
      {message && <small role="status">{message}</small>}
    </span>
  );
}
function Notice({
  kind,
  title,
  children,
}: {
  kind: 'info' | 'warning' | 'success' | 'error';
  title: string;
  children: ReactNode;
}) {
  return (
    <div role={kind === 'error' ? 'alert' : 'status'} className={'v2c-notice ' + kind}>
      {icon(
        kind === 'info'
          ? 'info'
          : kind === 'success'
            ? 'check-circle'
            : kind === 'error'
              ? 'error'
              : 'warning'
      )}
      <div>
        <strong>{title}</strong>
        <p>{children}</p>
      </div>
    </div>
  );
}
type Recent = {
  boardId: string;
  boardName: string;
  path: string;
  sizeBytes: number;
  scope?: string;
  loaderPath?: string;
};
const recentKey = 'lynx-imager.basic.local-files.v2';
function readRecent(): Recent[] {
  try {
    const items: unknown = JSON.parse(localStorage.getItem(recentKey) || '[]');
    if (!Array.isArray(items)) return [];
    return items
      .filter(
        (i): i is Recent =>
          !!i &&
          typeof i.path === 'string' &&
          typeof i.boardId === 'string' &&
          typeof i.boardName === 'string' &&
          Number.isSafeInteger(i.sizeBytes) &&
          i.sizeBytes > 0
      )
      .slice(0, 10);
  } catch {
    return [];
  }
}
function writeRecent(items: Recent[]) {
  try {
    localStorage.setItem(recentKey, JSON.stringify(items));
  } catch {
    /* History is optional in restricted profiles. */
  }
}
export function imagePickerDirectory(currentPath: string) {
  const path = currentPath || readRecent()[0]?.path || '';
  const end = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
  return end >= 0 ? path.slice(0, end + 1) : undefined;
}
function RecentFiles({
  items,
  choose,
  clear,
}: {
  items: Recent[];
  choose: (item: Recent) => void;
  clear: () => void;
}) {
  return (
    <section className="v2c-recent">
      <header>
        <h3>{icon('clock', 16)}最近使用</h3>
        <button type="button" disabled={!items.length} onClick={clear}>
          {icon('delete', 16)}清空记录
        </button>
      </header>
      {items.length ? (
        items.map((r) => (
          <button
            type="button"
            key={r.path}
            onClick={() => choose(r)}
            className="v2c-recent-row"
            title={r.path}
          >
            {icon('document', 16)}
            <b>{baseName(r.path)}</b>
            <span>{formatLinuxBytes(r.sizeBytes)}</span>
            <span>{r.path}</span>
            {icon('folder', 16)}
          </button>
        ))
      ) : (
        <p>暂无此板型的本地镜像记录。</p>
      )}
    </section>
  );
}
export interface V2SystemPickerProps {
  memoryScope: string;
  onRestore: (image: string, loader: string) => void;
  board: V2Board;
  boardName: string;
  source: V2ImageSource;
  onSource: (v: V2ImageSource) => void;
  imagePath: string;
  identity: Identity | null;
  onBrowse: () => void;
  onImagePath: (path: string) => void;
  dropRef: RefObject<HTMLDivElement | null>;
  onCandidate: (candidate: StagedFirmware) => void;
  candidate: StagedFirmware | null;
  fileError?: string;
  reading?: boolean;
  route: BasicRoute;
  onRoute: (v: BasicRoute) => void;
  onUseImageAsLoader: (nextRoute: BasicRoute) => void;
  loaderPath: string;
  loaderIdentity: Identity | null;
  loaderError?: string;
  onChooseLoader: () => void;
  onReviewReady: (ready: boolean, evidence?: ImageReviewEvidence) => void;
  vendorControls?: ReactNode;
  vendorTitle?: string;
}
export function V2SystemPicker(props: V2SystemPickerProps) {
  const {
    board,
    boardName,
    source,
    onSource,
    imagePath,
    identity,
    onBrowse,
    onImagePath,
    dropRef,
    onCandidate,
    candidate,
    fileError,
    reading,
  } = props;
  const [recent, setRecent] = useState<Recent[]>(readRecent);
  const [inspected, setInspected] = useState<FirmwareCatalogVersion | null>(null);
  const [textPath, setTextPath] = useState('');
  const recorded = useRef('');
  const restoredScope = useRef('');
  const { memoryScope, onRestore } = props;
  useEffect(() => {
    if (source !== 'local') {
      restoredScope.current = '';
      return;
    }
    if (restoredScope.current === memoryScope) return;
    restoredScope.current = memoryScope;
    if (imagePath || props.loaderPath) return;
    const last = readRecent().find((item) => item.scope === memoryScope);
    if (last) onRestore(last.path, typeof last.loaderPath === 'string' ? last.loaderPath : '');
  }, [memoryScope, source, imagePath, props.loaderPath, onRestore]);
  const inCatalog = source === 'catalog';
  useEffect(() => setInspected(null), [board.id, props.route, source]);
  const visibleRecent = recent.filter(
    (r) =>
      r.boardId === board.id && r.boardName === boardName && (!r.scope || r.scope === memoryScope)
  );
  useEffect(() => {
    if (source !== 'local' || !identity || !imagePath) return;
    if (props.loaderPath && !props.loaderIdentity && !props.loaderError) return;
    const loaderPath = props.loaderIdentity ? props.loaderPath : undefined;
    const key = JSON.stringify([memoryScope, imagePath, identity.sha256, loaderPath]);
    if (recorded.current === key) return;
    recorded.current = key;
    const next = [
      {
        boardId: board.id,
        boardName,
        path: imagePath,
        sizeBytes: identity.sizeBytes,
        scope: memoryScope,
        loaderPath,
      },
      ...readRecent().filter(
        (r) =>
          !(
            r.boardId === board.id &&
            r.boardName === boardName &&
            r.path === imagePath &&
            (!r.scope || r.scope === memoryScope)
          )
      ),
    ].slice(0, 10);
    writeRecent(next);
    setRecent(next);
  }, [
    board.id,
    boardName,
    memoryScope,
    source,
    imagePath,
    identity,
    props.loaderIdentity,
    props.loaderPath,
    props.loaderError,
  ]);
  function clearRecent() {
    recorded.current = '';
    const next = readRecent().filter(
      (r) =>
        r.boardId !== board.id || r.boardName !== boardName || (r.scope && r.scope !== memoryScope)
    );
    writeRecent(next);
    setRecent(next);
  }
  function chooseRecent(item: Recent) {
    onRestore(
      item.path,
      item.scope === memoryScope && typeof item.loaderPath === 'string' ? item.loaderPath : ''
    );
  }
  const manifest = inspected?.entry.manifest || candidate?.manifest;
  const hasCatalog = board.id === 'f101' && props.route === 'mainline_spinor';
  const tabs = (
    <div className="v2c-tabs" role="group" aria-label="镜像来源">
      {(
        [
          ['local', 'folder', '本地文件'],
          ['catalog', 'cloud', '板型资源'],
          ['recent', 'clock', '最近使用'],
        ] as const
      )
        .filter(([key]) => key !== 'catalog' || hasCatalog)
        .map(([key, glyph, name]) => (
          <button
            type="button"
            aria-pressed={source === key}
            key={key}
            onClick={() => onSource(key)}
          >
            {icon(glyph)}
            {name}
            {key === 'recent' && visibleRecent.length > 0 && <small>{visibleRecent.length}</small>}
          </button>
        ))}
    </div>
  );
  const isAw = board.chipVendor === 'allwinner';
  const imageRole = isAw ? awRouteText[props.route]?.[1] || '系统镜像' : '系统镜像';
  return (
    <div className="v2c-layout v2c-system-layout image-desk">
      <header className="image-desk-title">
        <h2>选择系统镜像</h2>
        <div className="image-target-summary" aria-label="烧录目标">
          {icon('board', 16)}
          <strong>{boardName}</strong>
          <span>{board.storage}</span>
          <span>{board.mode}</span>
        </div>
        {board.id === 'f101' && (
          <p>
            YuzukiNeko · F101S3 参考配置 · 玄铁 C907 · RISC-V · 16 MiB PSRAM · 16 MiB SPI NOR ·
            USB-C · FEL
          </p>
        )}
      </header>
      {isAw && <AllwinnerRoute {...props} />}
      <div className="v2c-primary">
        <header className="image-source-bar">
          <h3>{props.vendorTitle || '烧录文件'}</h3>
          {tabs}
        </header>
        {source === 'local' && props.vendorControls}
        {inCatalog && (
          <BasicCatalog
            key={board.id + ':' + props.route}
            onChoose={onCandidate}
            onInspect={setInspected}
            selected={candidate}
            onLocal={onBrowse}
          />
        )}
        {source === 'local' && (
          <>
            {!imagePath && !props.vendorControls && (
              <div className="v2c-drop imager-empty-files" ref={dropRef}>
                <span className="image-desk-file-icon">{icon('document', 24)}</span>
                <div className="image-desk-file-copy">
                  <strong>{imageRole}</strong>
                  <small>支持拖入本地镜像文件。</small>
                </div>
                <span className="image-desk-destination">
                  {icon('database', 16)}写入 {board.storage}
                </span>
                <button type="button" className="v2c-solid" onClick={onBrowse}>
                  {icon('folder')}选择镜像…
                </button>
              </div>
            )}
            {fileError ? (
              <Notice kind="error" title="文件读取失败">
                {fileError}
              </Notice>
            ) : (
              reading && <p role="status">正在读取镜像文件…</p>
            )}
          </>
        )}
        {(source !== 'catalog' || candidate) && (
          <SystemImageReview
            key={JSON.stringify([
              board.id,
              boardName,
              props.route,
              inCatalog,
              imagePath,
              candidate?.indexSha256,
            ])}
            {...props}
          />
        )}
        {source === 'local' && !imagePath && !props.vendorControls && (
          <details className="v2c-path-entry">
            <summary>输入文件路径</summary>
            <form
              onSubmit={(e) => {
                e.preventDefault();
                if (textPath.trim()) onImagePath(textPath.trim());
              }}
            >
              <label className="v2c-visually-hidden" htmlFor="basic-image">
                镜像文件路径
              </label>
              <input
                id="basic-image"
                value={textPath}
                onChange={(e) => setTextPath(e.target.value)}
                placeholder="粘贴镜像的完整路径…"
                spellCheck={false}
              />
              <button type="submit" disabled={!textPath.trim()}>
                读取文件
              </button>
            </form>
          </details>
        )}
        {source === 'local' && imagePath && (
          <details className="imager-file-details">
            <summary>文件详情</summary>
            <Facts
              rows={[
                ['系统镜像路径', imagePath, 'document'],
                [
                  '文件大小',
                  identity
                    ? formatLinuxBytes(identity.sizeBytes) +
                      '（' +
                      identity.sizeBytes.toLocaleString() +
                      ' 字节）'
                    : '—',
                  'database',
                ],
                ['镜像 SHA-256', identity ? <CopyHash hash={identity.sha256} /> : '—', 'shield'],
                ...(props.loaderPath
                  ? ([
                      ['引导来源路径', props.loaderPath, 'folder'],
                      [
                        '引导包 SHA-256',
                        props.loaderIdentity ? (
                          <CopyHash hash={props.loaderIdentity.sha256} />
                        ) : (
                          '—'
                        ),
                        'shield',
                      ],
                    ] as Array<[string, ReactNode, LynxIconName?]>)
                  : []),
              ]}
            />
          </details>
        )}
        {source === 'recent' && (
          <RecentFiles items={visibleRecent} choose={chooseRecent} clear={clearRecent} />
        )}
        {inCatalog && manifest && (
          <div className="image-manifest">
            <DetailsPane title="镜像信息" glyph="document">
              <div className="v2c-image-identity">
                <span>{icon('archive', 24)}</span>
                <div>
                  <strong>
                    {manifest.applicationId} {manifest.version}
                  </strong>
                  <small>{manifest.image.fileName}</small>
                </div>
              </div>
              <Facts
                rows={[
                  ['格式', manifest.image.fileName.split('.').pop()?.toUpperCase()],
                  ['大小', formatLinuxBytes(manifest.image.sizeBytes)],
                  ['适用板型', manifest.boardModel],
                  ['平台', manifest.chip],
                  ['系统版本', manifest.version],
                  ['烧录路线', manifest.route],
                ]}
              />
              <Notice kind="info" title="清单声明的适用板型">
                {manifest.boardModel} · 真正写入前仍需预检目标。
              </Notice>
            </DetailsPane>
            <DetailsPane title="镜像校验" glyph="shield">
              <Facts rows={[['SHA-256', <CopyHash key="sha256" hash={manifest.image.sha256} />]]} />
              {candidate &&
              identity &&
              manifest.image.sha256 === identity.sha256 &&
              candidate.manifest.image.sha256 === identity.sha256 ? (
                <Notice kind="success" title="校验通过">
                  下载的镜像与发布清单的指纹一致。
                </Notice>
              ) : (
                <p className="v2c-hint">获取镜像后核对文件指纹。</p>
              )}
            </DetailsPane>
          </div>
        )}
      </div>
    </div>
  );
}
