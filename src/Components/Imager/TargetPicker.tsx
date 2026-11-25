import { EmmcErase } from './EmmcErase';
import { invokeCommand } from '../../Platform/IPC';
import { useCallback, useEffect, useRef, useState } from 'react';
import { LynxIcon } from '../../CoreUI';
import {
  manufacturers,
  mediaNames,
  models,
  searchModels,
  type TargetChoice,
} from './targetCatalog';
import './TargetPicker.css';

interface Props {
  active: boolean;
  choice: TargetChoice | null;
  onChoice: (next: TargetChoice | null) => void;
  boardName: string;
  onBoardName: (name: string) => void;
}
const mediaHints = ['串行 NOR 闪存', '串行 NAND 闪存', '板载多媒体存储', '可移除存储卡'];

const chipBrief: Record<string, string> = {
  RK3588: '八核 · Cortex-A76 / A55',
  RK3506: '三核 · Cortex-A7',
  RK3566: '四核 · Cortex-A55',
  RK3562: '四核 · Cortex-A53',
};
function preparation(vendor: string, medium: string): string[][] {
  switch (vendor) {
    case 'allwinner':
      return [
        ['厂商固件包', '使用适配当前板型的 IMAGEWTY 整包。'],
        [
          medium === 'SPI NAND' ? '主线 NAND 组件包' : '主线整盘镜像',
          '同时准备该板型配套的 RAM FES 引导文件。',
        ],
      ];
    case 'rockchip':
      return [
        ['厂商升级包', '使用板卡厂商提供的 update.img。'],
        ['主线整盘镜像', '准备 eMMC 整盘镜像和匹配的 Rockchip Loader。'],
      ];
    case 'nxp':
      return [
        ['UUU 脚本', '备齐脚本引用的文件，核对脚本选用的存储器。'],
        ['单板配置', '使用已保存的板级 UUU 配方。'],
      ];
    case 'st':
      return [
        ['DfuSe 容器', '核对镜像目标和设备枚举的 DFU 接口。'],
        ['RAW 二进制', '按板级布局确认目标区域和地址。'],
      ];
    default:
      return [
        ['KDImage 分区包', '核对分区表和目标介质。'],
        ['RAW 整盘镜像', '按板卡实际装配的存储器选择镜像。'],
      ];
  }
}

