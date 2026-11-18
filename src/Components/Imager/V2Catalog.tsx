import { useEffect, useRef, useState, type ReactNode, type RefObject } from 'react';
import { LynxIcon, type LynxIconName } from '../../CoreUI';
import type { WorkbenchDevice } from '../../Platform/IPC';
import type {
  FirmwareCandidate,
  FirmwareCatalogVersion,
} from '../../Platform/IPC/contracts/Workbench';
import { formatLinuxBytes } from '../Workbench/linuxWorkbenchModel';
import BasicCatalog from './BasicCatalog';
import { AllwinnerRoute, SystemImageReview } from './SystemImageReview';
import { awRouteText } from './imageReviewModel';
import type { ImageReviewEvidence } from './imageReviewEvidence';
import { copyVerifiedText } from './imagerAIControls';
import type { BasicRoute } from './BasicPlan';
import t113Art from './v2-art/t113-preview.png?url';
import t113Detail from './v2-art/t113-detail.png?url';
import rk3588Art from './v2-art/rk3588-preview.png?url';
import emptyChip from './v2-art/chip-empty.png?url';
import './V2Catalog.css';

export interface V2Board {
  id: string;
  name: string;
  chipVendor: WorkbenchDevice['chipVendor'];
  chip: string;
  storage: string;
  mode: string;
  route: BasicRoute;
}
export type V2ImageSource = 'catalog' | 'local' | 'recent';
type Identity = { sizeBytes: number; sha256: string };
const vendors: Record<string, string> = {
  allwinner: 'Allwinner',
  rockchip: 'Rockchip',
  nxp: 'NXP',
  st: 'STMicroelectronics',
  kendryte: 'Canaan',
};
const chipPackages: Record<string, [string, string]> = {
  f101: ['F101', 'ALLWINNER'],
  't153-emmc': ['T153', 'ALLWINNER'],
  'nxp-imx': ['i.MX', 'NXP'],
  stm32mp1: ['STM32MP1', 'ST'],
  'canaan-k230': ['K230', 'CANAAN'],
};
const baseName = (path: string) => path.split(/[/\\]/).pop() || path;
const prettyRoute = (board: V2Board) => {
  if (board.route === 'mainline_spinor') return '主线系统 · SPI NOR';
  if (board.route === 'mainline_emmc') return '主线系统 · eMMC';
  if (board.route === 'mainline_nand') return '主线系统 · SPI NAND';
  if (board.route === 'rockchip_raw_emmc') return '主线系统 · eMMC';
  return '厂商系统镜像';
};
function icon(name: LynxIconName, size: 16 | 20 | 24 = 20) {
  return <LynxIcon name={name} size={size} aria-hidden="true" />;
}

