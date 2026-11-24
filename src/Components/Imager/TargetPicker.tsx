import { useCallback, useEffect, useRef, useState } from 'react';
import { LynxIcon } from '../../CoreUI';
import { V2BoardArt } from './V2Catalog';
import {
  manufacturers,
  mediaNames,
  models,
  searchModels,
  type TargetChoice,
} from './targetCatalog';
import './TargetPicker.css';

interface Props {
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

export function V2BoardPicker({ choice, onChoice, boardName, onBoardName }: Props) {
  const saved = models.find((m) => m.name === choice?.model);
  const [vendorId, setVendor] = useState(saved?.vendor ?? 'allwinner');
  const [family, setFamily] = useState(saved?.family ?? 'T');
  const [model, setModel] = useState(saved?.name ?? '');
  const [familyQuery, setFamilyQuery] = useState('');
  const [query, setQuery] = useState('');
  const input = useRef<HTMLInputElement>(null);
  const vendor = manufacturers.find((v) => v.id === vendorId)!;
  const list = query.trim()
    ? searchModels(query)
    : models.filter(
        (m) =>
          m.vendor === vendorId &&
          m.family === family &&
          m.name.toLowerCase().includes(familyQuery.trim().toLowerCase())
      );
  const part = models.find((m) => m.name === model);
  const hero = part ?? list.find((m) => m.profile);
  const change = (v: string, f: string, m = '') => {
    if (v !== vendorId || f !== family || m !== model) onChoice(null);
    setQuery('');
    setFamilyQuery('');
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
    input.current.focus();
  }, []);
  useEffect(() => {
    document.addEventListener('keydown', handle);
    return () => document.removeEventListener('keydown', handle);
  }, [handle]);
  return (
    <div className="target-picker">
      <header className="target-heading">
        <div className="target-vendors" aria-label="主控厂商">
          <span className="target-vendor-label">
            <LynxIcon name="board" size={16} /> 主控厂商
          </span>
          {manufacturers.map((v) => (
            <button
              key={v.id}
              aria-label={v.name}
              aria-pressed={vendorId === v.id}
              onClick={() => change(v.id, Object.keys(v.families)[0])}
            >
              <i aria-hidden="true">{v.mark}</i>
              <span>{v.name}</span>
            </button>
          ))}
        </div>
        <label className="v2c-search imager-board-search">
          <LynxIcon name="search" size={16} />
          <input
            ref={input}
            type="search"
            aria-label="搜索芯片型号或板名"
            aria-keyshortcuts="Control+K Meta+K"
            placeholder="搜索芯片型号或板名"
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              if (e.target.value && model) {
                onChoice(null);
                setModel('');
              }
            }}
          />
          <kbd>{/Mac/i.test(navigator.userAgent) ? '⌘ K' : 'Ctrl K'}</kbd>
        </label>
      </header>
      <div className="target-workspace">
        <aside className="target-families">
          <h3>
            <LynxIcon name="branch" size={16} /> 芯片系列
          </h3>
          {Object.entries(vendor.families).map(([f, names]) => (
            <button
              key={f}
              aria-label={f + ' 系列'}
              aria-pressed={family === f}
              onClick={() => change(vendorId, f)}
            >
              {f} 系列 <small>{names.split(' ').length}</small>
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
          <header className="target-hero">
            <div>
              <small>
                {query
                  ? '全部厂商'
                  : vendor.name + ' ' + vendor.brand + (part ? ' / ' + family + ' 系列' : '')}
              </small>
              <h3>{query ? '搜索结果' : (part?.name ?? family + ' 系列')}</h3>
              <p>
                {query
                  ? '可按芯片丝印或开发板名称定位。'
                  : part && vendor.id === 'allwinner'
                    ? '按板上实际安装的存储器选择目标介质。'
                    : vendor.tip}
              </p>
            </div>
            {!query && hero?.profile && (
              <figure>
                <V2BoardArt id={hero.profile.preset} name={hero.name} />
                <figcaption>
                  {hero.name}{' '}
                  {hero.profile.preset === 'rk3588' || hero.profile.preset === 't113-pro'
                    ? '板卡示例'
                    : '芯片示意'}
                </figcaption>
              </figure>
            )}
          </header>
          {part && !query ? (
            <>
              <div className="target-subhead">
                <h4>目标存储</h4>
                <button onClick={() => change(vendorId, family)}>返回型号列表</button>
              </div>
              <div className="target-media">
                {mediaNames.map((m, i) => (
                  <button
                    key={m}
                    data-medium={m}
                    disabled={!part.profile?.media.includes(m)}
                    aria-pressed={choice?.medium === m}
                    onClick={() => onChoice({ model: part.name, medium: m })}
                  >
                    <LynxIcon
                      name={(['board', 'branch', 'database', 'archive'] as const)[i]}
                      size={20}
                    />
                    <span>
                      <strong>{m}</strong>
                      <small>{mediaHints[i]}</small>
                    </span>
                    <small>
                      {!part.profile?.media.includes(m)
                        ? '未适配'
                        : choice?.medium === m
                          ? '已选'
                          : '可选'}
                    </small>
                  </button>
                ))}
              </div>
              {choice && part.profile ? (
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
                    : '此型号已收录，尚无可执行的存储配方。'}
                </p>
              )}
              {choice && part.vendor !== 'allwinner' && (
                <label className="v2c-select-model">
                  具体板型（选填）
                  {part.vendor === 'rockchip' && !!part.profile?.aliases.length ? (
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
              <div className="target-subhead">
                <h4>{query ? '匹配的芯片' : '芯片型号'}</h4>
                <small>{list.length} 款型号</small>
              </div>
              {!query && (
                <label className="target-series-search">
                  <LynxIcon name="search" size={16} />
                  <input
                    type="search"
                    aria-label="筛选当前系列型号"
                    placeholder={'搜索 ' + family + ' 系列型号'}
                    value={familyQuery}
                    onChange={(e) => setFamilyQuery(e.target.value)}
                  />
                </label>
              )}
              <div className="target-columns">
                <span>芯片型号</span>
                <span>烧录路线</span>
                <span>目标存储</span>
              </div>
              <div className="target-models">
                {list.map((m) => (
                  <button
                    key={m.name}
                    data-model={m.name}
                    onClick={() => change(m.vendor, m.family, m.name)}
                  >
                    <LynxIcon name="board" size={20} />
                    <span>
                      <strong>{m.name}</strong>
                      <small>{chipBrief[m.name] ?? m.profile?.aliases[0]}</small>
                    </span>
                    <span>{m.profile?.routes ?? '—'}</span>
                    <small>{m.profile?.media.join(' / ') ?? '—'}</small>
                    <span aria-hidden="true">›</span>
                  </button>
                ))}
              </div>
              {!!list.length && (
                <p className="target-note">
                  <LynxIcon name="info" size={16} /> “—” 表示当前型号尚无适配的烧录路线。
                </p>
              )}
              {!list.length && <p role="status">未找到匹配型号，请核对丝印或尝试板名。</p>}
            </>
          )}
        </section>
      </div>
    </div>
  );
}
