import type { RefObject } from 'react';
import { LynxIcon } from '../../CoreUI';
import type {
  WorkbenchDevice,
  WorkbenchFlasherDevice,
} from '../../Platform/IPC/contracts/Workbench';
import {
  defaultVendorSelection,
  k230Media,
  vendorRoutes,
  type VendorInspection,
  type VendorSelection,
} from './vendorPlan';
import { ImageLayout, vendorLayouts } from './ImageLayout';
import './VendorRoutePanel.css';

interface RouteProps {
  dropRef: RefObject<HTMLDivElement | null>;
  boardName: string;
  mode: string;
  vendor: WorkbenchDevice['chipVendor'];
  selection: VendorSelection;
  onChange: (next: VendorSelection) => void;
  inspection?: VendorInspection | null;
  fileIssue?: string;
  loaderError?: string;
  imagePath: string;
  loaderPath: string;
  onChooseImage: () => void;
  onChooseLoader: () => void;
}
interface TargetProps {
  targetMedium?: string;
  selection: VendorSelection;
  onChange: (next: VendorSelection) => void;
  inspection?: VendorInspection | null;
  usb?: WorkbenchFlasherDevice | null;
  profile?: WorkbenchDevice | null;
}
const basename = (path: string) => path.split(/[\\/]/).pop() || path;
const flasherPrograms: Partial<Record<RouteProps['vendor'], string>> = {
  rockchip: 'OpenRockCLI',
  nxp: 'UUU',
  st: 'dfu-util',
  kendryte: 'K230',
};
const descriptions: Record<VendorSelection['route'], string> = {
  'rk-update': '选择板厂提供的 update.img，使用包内的固件结构。',
  'rk-emmc': '选择 .img、.img.xz 或 .img.gz 整盘镜像，并提供与板型匹配的 RK RAM Loader。',
  'nxp-uuu': '从本地选择 UUU 脚本。保留脚本和配套文件的原有目录结构。',
  'nxp-configured': '选择单板已保存 UUU 配置要求的输入文件。目标介质和执行顺序由单板配置决定。',
  'st-dfuse': '从 DfuSe 容器读取 Alternate、元素地址与长度，在设备连接时核对目标。',
  'st-raw': '选择平铺的 .bin 或 .img。连接设备后需要指定 Alternate 和板卡文档中的写入地址。',
  'k230-kdimage': '解析 KDImage 分区表、目标偏移和文件边界，连接后选择板卡的存储介质。',
  'k230-raw': '整个文件从所选介质的偏移 0 写入。请核对实际板型和介质。',
};
export function VendorRoutePanel({
  dropRef,
  boardName,
  mode,
  vendor,
  selection: s,
  onChange,
  inspection,
  fileIssue,
  loaderError,
  imagePath,
  loaderPath,
  onChooseImage,
  onChooseLoader,
}: RouteProps) {
  const choices = vendorRoutes[vendor] || [];
  if (!choices.length) return null;
  const result = inspection?.route === s.route && inspection.path === imagePath ? inspection : null;
  const title =
    s.route === 'nxp-uuu'
      ? '脚本引用的文件'
      : s.route === 'st-dfuse'
        ? '容器内的目标元素'
        : s.route === 'k230-kdimage'
          ? 'KDImage 分区表'
          : '文件检查';
  return (
    <div className="vendor-file-layout">
      <section className="vendor-detail" aria-label="平台烧录路线">
        <div className="vendor-detail-heading">
          <h3>烧录路线</h3>
          <div className="vendor-switch" role="group" aria-label="选择烧录路线">
            {choices.map(({ route, label }) => (
              <button
                type="button"
                key={route}
                aria-pressed={s.route === route}
                onClick={() => {
                  if (route !== s.route) onChange({ ...defaultVendorSelection(vendor), route });
                }}
              >
                {label}
              </button>
            ))}
          </div>
        </div>
        <p>{descriptions[s.route]}</p>
        <div className="vendor-file vendor-image-slot" ref={dropRef}>
          <LynxIcon name={s.route === 'nxp-uuu' ? 'code' : 'document'} size={24} />
          <div>
            <small>{s.route === 'nxp-uuu' ? 'UUU 脚本' : '镜像文件'}</small>
            <strong title={imagePath}>{imagePath ? basename(imagePath) : '选择或拖入文件'}</strong>
          </div>
          <button type="button" onClick={onChooseImage}>
            <LynxIcon name="folder" size={16} /> {imagePath ? '更换文件' : '选择文件'}
          </button>
        </div>
        {s.route === 'rk-emmc' && (
          <div className="vendor-file">
            <span>RK RAM Loader</span>
            <strong title={loaderPath}>
              {loaderPath ? basename(loaderPath) : '缺少配套 Loader'}
            </strong>
            <button type="button" onClick={onChooseLoader}>
              {loaderPath ? '更换 Loader' : '选择 Loader'}
            </button>
          </div>
        )}
        {result && (
          <div className="vendor-readout">
            <h4>{title}</h4>
            {result.items.length > 0 ? (
              <dl>
                {result.items.map((v, i) => (
                  <div key={i} data-valid={v.ok}>
                    <dt>{v.label}</dt>
                    <dd>{v.detail}</dd>
                  </div>
                ))}
              </dl>
            ) : (
              !fileIssue && <p>未发现可静态列出的文件引用。</p>
            )}
            {result.blockers.map((text, i) => (
              <p className="vendor-issue" key={i}>
                {text}
              </p>
            ))}
            {fileIssue && !result.blockers.includes(fileIssue) && (
              <p className="vendor-issue" role="alert">
                {fileIssue}
              </p>
            )}
          </div>
        )}
        {s.route === 'rk-emmc' && loaderError && (
          <p className="vendor-issue" role="alert">
            Loader 读取失败：{loaderError}
          </p>
        )}
      </section>
      <aside className="vendor-detail vendor-context" aria-label="本次写入目标">
        <h3>本次写入目标</h3>
        <dl>
          <div>
            <dt>目标型号</dt>
            <dd>{boardName}</dd>
          </div>
          <div>
            <dt>下载模式</dt>
            <dd>{mode}</dd>
          </div>
          <div>
            <dt>烧录路线</dt>
            <dd>{choices.find((item) => item.route === s.route)?.label}</dd>
          </div>
          <div>
            <dt>烧录程序</dt>
            <dd>{flasherPrograms[vendor]}</dd>
          </div>
          {result && (
            <div>
              <dt>输入格式</dt>
              <dd>{result.format}</dd>
            </div>
          )}
        </dl>
      </aside>
      {result && !result.blockers.length && (
        <ImageLayout key={result.route + result.path} views={vendorLayouts(result)} />
      )}
    </div>
  );
}
export function VendorTargetPanel({
  targetMedium,
  selection: s,
  onChange,
  usb,
  inspection,
  profile,
}: TargetProps) {
  const key = usb?.key ?? '';
  const patch = (p: Partial<VendorSelection>) =>
    onChange({
      ...s,
      ...(s.targetKey !== key ? { alt: '', address: '', medium: '', eraseConfirmed: false } : {}),
      ...p,
      targetKey: key,
    });
  const current = s.targetKey === key;
  if (s.route === 'rk-emmc')
    return (
      <section className="vendor-detail">
        <h3>核对 eMMC 覆盖范围</h3>
        <dl>
          <div>
            <dt>目标</dt>
            <dd>{usb ? usb.chip + ' · USB ' + key : '等待连接 RK 设备'}</dd>
          </div>
          <div>
            <dt>写入区域</dt>
            <dd>eMMC 用户区整盘</dd>
          </div>
        </dl>
        <label className="vendor-check">
          <input
            type="checkbox"
            checked={current && s.eraseConfirmed}
            disabled={!usb}
            onChange={(e) => patch({ eraseConfirmed: e.target.checked })}
          />
          先擦除整个 eMMC 用户区，再写入镜像。
        </label>
      </section>
    );
  if (s.route.startsWith('st-')) {
    const alts = usb?.dfuAlternates ?? [];
    return (
      <section className="vendor-detail">
        <h3>DFU 目标接口</h3>
        <label className="vendor-field">
          设备 Alternate
          <select
            disabled={!alts.length}
            value={current ? s.alt : ''}
            onChange={(e) => patch({ alt: e.target.value })}
          >
            <option value="">选择设备返回的 Alternate</option>
            {alts.map((a, i) => (
              <option key={i} value={a.number}>
                {a.number} · {a.name || '未命名接口'}
              </option>
            ))}
          </select>
        </label>
        {s.route === 'st-raw' ? (
          <label className="vendor-field">
            RAW 目的地址
            <input
              type="text"
              value={current ? s.address : ''}
              onChange={(e) => patch({ address: e.target.value, alt: current ? s.alt : '' })}
              placeholder="按板卡文档填写 0x…"
            />
          </label>
        ) : (
          <p>
            容器目标 Alternate {inspection?.alternates.join('、') || '待解析'}，元素地址来自文件。
          </p>
        )}
        {!alts.length && <p>当前扫描尚未返回 Alternate，请进入 DFU 模式后重新扫描。</p>}
      </section>
    );
  }
  if (s.route.startsWith('k230-'))
    return (
      <section className="vendor-detail">
        <h3>选择实际存储介质</h3>
        <label className="vendor-field">
          单板存储
          <select
            value={current ? s.medium : ''}
            disabled={!usb}
            onChange={(e) => patch({ medium: e.target.value })}
          >
            <option value="">按板卡实际配置选择</option>
            {k230Media
              .filter(
                ([, label]) =>
                  !targetMedium ||
                  label === targetMedium ||
                  (targetMedium === 'SD' && label === 'SD 卡')
              )
              .map(([v, l]) => (
                <option key={v} value={v}>
                  {l}
                </option>
              ))}
          </select>
        </label>
        <p>USB 枚举仅识别烧录端口。容量与块大小由 Loader 在介质探测时返回。</p>
      </section>
    );
  if (s.route.startsWith('nxp-'))
    return (
      <section className="vendor-detail">
        <h3>UUU 执行来源</h3>
        <dl>
          <div>
            <dt>USB 协议</dt>
            <dd>{usb?.mode || '待连接'}</dd>
          </div>
          <div>
            <dt>{s.route === 'nxp-uuu' ? '命令来源' : '单板保存参数'}</dt>
            <dd>
              {s.route === 'nxp-uuu' ? '已选脚本' : profile?.flasherArgs || '待选择已配置的单板'}
            </dd>
          </div>
        </dl>
      </section>
    );
  return null;
}
