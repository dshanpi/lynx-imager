import { useState, type CSSProperties } from 'react';
import type {
  ImageInspection,
  ImageReference,
  ImageLayoutData,
} from '../../Platform/IPC/contracts/ImagerImage';
import type { VendorInspection } from './vendorPlan';
import './ImageLayout.css';

type Region = { name: string; offset: number | null; bytes: number | null };
type View = {
  source: string;
  coordinate: string;
  regions: Region[];
  extent?: number | null;
  flat?: boolean;
};
const hex = (v: number) => '0x' + v.toString(16).toUpperCase().padStart(6, '0');
function size(v: number) {
  if (v < 1024) return v + ' B';
  const unit = v >= 2 ** 30 ? 3 : v >= 2 ** 20 ? 2 : 1;
  return +(v / 1024 ** unit).toFixed(2) + ' ' + ['B', 'KiB', 'MiB', 'GiB'][unit];
}
const percent = (v: number) => (v > 0 && v < 0.1 ? '<0.1%' : +v.toFixed(1) + '%');

export function awLayouts(
  report: ImageInspection,
  ref: ImageReference | null,
  nor: boolean
): View[] {
  if (nor && report.containerVersion == null) {
    const regions: Region[] = [];
    if (report.boot0Bytes)
      regions.push({ name: '启动段 · eGON', offset: 0, bytes: report.boot0Bytes });
    else if (report.egonAtZero) regions.push({ name: 'eGON 启动标记', offset: 4, bytes: 8 });
    const parameters = ref?.parametersOffset;
    if (parameters != null && parameters >= 0 && parameters < report.sizeBytes)
      regions.push({ name: '参数起点 · 清单', offset: parameters, bytes: null });
    const start = ref?.applicationOffset;
    if (start != null && start >= 0 && start < report.sizeBytes) {
      const bytes = ref?.applicationBytes ?? null;
      if (bytes == null || (bytes > 0 && bytes <= report.sizeBytes - start))
        regions.push({
          name: bytes == null ? '应用起点 · 清单' : '应用区 · 清单',
          offset: start,
          bytes,
        });
    }
    return [
      {
        source: 'SPI NOR 平铺镜像',
        coordinate: '文件偏移',
        extent: report.sizeBytes,
        flat: true,
        regions,
      },
    ];
  }
  const views: View[] = report.layout ? [report.layout] : [];
  if (report.entries.length)
    views.push({
      source: 'IMAGEWTY 包目录',
      coordinate: '包内偏移',
      extent: report.sizeBytes,
      regions: report.entries.map((e) => ({
        name: e.name,
        offset: e.offset,
        bytes: e.storedBytes ?? null,
      })),
    });
  return views;
}

export function vendorLayouts(result: VendorInspection): View[] {
  const views: View[] = (result.layouts ?? []).map((layout: ImageLayoutData) => ({
    ...layout,
    extent: layout.extent ?? (layout.coordinate === '包内偏移' ? result.bytes : undefined),
  }));
  if (result.dependencies.length)
    views.push({
      source: 'UUU 引用文件',
      coordinate: '文件集合',
      regions: result.dependencies.map((d) => ({ name: d.path, offset: null, bytes: d.bytes })),
    });
  if (!views.length && result.route === 'st-raw')
    views.push({
      source: 'RAW 文件',
      coordinate: '文件偏移',
      extent: result.bytes,
      regions: [{ name: '固件数据', offset: 0, bytes: result.bytes }],
    });
  return views;
}

export function ImageLayout({ views }: { views: View[] }) {
  const [group, setGroup] = useState(0);
  const view = views[group] ?? views[0];
  if (!view)
    return (
      <div className="img-layout il-empty">
        <strong>镜像布局</strong>此文件未解析出可显示的分区表。
      </div>
    );
  const included = view.regions.filter(
    (r) => Number.isSafeInteger(r.offset ?? 0) && Number.isSafeInteger(r.bytes ?? 0)
  );
  const total = view.extent ?? included.reduce((n, r) => n + (r.bytes ?? 0), 0);
  let cursor = 0;
  const rows = included.map((r, i) => {
    const start = view.extent ? (r.offset ?? 0) : cursor;
    cursor += r.bytes ?? 0;
    return {
      ...r,
      left: total ? (start / total) * 100 : 0,
      share: total ? ((r.bytes ?? 0) / total) * 100 : 0,
      index: i,
    };
  });
  return (
    <section className="img-layout" aria-label="镜像布局">
      <header>
        <div>
          <h3>镜像布局</h3>
          {views.length > 1 ? (
            <select
              aria-label="布局来源"
              value={group}
              onChange={(e) => setGroup(Number(e.target.value))}
            >
              {views.map((v, i) => (
                <option value={i} key={i}>
                  {v.source}
                </option>
              ))}
            </select>
          ) : (
            <span>{view.source}</span>
          )}
        </div>
        <span>
          {view.extent ? '文件大小' : '按大小排列 · 已知区段合计'} <b>{size(total)}</b>
        </span>
      </header>
      <div
        className={'il-map' + (view.flat ? ' il-flat' : '')}
        role="img"
        aria-label={view.flat ? '系统镜像文件范围与已知标记' : '各区段按已知大小绘制，详见下表'}
      >
        {view.flat && <span className="il-flat-caption">连续写入 SPI NOR · {size(total)}</span>}
        {rows.map((r) => (
          <i
            key={r.index}
            data-tone={r.index % 6}
            style={{ left: r.left + '%', width: r.share + '%' }}
            title={r.name}
          >
            {r.share >= 13 && <span>{r.name}</span>}
          </i>
        ))}
      </div>
      {view.extent && (
        <div className="il-axis">
          <span>0x000000</span>
          <span>{hex(total)} · 文件末端</span>
        </div>
      )}
      {!rows.length ? (
        <small>目前只确定文件范围。</small>
      ) : (
        <div className="il-scroll" tabIndex={0} role="region" aria-label="区段明细">
          <table>
            <thead>
              <tr>
                <th>{view.flat ? '已知标记' : '区段名称'}</th>
                {view.coordinate !== '文件集合' && <th>{view.coordinate}</th>}
                <th>大小</th>
                <th>{view.extent ? '占文件' : '占已知区段'}</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => (
                <tr key={r.index} data-tone={r.index % 6}>
                  <th scope="row">
                    <i />
                    {r.name}
                  </th>
                  {view.coordinate !== '文件集合' && (
                    <td>{r.offset != null ? hex(r.offset) : '—'}</td>
                  )}
                  <td>{r.bytes ? size(r.bytes) : '未声明'}</td>
                  <td>
                    <div className="il-ratio">
                      <span>{r.bytes ? percent(r.share) : '—'}</span>
                      <i style={{ '--share': r.share + '%' } as CSSProperties} />
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}