export function V2BoardArt({
  id,
  name,
  large = false,
}: {
  id: string;
  name?: string;
  large?: boolean;
}) {
  const chip = chipPackages[id];
  if (chip) {
    return (
      <span
        role="img"
        aria-label={chip.join(' · ') + ' 芯片封装示意图'}
        className={'v2c-board-art v2c-chip-art' + (large ? ' v2c-art-large' : '')}
      >
        <span className={'v2c-chip-package pkg-' + id}>
          <small>{chip[1]}</small>
          <b>{chip[0]}</b>
        </span>
      </span>
    );
  }
  const src =
    id === 't113-pro' ? (large ? t113Detail : t113Art) : id === 'rk3588' ? rk3588Art : emptyChip;
  return (
    <img
      className={'v2c-board-art' + (large ? ' v2c-art-large' : '')}
      src={src}
      alt={id === 't113-pro' || id === 'rk3588' ? (name || id) + ' 示意图' : '芯片平台示意图'}
      draggable={false}
    />
  );
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
type Recent = { boardId: string; boardName: string; path: string; sizeBytes: number };
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
function RecentFiles({
  items,
  choose,
  clear,
}: {
  items: Recent[];
  choose: (v: string) => void;
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
            onClick={() => choose(r.path)}
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
  board: V2Board;
  boardName: string;
  source: V2ImageSource;
  onSource: (v: V2ImageSource) => void;
  imagePath: string;
  identity: Identity | null;
  onBrowse: () => void;
  onImagePath: (path: string) => void;
  dropRef: RefObject<HTMLDivElement | null>;
  catalogDevice?: WorkbenchDevice | null;
  devices: WorkbenchDevice[];
  onCatalogDevice: (id: number | null) => void;
  onCandidate: (candidate: FirmwareCandidate) => void;
  candidate: FirmwareCandidate | null;
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
    catalogDevice,
    devices,
    onCatalogDevice,
    onCandidate,
    candidate,
    fileError,
    reading,
  } = props;
  const [recent, setRecent] = useState<Recent[]>(readRecent);
  const [inspected, setInspected] = useState<FirmwareCatalogVersion | null>(null);
  const [textPath, setTextPath] = useState('');
  const recorded = useRef('');
  const inCatalog = source === 'catalog';
  const visibleRecent = recent.filter((r) => r.boardId === board.id && r.boardName === boardName);
  useEffect(() => {
    if (source !== 'local' || !identity || !imagePath) return;
    const key = JSON.stringify([board.id, boardName, imagePath, identity.sha256]);
    if (recorded.current === key) return;
    recorded.current = key;
    const next = [
      { boardId: board.id, boardName, path: imagePath, sizeBytes: identity.sizeBytes },
      ...readRecent().filter(
        (r) => !(r.boardId === board.id && r.boardName === boardName && r.path === imagePath)
      ),
    ].slice(0, 10);
    writeRecent(next);
    setRecent(next);
  }, [board.id, boardName, source, imagePath, identity]);
  function clearRecent() {
    const next = readRecent().filter((r) => r.boardId !== board.id || r.boardName !== boardName);
    writeRecent(next);
    setRecent(next);
  }
  function chooseRecent(path: string) {
    onSource('local');
    onImagePath(path);
  }
  const manifest = inspected?.entry.manifest || candidate?.manifest;
  const tabs = (
    <div className={'v2c-tabs' + (inCatalog ? ' is-catalog' : '')} aria-label="镜像来源">
      {(
        [
          ['catalog', 'cloud', '板型资源'],
          ['local', 'folder', '本地文件'],
          ['recent', 'clock', '最近使用'],
        ] as const
      ).map(([key, glyph, name]) => (
        <button type="button" aria-pressed={source === key} key={key} onClick={() => onSource(key)}>
          {icon(glyph)}
          {name}
          {key === 'recent' && visibleRecent.length > 0 && <small>{visibleRecent.length}</small>}
        </button>
      ))}
    </div>
  );
  return (
    <div className={'v2c-layout v2c-system-layout ' + (inCatalog ? 'catalog' : 'local')}>
      <div className="v2c-primary">
        {inCatalog ? (
          <div className="v2c-board-banner">
            <V2BoardArt id={board.id} name={boardName} large />
            <div>
              <h2>
                {boardName} <small>{vendors[board.chipVendor]}</small>
              </h2>
              <p>
                {board.chip} · {board.storage}
              </p>
              <span>
                {icon('board', 16)}
                {board.mode}
              </span>
              <span>
                {icon('branch', 16)}
                {board.chipVendor === 'allwinner'
                  ? awRouteText[props.route]?.[0]
                  : prettyRoute(board)}
              </span>
            </div>
          </div>
        ) : (
          <header className="v2c-page-heading">
            <h2>
              {icon(source === 'recent' ? 'clock' : 'folder', 24)}
              {source === 'recent'
                ? '最近使用'
                : board.chipVendor === 'allwinner' && props.route !== 'vendor'
                  ? '选择系统与引导文件'
                  : props.vendorTitle || '选择系统镜像'}
            </h2>
            <p>
              {source === 'recent'
                ? '选择此前使用的本地文件，重新读取镜像指纹。'
                : boardName + ' · ' + board.storage + '，请依据板型配套资料选择文件。'}
            </p>
          </header>
        )}
        {tabs}
        {board.chipVendor === 'allwinner' && <AllwinnerRoute {...props} />}
        {source === 'local' && props.vendorControls}
        {inCatalog && (
          <>
            {board.id === 'f101' && (
              <label className="v2c-device-select">
                资源对应的单板
                <select
                  aria-label="资源对应的单板"
                  value={catalogDevice?.id ?? ''}
                  onChange={(e) => {
                    setInspected(null);
                    onCatalogDevice(e.target.value ? Number(e.target.value) : null);
                  }}
                >
                  <option value="">选择已保存的 F101 单板…</option>
                  {devices.map((d) => (
                    <option key={d.id} value={d.id ?? ''}>
                      {d.name || d.boardUid}
                    </option>
                  ))}
                </select>
              </label>
            )}
            <BasicCatalog
              key={board.id + ':' + (catalogDevice?.id ?? '')}
              device={board.id === 'f101' ? catalogDevice : null}
              hasCatalog={board.id === 'f101'}
              onChoose={onCandidate}
              onInspect={setInspected}
              selected={candidate}
              onLocal={() => onSource('local')}
            />
          </>
        )}
        {source === 'local' && (
          <>
            {!imagePath && (
              <div className="v2c-drop imager-empty-files" ref={dropRef}>
                <span className="v2c-upload-icon">{icon('upload', 24)}</span>
                <strong>
                  {props.vendorControls ? '也可将本地文件拖入此处' : '将系统镜像拖到这里'}
                </strong>
                {!props.vendorControls && (
                  <button type="button" className="v2c-solid" onClick={onBrowse}>
                    {icon('folder')}选择文件
                  </button>
                )}
              </div>
            )}
            {!imagePath && (
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
            {fileError ? (
              <Notice kind="error" title="文件读取失败">
                {fileError}
              </Notice>
            ) : (
              reading && <p role="status">正在读取镜像文件…</p>
            )}
          </>
        )}
        <SystemImageReview
          key={JSON.stringify([board.id, boardName, props.route, source, imagePath, candidate?.id])}
          {...props}
        />
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
      </div>
      {inCatalog ? (
        <div className="v2c-detail-stack">
          <DetailsPane title="镜像信息" glyph="document">
            <div className="v2c-image-identity">
              <span>{icon('archive', 24)}</span>
              <div>
                <strong>
                  {manifest ? manifest.applicationId + ' ' + manifest.version : '尚未选择资源'}
                </strong>
                <small>{manifest?.image.fileName ?? '选择当前板型的资源或本地文件。'}</small>
              </div>
            </div>
            <Facts
              rows={[
                ['格式', manifest?.image.fileName.split('.').pop()?.toUpperCase() ?? '—'],
                ['大小', manifest ? formatLinuxBytes(manifest.image.sizeBytes) : '—'],
                ['适用板型', manifest?.boardModel ?? '—'],
                ['平台', manifest?.chip ?? '—'],
                ['系统版本', manifest?.version ?? '—'],
                ['烧录路线', manifest?.route ?? '—'],
              ]}
            />
            <Notice kind="info" title={manifest ? '清单声明的适用板型' : '板型适配待检查'}>
              {manifest
                ? manifest.boardModel + ' · 真正写入前仍需预检目标。'
                : '选择资源后查看发布清单与板型信息。'}
            </Notice>
          </DetailsPane>
          <DetailsPane title="镜像校验" glyph="shield">
            <Facts
              rows={[['SHA-256', manifest ? <CopyHash hash={manifest.image.sha256} /> : '—']]}
            />
            {candidate &&
            identity &&
            manifest?.image.sha256 === identity.sha256 &&
            candidate.manifest.image.sha256 === identity.sha256 ? (
              <Notice kind="success" title="校验通过">
                下载的镜像与发布清单的指纹一致。
              </Notice>
            ) : (
              <p className="v2c-hint">获取镜像后核对文件指纹。</p>
            )}
          </DetailsPane>
        </div>
      ) : null}
    </div>
  );
}
