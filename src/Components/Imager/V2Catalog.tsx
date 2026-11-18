import { useCallback, useEffect, useRef, useState, type ReactNode, type RefObject } from 'react';
import { Star20Filled, DeveloperBoard20Regular } from '@fluentui/react-icons';
import { LynxIcon, type LynxIconName } from '../../CoreUI';
import type { WorkbenchDevice } from '../../Platform/IPC';
import type {
  FirmwareCandidate,
  FirmwareCatalogVersion,
} from '../../Platform/IPC/contracts/Workbench';
import { formatLinuxBytes } from '../Workbench/linuxWorkbenchModel';
import BasicCatalog from './BasicCatalog';
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
const vendorCN: Record<string, string> = {
  allwinner: '全志（Allwinner）',
  rockchip: '瑞芯微（Rockchip）',
  nxp: 'NXP',
  st: 'STMicroelectronics',
  kendryte: '嘉楠（Canaan）',
};
const rkModels = ['RK3588-EVB1', 'LubanCat-4 (RK3588S)', 'Rock 5B (RK3588)'];
const chipPackages: Record<string, [string, string]> = {
  f101: ['F101', 'ALLWINNER'],
  'nxp-imx': ['i.MX', 'NXP'],
  stm32mp1: ['STM32MP1', 'ST'],
  'canaan-k230': ['K230', 'CANAAN'],
};
const baseName = (path: string) => path.split(/[/\\]/).pop() || path;
const platform = (board: V2Board) => board.chip.split('·').pop()?.trim() || board.name;
const prettyRoute = (board: V2Board) => {
  if (board.route === 'mainline_spinor') return '主线系统 · SPI NOR';
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

export function V2BoardPicker({
  boards,
  selectedId,
  onSelect,
  rkBoard,
  onRkBoard,
}: {
  boards: readonly V2Board[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  rkBoard: string;
  onRkBoard: (name: string) => void;
}) {
  const [query, setQuery] = useState('');
  const [filter, setFilter] = useState('all');
  const searchInput = useRef<HTMLInputElement>(null);
  const focusSearch = useCallback((event: KeyboardEvent) => {
    if (
      event.defaultPrevented ||
      event.isComposing ||
      event.repeat ||
      !event.ctrlKey ||
      event.altKey ||
      event.shiftKey ||
      event.metaKey ||
      event.key.toLowerCase() !== 'k'
    )
      return;
    const input = searchInput.current;
    if (
      !input ||
      !input.getClientRects().length ||
      input.closest('[hidden], [inert], [aria-hidden="true"]')
    )
      return;
    if (
      event.target instanceof HTMLElement &&
      event.target.closest(
        'textarea, select, input:not([type="radio"]):not([type="checkbox"]), [contenteditable]:not([contenteditable="false"]), [role="textbox"]'
      )
    )
      return;
    const overlays = document.querySelectorAll(
      'dialog[open], [aria-modal="true"], [role="alertdialog"], .popup-overlay'
    );
    if (Array.from(overlays).some((el) => el.getClientRects().length)) return;
    event.preventDefault();
    input.focus();
  }, []);
  useEffect(() => {
    document.addEventListener('keydown', focusSearch);
    return () => document.removeEventListener('keydown', focusSearch);
  }, [focusSearch]);
  const selected = boards.find((b) => b.id === selectedId);
  const visible = boards.filter(
    (b) =>
      (filter === 'all' ||
        (filter === 'other'
          ? !['allwinner', 'rockchip'].includes(b.chipVendor)
          : filter === b.chipVendor)) &&
      [b.name, b.chip, b.storage, vendors[b.chipVendor], ...(b.id === 'rk3588' ? rkModels : [])]
        .join(' ')
        .toLowerCase()
        .includes(query.trim().toLowerCase())
  );
  return (
    <div className="v2c-layout v2c-board-layout">
      <div className="v2c-primary imager-board-picker">
        <header className="v2c-page-heading">
          <h2>
            <DeveloperBoard20Regular aria-hidden="true" />
            选择板子
          </h2>
          <p>选择开发板型号，系统将根据板型提供匹配的镜像、Loader 和烧录路线。</p>
        </header>
        <label className="v2c-search imager-board-search">
          {icon('search')}
          <input
            type="search"
            ref={searchInput}
            aria-label="搜索板型、芯片或厂商"
            aria-keyshortcuts="Control+K"
            placeholder="搜索板型：如 T113、F101、RK3588…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
          <kbd aria-hidden="true">Ctrl+K</kbd>
        </label>
        <div className="v2c-filters imager-board-filters" aria-label="芯片厂商">
          <strong>芯片厂商</strong>
          {(
            [
              ['all', '全部'],
              ['allwinner', 'Allwinner（全志）'],
              ['rockchip', 'Rockchip（瑞芯微）'],
              ['other', '其他'],
            ] as const
          ).map(([key, name]) => (
            <button
              type="button"
              aria-pressed={filter === key}
              key={key}
              onClick={() => setFilter(key)}
            >
              {key === 'allwinner' && (
                <i className="v2c-brandmark" aria-hidden="true">
                  AW
                </i>
              )}
              {key === 'rockchip' && (
                <i className="v2c-brandmark" aria-hidden="true">
                  RK
                </i>
              )}
              {key === 'other' && <DeveloperBoard20Regular aria-hidden="true" />}
              {name}
            </button>
          ))}
        </div>
        <h3 className="v2c-heading">
          <Star20Filled aria-hidden="true" />
          常用板型
        </h3>
        <div className="v2c-board-grid" role="radiogroup" aria-label="选择板型">
          {visible.map((board) => (
            <label
              key={board.id}
              className={'v2c-board-card' + (selectedId === board.id ? ' selected' : '')}
            >
              <input
                type="radio"
                name="basic-board"
                value={board.id}
                checked={selectedId === board.id}
                onChange={() => onSelect(board.id)}
              />
              <V2BoardArt id={board.id} name={board.name} />
              <div className="v2c-board-copy">
                <strong>{board.name}</strong>
                <span className="v2c-chip-vendor">
                  {vendors[board.chipVendor]} · {platform(board)}
                </span>
                <span className="v2c-tags">
                  {[board.mode, board.storage].map((tag) => (
                    <small key={tag}>{tag}</small>
                  ))}
                </span>
              </div>
            </label>
          ))}
        </div>
        {!visible.length && (
          <p className="v2c-empty" role="status">
            未找到匹配的板型。请更换关键词或芯片厂商。
          </p>
        )}
      </div>
      <DetailsPane title="当前任务" glyph="records">
        <div className="v2c-task-preview imager-board-detail">
          {selected ? (
            <V2BoardArt id={selected.id} name={selected.name} large />
          ) : (
            <img src={emptyChip} alt="" draggable={false} />
          )}
          <h3>
            {selected
              ? rkBoard && selected.id === 'rk3588'
                ? rkBoard
                : selected.name
              : '尚未选择板子'}
          </h3>
          <p>
            {selected ? '已选择板型，请核对下方配置。' : '请在板型列表选择需要烧录的开发板型号。'}
          </p>
        </div>
        {selected?.id === 'rk3588' && (
          <label className="v2c-select-model">
            具体板型
            <select
              aria-label="具体 RK 板型"
              value={rkBoard}
              onChange={(e) => onRkBoard(e.target.value)}
            >
              <option value="">请选择具体型号…</option>
              {rkModels.map((name) => (
                <option key={name}>{name}</option>
              ))}
            </select>
          </label>
        )}
        <Facts
          rows={[
            [
              '板型',
              selected
                ? selected.id === 'rk3588'
                  ? rkBoard || '待选择型号'
                  : selected.name
                : '未选择',
              'board',
            ],
            ['芯片厂商', selected ? vendors[selected.chipVendor] : '—', 'overview'],
            ['目标存储', selected?.storage || '—', 'database'],
            ['默认烧录路线', selected ? prettyRoute(selected) : '—', 'branch'],
          ]}
        />
      </DetailsPane>
    </div>
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
function SelectedBoard({ board, name }: { board: V2Board; name: string }) {
  return (
    <div className="v2c-board-summary">
      <V2BoardArt id={board.id} name={name} large />
      <div>
        <h3>{name}</h3>
        <p>厂商：{vendorCN[board.chipVendor]}</p>
        <p>平台：{platform(board)}</p>
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
          <>
            <ol className="v2c-progress" aria-label="当前烧录进度">
              {['选择板子', '选择系统', '设备连接', '烧写系统'].map((name, i) => (
                <li
                  key={name}
                  className={i === 0 ? 'done' : i === 1 ? 'current' : ''}
                  aria-current={i === 1 ? 'step' : undefined}
                >
                  <span>{i === 0 ? icon('check', 16) : i + 1}</span>
                  <div>
                    <b>{name}</b>
                    <small>
                      {[boardName, '选择镜像文件', '连接设备与模式', '执行写入与验证'][i]}
                    </small>
                  </div>
                </li>
              ))}
            </ol>
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
                  {prettyRoute(board)}
                </span>
              </div>
            </div>
          </>
        ) : (
          <header className="v2c-page-heading">
            <h2>
              {icon(source === 'recent' ? 'clock' : 'folder', 24)}
              {source === 'recent' ? '最近使用' : '本地镜像'}
            </h2>
            <p>
              {source === 'recent'
                ? '选择此前使用的本地文件，重新读取镜像指纹。'
                : '可以拖入镜像，或选择本地文件进行烧录。'}
            </p>
          </header>
        )}
        {tabs}
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
            <div className="v2c-drop" ref={dropRef}>
              <span className="v2c-upload-icon">{icon('upload', 24)}</span>
              <strong>拖入本地镜像文件到此处，或点击下方按钮选择文件</strong>
              <p>
                {board.route === 'mainline_spinor'
                  ? '选择与当前 F101 布局匹配的完整 SPI NOR 镜像。'
                  : '选择与当前板型和烧录路线匹配的系统镜像。'}
              </p>
              <button type="button" className="v2c-solid" onClick={onBrowse}>
                {icon('folder')}选择文件
              </button>
            </div>
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
            {imagePath && (
              <section className="v2c-file">
                <header>
                  {icon('document')}
                  <b title={imagePath}>{baseName(imagePath)}</b>
                  {identity && <small>{formatLinuxBytes(identity.sizeBytes)}</small>}
                  <button
                    type="button"
                    onClick={() => {
                      setTextPath('');
                      onImagePath('');
                    }}
                  >
                    {icon('dismiss', 16)}移除
                  </button>
                </header>
                <Facts
                  rows={[
                    ['文件路径', imagePath, 'folder'],
                    [
                      'SHA-256',
                      identity ? (
                        <CopyHash hash={identity.sha256} />
                      ) : fileError ? (
                        '未取得指纹'
                      ) : (
                        '正在读取…'
                      ),
                      'shield',
                    ],
                  ]}
                />
              </section>
            )}
            {fileError ? (
              <Notice kind="error" title="文件读取失败">
                {fileError}
              </Notice>
            ) : imagePath && identity ? (
              <div className="v2c-notice-row">
                <Notice kind="success" title="文件读取成功">
                  已读取镜像并计算 SHA-256。
                </Notice>
                <Notice kind="warning" title="板型适配待检查">
                  请在后续预检中核对镜像来源、板型和存储路线。
                </Notice>
              </div>
            ) : (
              reading && <p role="status">正在读取镜像文件…</p>
            )}
          </>
        )}
        {!inCatalog && (
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
      ) : (
        <DetailsPane title="当前板子" glyph="board">
          <SelectedBoard board={board} name={boardName} />
          <h3 className="v2c-heading">{icon('archive')}镜像详情</h3>
          <Facts
            rows={[
              ['文件路径', imagePath || '待选择', 'document'],
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
              ['SHA-256', identity ? <CopyHash hash={identity.sha256} /> : '—', 'shield'],
              ['来源', imagePath ? '本地文件' : '—', 'folder'],
            ]}
          />
          <Notice kind="info" title="说明">
            {imagePath
              ? '本地文件的板型适配仍需核对。请前往「设备连接」，完成目标设备与烧录参数的预检。'
              : '选择本地镜像后，将在这里显示路径、文件大小和 SHA-256 指纹。'}
          </Notice>
        </DetailsPane>
      )}
    </div>
  );
}