export function V2BoardPicker({ active, choice, onChoice, boardName, onBoardName }: Props) {
  const saved = models.find((m) => m.name === choice?.model);
  const [vendorId, setVendor] = useState(saved?.vendor ?? 'allwinner');
  const [family, setFamily] = useState(saved?.family ?? 'T');
  const [model, setModel] = useState(saved?.name ?? '');
  const [seriesOnly, setSeriesOnly] = useState(false);
  const [query, setQuery] = useState('');
  const input = useRef<HTMLInputElement>(null);
  const manual = useRef(false);
  useEffect(() => {
    if (!active || choice) return;
    let disposed = false;
    let timer: ReturnType<typeof setTimeout>;
    async function detect() {
      if (disposed || manual.current) return;
      try {
        if (document.visibilityState === 'visible') {
          const connected = await invokeCommand('imager_usb_targets', {});
          const detected =
            connected.length === 1 && manufacturers.find((v) => v.id === connected[0].vendor);
          if (!disposed && !manual.current && detected) {
            setVendor(detected.id);
            setFamily((current) =>
              current in detected.families ? current : Object.keys(detected.families)[0]
            );
          }
        }
      } catch {
        /* Manual target selection also works without USB access. */
      }
      if (!disposed && !manual.current) timer = setTimeout(detect, 2000);
    }
    void detect();
    return () => {
      disposed = true;
      clearTimeout(timer);
    };
  }, [active, choice]);
  const vendor = manufacturers.find((v) => v.id === vendorId)!;
  const searching = Boolean(query.trim());
  const matches = searching ? searchModels(query) : models;
  const list =
    searching && !seriesOnly
      ? matches
      : matches.filter((m) => m.vendor === vendorId && m.family === family);
  const entries = list.map((chip) => ({
    chip,
    media: Array.from(new Set([...(chip.profile?.media ?? []), 'SD'])).join(' / '),
    route: chip.profile?.routes ?? 'SD 整盘镜像',
  }));
  const first = entries[0];
  const sharedMedia = first && entries.every((entry) => entry.media === first.media) && first.media;
  const sharedRoute = first && entries.every((entry) => entry.route === first.route) && first.route;
  const part = models.find((m) => m.name === model);
  const change = (v: string, f: string, m = '') => {
    manual.current = true;
    if (v !== vendorId || f !== family || m !== model) onChoice(null);
    setQuery('');
    setVendor(v);
    setFamily(f);
    setModel(m);
  };
  const handle = useCallback((e: KeyboardEvent) => {
    if (
      !(e.ctrlKey || e.metaKey) ||
      e.altKey ||
      (e.ctrlKey && e.metaKey) ||
      e.shiftKey ||
      e.key.toLowerCase() !== 'k' ||
      e.isComposing ||
      e.defaultPrevented ||
      !input.current?.getClientRects().length
    )
      return;
    if (
      e.target instanceof HTMLElement &&
      e.target !== input.current &&
      e.target.closest('textarea, input, select, [contenteditable=true]')
    )
      return;
    if (
      Array.from(document.querySelectorAll('dialog[open], [aria-modal=true], .popup-overlay')).some(
        (el) => el.getClientRects().length
      )
    )
      return;
    e.preventDefault();
    setSeriesOnly(false);
    input.current.focus();
    input.current.select();
  }, []);
  useEffect(() => {
    document.addEventListener('keydown', handle);
    return () => document.removeEventListener('keydown', handle);
  }, [handle]);
  return (
    <div className="target-picker">
      <header className="target-heading">
        <h2>选择烧录目标</h2>
        <div className="v2c-search imager-board-search" role="search">
          <select
            aria-label="搜索范围"
            value={seriesOnly ? 'series' : 'all'}
            onChange={(event) => {
              manual.current = true;
              setSeriesOnly(event.target.value === 'series');
              input.current?.focus();
            }}
          >
            <option value="all">全部型号</option>
            <option value="series">当前系列</option>
          </select>
          <LynxIcon name="search" size={16} />
          <input
            ref={input}
            type="search"
            aria-label={seriesOnly ? '筛选当前系列型号' : '搜索芯片型号或板名'}
            aria-keyshortcuts="Control+K Meta+K"
            placeholder={seriesOnly ? '搜索 ' + family + ' 系列' : '搜索型号或板名'}
            value={query}
            onChange={(e) => {
              manual.current = true;
              setQuery(e.target.value);
              if (e.target.value && model) {
                onChoice(null);
                setModel('');
              }
            }}
          />
          <kbd>{/Mac/i.test(navigator.userAgent) ? '⌘ K' : 'Ctrl K'}</kbd>
        </div>
      </header>
      <div className="target-vendors" role="group" aria-label="主控厂商">
        {manufacturers.map((v) => (
          <button
            key={v.id}
            aria-label={v.name}
            aria-pressed={vendorId === v.id}
            onClick={() => change(v.id, Object.keys(v.families)[0])}
          >
            <strong>{v.name}</strong>
            <span>{v.brand}</span>
          </button>
        ))}
      </div>
      <div className="target-workspace">
        <aside className="target-families">
          <h3>
            <LynxIcon name="branch" size={16} />
            芯片系列
          </h3>
          {Object.entries(vendor.families).map(([f, names]) => (
            <button
              key={f}
              aria-label={f + ' 系列'}
              aria-pressed={family === f}
              onClick={() => change(vendorId, f)}
            >
              <span>{f} 系列</span>
              <small>{names.split(' ').length}</small>
            </button>
          ))}
          <details className="target-chip-help">
            <summary>
              <LynxIcon name="book" size={16} /> 如何识别主控？
            </summary>
            <p>查看主控芯片表面的丝印，或在板卡手册中查找 SoC 型号。</p>
          </details>
        </aside>
        <section className="target-main">
          {part && !query ? (
            <>
              <div className="target-subhead">
                <div>
                  <small>
                    {vendor.name} / {family} 系列
                  </small>
                  <h4>{part.name} · 目标存储</h4>
                </div>
                <button onClick={() => change(vendorId, family)}>返回型号列表</button>
              </div>
              <div className="target-media">
                {mediaNames.map((m, i) => (
                  <button
                    key={m}
                    data-medium={m}
                    disabled={m !== 'SD' && !part.profile?.media.includes(m)}
                    aria-pressed={choice?.medium === m}
                    onClick={() =>
                      onChoice({
                        model: part.name,
                        medium: m,
                        reader: m === 'SD' && !part.profile?.media.includes(m),
                      })
                    }
                  >
                    <LynxIcon
                      name={(['board', 'branch', 'database', 'archive'] as const)[i]}
                      size={20}
                    />
                    <span>
                      <strong>{m === 'SD' ? 'TF / SD' : m}</strong>
                      <small>{mediaHints[i]}</small>
                    </span>
                    <small>
                      {m !== 'SD' && !part.profile?.media.includes(m)
                        ? '未适配'
                        : choice?.medium === m
                          ? '已选'
                          : '可选'}
                    </small>
                  </button>
                ))}
              </div>
              {choice?.medium === 'SD' && part.profile?.media.includes('SD') && (
                <label>
                  连接方式{' '}
                  <select
                    value={choice.reader ? 'reader' : 'board'}
                    onChange={(e) => onChoice({ ...choice, reader: e.target.value === 'reader' })}
                  >
                    <option value="board">开发板下载口</option>
                    <option value="reader">电脑读卡器</option>
                  </select>
                </label>
              )}
              {choice?.reader ? (
                <p className="target-note">
                  通过电脑读卡器写入板卡的 SD 整盘镜像，无需连接开发板。
                </p>
              ) : choice && part.profile ? (
                <div className="target-prep">
                  <h4>{choice.medium} 可用的镜像类型</h4>
                  {preparation(part.vendor, choice.medium).map(([title, hint], index) => (
                    <div key={title}>
                      <LynxIcon name={index ? 'branch' : 'box'} size={16} />
                      <strong>{title}</strong>
                      <span>{hint}</span>
                    </div>
                  ))}
                </div>
              ) : (
                <p className="target-note">
                  <LynxIcon name="info" size={16} />
                  {part.profile
                    ? '选择介质后查看镜像和引导文件要求。'
                    : '此型号可使用电脑读卡器写入 TF / SD 卡。'}
                </p>
              )}
              {choice?.medium === 'eMMC' && part.vendor === 'rockchip' && (
                <EmmcErase model={part.name} />
              )}
              {choice && !choice.reader && part.vendor !== 'allwinner' && (
                <label className="v2c-select-model">
                  具体板型（选填）
                  {part.name === 'RK3576' ? (
                    <>
                      <input
                        aria-label="具体板型"
                        list="rk3576-boards"
                        value={boardName}
                        placeholder="输入板名或选择 DshanPI-A1"
                        onChange={(e) => onBoardName(e.target.value)}
                      />
                      <datalist id="rk3576-boards">
                        <option value="DshanPI-A1" />
                      </datalist>
                    </>
                  ) : part.vendor === 'rockchip' && !!part.profile?.aliases.length ? (
                    <select
                      aria-label="具体 RK 板型"
                      value={boardName}
                      onChange={(e) => onBoardName(e.target.value)}
                    >
                      <option value="">使用芯片型号 {part.name}</option>
                      {part.profile?.aliases.map((a) => (
                        <option key={a}>{a}</option>
                      ))}
                    </select>
                  ) : (
                    <input
                      aria-label="具体板型"
                      value={boardName}
                      placeholder={`默认使用 ${part.name}`}
                      onChange={(e) => onBoardName(e.target.value)}
                    />
                  )}
                </label>
              )}
            </>
          ) : (
            <>
              <div className="target-subhead target-list-toolbar">
                <div className="target-family-title">
                  <h4>{searching ? '搜索结果' : family + ' 系列'}</h4>
                  <span aria-live="polite">{list.length} 款型号</span>
                </div>
                {(sharedMedia || sharedRoute) && (
                  <dl className="target-series-spec">
                    {sharedMedia && (
                      <div>
                        <dt>存储</dt>
                        <dd className="target-model-media">{sharedMedia}</dd>
                      </div>
                    )}
                    {sharedRoute && (
                      <div>
                        <dt>镜像</dt>
                        <dd>{sharedRoute}</dd>
                      </div>
                    )}
                  </dl>
                )}
              </div>
              <div className="target-models">
                {entries.map(({ chip: m, media, route }) => (
                  <button
                    key={m.name}
                    data-model={m.name}
                    className={m.profile ? 'target-model-row' : 'target-model-row is-unavailable'}
                    onClick={() => change(m.vendor, m.family, m.name)}
                  >
                    <span className="target-model-name">
                      <strong>{m.name}</strong>
                      <small>
                        {searching && !seriesOnly
                          ? manufacturers.find((v) => v.id === m.vendor)?.name
                          : (m.profile?.aliases[0] ?? chipBrief[m.name])}
                      </small>
                    </span>
                    {(!sharedMedia || !sharedRoute) && (
                      <span className="target-model-spec">
                        {!sharedMedia && <span className="target-model-media">{media}</span>}
                        {!sharedRoute && <span>{route}</span>}
                      </span>
                    )}
                    <span className="target-row-arrow" aria-hidden="true">
                      ›
                    </span>
                  </button>
                ))}
              </div>
              {!list.length && <p role="status">未找到匹配型号，请核对丝印或尝试板名。</p>}
            </>
          )}
        </section>
      </div>
    </div>
  );
}
