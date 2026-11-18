import { useEffect, useRef, useState } from 'react';
import { flushSync } from 'react-dom';
import { open } from '@tauri-apps/plugin-dialog';
import { LynxIcon } from '../../CoreUI';
import { invokeCommand } from '../../Platform/IPC';
import type { ImageInspection, ImageReference } from '../../Platform/IPC/contracts/ImagerImage';
import { formatErrorMessage } from '../../Utils';
import type { BasicRoute } from './BasicPlan';
import type { ImageReviewEvidence } from './imageReviewEvidence';
import type { AppComparison } from '../../Platform/IPC/contracts/ImagerImage';
import type { V2SystemPickerProps } from './V2Catalog';
import {
  awMainline,
  awRouteText,
  ramComponents,
  referenceProblem,
  sourceProblem,
} from './imageReviewModel';
import './SystemImageReview.css';

const basename = (s: string) => s.split(/[\\/]/).pop() || s;
const address = (n: number) => '0x' + n.toString(16).toUpperCase().padStart(6, '0');
const hexLine = (s: string) => (s.match(/../g) || []).join(' ').toUpperCase();

type Inspection = { report?: ImageInspection; error?: string };
function useInspection(path: string, digest: string | undefined): Inspection {
  const [snapshot, setSnapshot] = useState<{ key: string; result: Inspection } | null>(null);
  const key = JSON.stringify([path, digest]);
  useEffect(() => {
    if (!path || !digest) return;
    let alive = true;
    invokeCommand('imager_inspect_image', { path }).then(
      (report) => {
        if (alive)
          setSnapshot({
            key,
            result:
              report.sha256 === digest
                ? { report }
                : { error: '文件分析期间内容发生变化，请重新选择。' },
          });
      },
      (error) => {
        if (alive) setSnapshot({ key, result: { error: formatErrorMessage(error) } });
      }
    );
    return () => {
      alive = false;
    };
  }, [path, digest, key]);
  return snapshot?.key === key ? snapshot.result : {};
}

function ContainerEntries({ report }: { report: ImageInspection }) {
  return (
    <details className="imager-image-entries">
      <summary>
        IMAGEWTY v{report.containerVersion} · 查看 {report.entries.length} 个目录条目
      </summary>
      <div className="imager-image-directory">
        <table>
          <caption>包内文件目录</caption>
          <thead>
            <tr>
              <th>文件</th>
              <th>包内偏移</th>
              <th>原始大小</th>
            </tr>
          </thead>
          <tbody>
            {report.entries.map((e, i) => (
              <tr key={i}>
                <td>{e.name}</td>
                <td>
                  <code>{address(e.offset)}</code>
                </td>
                <td>{e.sizeBytes.toLocaleString()} B</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </details>
  );
}

function Analysis({
  allwinner,
  report,
  reference,
  referenceName,
  issue,
}: {
  allwinner: boolean;
  report: ImageInspection;
  reference: ImageReference | null;
  referenceName: string;
  issue: string;
}) {
  const app = reference?.applicationOffset;
  const layout = !!reference && !issue && app != null && app > 0 && app < report.sizeBytes;
  const span = layout ? (reference.applicationBytes ?? report.sizeBytes - app) : 0;
  const segments = layout
    ? [
        { label: '启动与预留区', start: 0, size: app },
        {
          label: reference.applicationBytes == null ? '应用所在范围' : '应用区',
          start: app,
          size: span,
        },
        ...(app + span < report.sizeBytes
          ? [{ label: '尾部区域', start: app + span, size: report.sizeBytes - app - span }]
          : []),
      ]
    : [];
  const samples = [
    {
      offset: 0,
      hex: report.headerHex,
      meaning: report.egonAtZero ? 'eGON.BT0 启动标记' : '文件开头',
    },
  ];
  if (report.egonAtZero && report.appHex)
    samples.push({
      offset: 0x80000,
      hex: report.appHex,
      meaning: layout && app === 0x80000 ? '清单标注的应用起点' : '文件偏移抽样',
    });
  return (
    <>
      <section className="imager-review-block">
        <div className="imager-review-band">
          <h3>镜像内布局</h3>
          <span>
            {layout
              ? '清单声明的文件布局 · ' + referenceName
              : report.containerVersion
                ? '来源为包内文件目录'
                : '布局来源待核对'}
          </span>
        </div>
        {layout ? (
          <>
            <div
              className="imager-image-map"
              role="img"
              aria-label={'应用区从 ' + address(app) + ' 开始'}
            >
              {segments.map((s, i) => (
                <span key={s.label} data-segment={i} style={{ flexGrow: s.size }} />
              ))}
            </div>
            <div className="imager-map-axis">
              <code>{address(0)}</code>
              <span>{report.sizeBytes.toLocaleString()} B</span>
            </div>
            <ul className="imager-map-legend">
              {segments.map((s, i) => (
                <li key={s.label}>
                  <i data-segment={i} />
                  <div>
                    <b>{s.label}</b>
                    <small>
                      <code>{address(s.start)}</code> · {s.size.toLocaleString()} B
                    </small>
                  </div>
                </li>
              ))}
            </ul>
          </>
        ) : report.containerVersion ? (
          <ContainerEntries report={report} />
        ) : (
          <p className="imager-review-empty">
            {issue
              ? '清单未通过核对，无法采用其中的布局。'
              : reference
                ? '清单中没有可展示的应用区边界。'
                : '尚无可核对的分区布局，可从文件选择页添加板型配套清单。'}
          </p>
        )}
      </section>
      <section className="imager-review-block">
        <div className="imager-review-band">
          <h3>文件字节抽查</h3>
          <span>本地文件偏移 · 每行最多 16 字节</span>
        </div>
        <table className="imager-byte-table">
          <thead>
            <tr>
              <th>偏移</th>
              <th>十六进制数据</th>
              <th>位置说明</th>
            </tr>
          </thead>
          <tbody>
            {samples.map((s) => (
              <tr key={s.offset}>
                <th scope="row">
                  <code>{address(s.offset)}</code>
                </th>
                <td>
                  <code>{hexLine(s.hex)}</code>
                </td>
                <td>{s.meaning}</td>
              </tr>
            ))}
          </tbody>
        </table>
        <p className="imager-feature">
          {report.containerVersion
            ? 'IMAGEWTY v' + report.containerVersion
            : report.egonAtZero
              ? '起始位置有 eGON.BT0 标记'
              : report.splAt8192
                ? '0x2000 处有主线 SPL 头'
                : allwinner
                  ? '未识别到全志容器或启动标记'
                  : '已读取原始文件，格式与用途见平台烧录配置。'}
        </p>
      </section>
      <section className="imager-review-block">
        <div className="imager-review-band">
          <h3>文件完整性</h3>
          <span role="status">
            {issue || (reference ? '文件指纹与清单一致。' : '尚未提供参考值，仅计算了文件指纹')}
          </span>
        </div>
        <div className="imager-review-hashes">
          <div>
            <small>本地 SHA-256</small>
            <code>{report.sha256}</code>
          </div>
          <div>
            <small>{reference ? '参考 SHA-256 · ' + referenceName : '参考值来源'}</small>
            {reference ? <code>{reference.sha256}</code> : <span>待提供构建或发布清单</span>}
          </div>
        </div>
      </section>
    </>
  );
}

export function AllwinnerRoute({ board, route, onRoute, candidate }: V2SystemPickerProps) {
  const mainline = awMainline(board.storage);
  return (
    <fieldset className="imager-aw-route">
      <legend>系统用途</legend>
      {(['vendor', mainline].filter(Boolean) as BasicRoute[]).map((r) => (
        <label key={r}>
          <input
            type="radio"
            name="aw-system-route"
            value={r}
            checked={route === r}
            disabled={!!candidate}
            onChange={() => onRoute(r)}
          />
          <span>
            <b>{awRouteText[r]?.[0]}</b>
            <small>{awRouteText[r]?.[2]}</small>
          </span>
        </label>
      ))}
    </fieldset>
  );
}

export function SystemImageReview(props: V2SystemPickerProps) {
  const {
    board,
    boardName,
    route,
    imagePath,
    identity,
    candidate,
    loaderPath,
    loaderIdentity,
    onReviewReady,
  } = props;
  const aw = board.chipVendor === 'allwinner';
  const needsRam = aw && route !== 'vendor';
  const [panel, setPanel] = useState<'image' | 'loader' | null>(null);
  const image = useInspection(aw || panel === 'image' ? imagePath : '', identity?.sha256);
  const loader = useInspection(needsRam ? loaderPath : '', loaderIdentity?.sha256);
  const heading = useRef<HTMLHeadingElement>(null);
  const imageButton = useRef<HTMLButtonElement>(null);
  const loaderButton = useRef<HTMLButtonElement>(null);
  function navigatePanel(next: typeof panel) {
    const origin = panel === 'loader' ? loaderButton : imageButton;
    flushSync(() => setPanel(next));
    (next ? heading.current : origin.current)?.focus();
  }
  const [manual, setManual] = useState<{
    reference?: ImageReference;
    name?: string;
    path?: string;
    error?: string;
    busy?: boolean;
  }>({});
  const [comparison, setComparison] = useState<{
    path?: string;
    result?: AppComparison;
    busy?: boolean;
    error?: string;
  }>({});
  const reviewKey = JSON.stringify([
    imagePath,
    identity?.sha256,
    needsRam && loaderPath,
    needsRam && loaderIdentity?.sha256,
  ]);
  const [acceptedKey, setAcceptedKey] = useState('');
  const accepted = !!identity && (!needsRam || !!loaderIdentity) && acceptedKey === reviewKey;
  const pending = useRef(0);
  useEffect(
    () => () => {
      pending.current++;
    },
    []
  );
  async function chooseReference() {
    const id = ++pending.current;
    let path: string | null = null;
    try {
      path = await open({
        multiple: false,
        directory: false,
        title: '选择系统镜像的构建或发布清单',
        filters: [{ name: 'JSON 清单', extensions: ['json'] }],
      });
      if (!path || id !== pending.current) return;
      setComparison({});
      setAcceptedKey('');
      setManual({ busy: true });
      const reference = await invokeCommand('imager_read_image_reference', { path });
      if (id === pending.current) setManual({ name: basename(path), path, reference });
    } catch (e) {
      if (id === pending.current)
        setManual({ name: path ? basename(path) : undefined, error: formatErrorMessage(e) });
    }
  }
  const manifest = candidate?.manifest;
  const reference: ImageReference | null = manifest
    ? {
        sha256: manifest.image.sha256,
        sizeBytes: manifest.image.sizeBytes,
        applicationOffset: manifest.layout.applicationOffset,
        applicationBytes: null,
        boardModel: manifest.boardModel,
        fileName: manifest.image.fileName,
      }
    : (manual.reference ?? null);
  const report = image.report;
  const conflict = reference && report ? referenceProblem(reference, report, route, boardName) : '';
  async function chooseApplication() {
    const offset = reference?.applicationOffset;
    if (offset == null || conflict) return;
    const id = ++pending.current;
    try {
      const path = await open({
        multiple: false,
        directory: false,
        title: '选择构建产出的应用文件',
        filters: [
          { name: '应用文件', extensions: ['bin'] },
          { name: '所有文件', extensions: ['*'] },
        ],
      });
      if (!path || id !== pending.current) return;
      setAcceptedKey('');
      setComparison({ path, busy: true });
      const result = await invokeCommand('imager_compare_application', {
        imagePath,
        applicationPath: path,
        offset,
        expectedBytes: reference?.applicationBytes ?? null,
      });
      if (id === pending.current) setComparison({ path, result });
    } catch (e) {
      if (id === pending.current) setComparison({ error: formatErrorMessage(e) });
    }
  }
  const profile = aw && report ? sourceProblem(route, report) : '';
  const ramProblem =
    needsRam && loader.report && !ramComponents(loader.report).every(Boolean)
      ? 'RAM 引导来源未同时包含 FES 和 U-Boot 组件。'
      : '';
  const loaderMatch =
    !manifest ||
    (loaderIdentity?.sha256 === manifest.loader.sha256 &&
      loaderIdentity.sizeBytes === manifest.loader.sizeBytes);
  const problem =
    manual.error ||
    (needsRam && props.loaderError) ||
    comparison.error ||
    (comparison.result && !comparison.result.matches
      ? '镜像的应用区与所选应用文件内容不同。'
      : '') ||
    image.error ||
    loader.error ||
    conflict ||
    profile ||
    ramProblem ||
    '';
  const ready =
    !!identity &&
    (aw
      ? !!report &&
        !problem &&
        !manual.busy &&
        !comparison.busy &&
        (!!manifest || accepted) &&
        (!needsRam || (!!loader.report && loaderMatch && loader.report.sha256 !== report.sha256))
      : true);
  useEffect(() => {
    const evidence: ImageReviewEvidence = {};
    if (manual.path && manual.reference?.documentSha256)
      evidence.reference = { path: manual.path, sha256: manual.reference.documentSha256 };
    if (comparison.result?.matches && comparison.path && reference?.applicationOffset != null)
      evidence.application = {
        imagePath,
        applicationPath: comparison.path,
        offset: reference.applicationOffset,
        expectedBytes: reference.applicationBytes,
        sha256: comparison.result.applicationSha256,
      };
    onReviewReady(ready, ready ? evidence : undefined);
  }, [
    ready,
    onReviewReady,
    manual.path,
    manual.reference,
    comparison,
    imagePath,
    reference?.applicationOffset,
    reference?.applicationBytes,
  ]);
  const mainline = awMainline(board.storage);
  const openRefButton = (
    <button
      type="button"
      disabled={!!manifest || !!manual.busy}
      onClick={() => void chooseReference()}
    >
      {manual.busy ? '读取清单…' : reference ? '更换参考清单…' : '选择参考清单…'}
    </button>
  );
  const bothSame = needsRam && !!loader.report && loader.report.sha256 === report?.sha256;
  const detailReport = panel === 'loader' ? loader.report : report;
  return (
    <section className="imager-image-review" aria-label="文件用途与分析">
      {!panel && (
        <>
          {!!imagePath && (
            <div
              className={'imager-image-line' + (props.vendorControls ? ' vendor-file-tools' : '')}
              ref={props.source === 'local' ? props.dropRef : undefined}
            >
              {!props.vendorControls && (
                <>
                  <div>
                    <h3>{aw ? awRouteText[route]?.[1] || '系统镜像' : '系统镜像'}</h3>
                    <code>
                      <LynxIcon name="document" size={16} />
                      {basename(imagePath)}
                    </code>
                    <small>
                      {manifest ? '板型资源' : '本地文件'}
                      {identity ? ' · ' + identity.sizeBytes.toLocaleString() + ' B' : ''}
                    </small>
                  </div>
                  <div className="imager-file-destination">
                    <LynxIcon name="database" size={20} />
                    <div>
                      <strong>
                        {route === 'mainline_spinor'
                          ? '写入 SPI NOR'
                          : route === 'mainline_nand'
                            ? '写入 NAND 组件'
                            : route === 'mainline_emmc'
                              ? '写入 eMMC'
                              : '安装厂商系统'}
                      </strong>
                      <small>
                        {route === 'mainline_spinor'
                          ? '物理起点 0x000000'
                          : awRouteText[route]?.[2]}
                      </small>
                    </div>
                  </div>
                </>
              )}
              <div className="imager-image-actions">
                {!manifest && !props.vendorControls && (
                  <button type="button" onClick={props.onBrowse}>
                    更换系统镜像…
                  </button>
                )}
                <button
                  type="button"
                  ref={imageButton}
                  disabled={!identity}
                  onClick={() => navigatePanel('image')}
                >
                  分析系统镜像
                </button>
                {!manifest && (
                  <button
                    type="button"
                    className="imager-image-remove"
                    onClick={() => props.onImagePath('')}
                  >
                    移除
                  </button>
                )}
              </div>
            </div>
          )}
          {aw &&
            route === 'vendor' &&
            mainline &&
            report &&
            ramComponents(report).every(Boolean) &&
            !manifest && (
              <button type="button" onClick={() => props.onUseImageAsLoader(mainline)}>
                将这个包改作 RAM 引导来源
              </button>
            )}
          {needsRam && (
            <div className="imager-image-line">
              <div>
                <h3>RAM FES 引导来源</h3>
                <code>
                  <LynxIcon name="document" size={16} />
                  {loaderPath ? basename(loaderPath) : '尚未选择引导包'}
                </code>
                <small>
                  {loaderIdentity
                    ? '板型配套包 · ' + loaderIdentity.sizeBytes.toLocaleString() + ' B'
                    : '请选择板型配套的全志原厂包。'}
                </small>
              </div>
              <div className="imager-file-destination">
                <LynxIcon name="board" size={20} />
                <div>
                  <strong>仅加载到 RAM</strong>
                  <small>提取 FES 与 U-Boot</small>
                </div>
              </div>
              <div className="imager-image-actions">
                <button type="button" disabled={!!manifest} onClick={props.onChooseLoader}>
                  {loaderPath ? '更换引导包…' : '选择引导包…'}
                </button>
                {loaderPath && (
                  <button type="button" ref={loaderButton} onClick={() => navigatePanel('loader')}>
                    检查引导组件
                  </button>
                )}
              </div>
            </div>
          )}
          {aw && !!imagePath && (
            <div className="imager-image-reference">
              <div>
                <b>系统镜像参考值</b>
                <small>
                  {manual.error ||
                    (reference
                      ? manifest
                        ? '板型资源发布清单'
                        : manual.name
                      : '可使用 BUILD-INFO.json 或发布清单核对指纹。')}
                </small>
              </div>
              {!manifest && openRefButton}
            </div>
          )}
          {(problem || bothSame) && (
            <p className="imager-image-problem" role="alert">
              {problem || '系统镜像与引导包内容相同，请分别选择写入文件及 RAM 引导来源。'}
            </p>
          )}
          {aw && !manifest && !!imagePath && (
            <label className="imager-image-confirm">
              <input
                type="checkbox"
                checked={accepted}
                disabled={!identity || (needsRam && !loaderIdentity)}
                onChange={(e) => setAcceptedKey(e.target.checked ? reviewKey : '')}
              />
              <span>
                已依据板卡配套资料核对 {boardName} 的镜像用途、存储介质
                {needsRam ? '和引导来源' : ''}。
              </span>
            </label>
          )}
          {aw && (!!imagePath || !!loaderPath) && !problem && (
            <small role="status">
              {ready
                ? '当前文件可进入设备连接。'
                : !identity && imagePath
                  ? '正在读取系统镜像…'
                  : identity && !report
                    ? '正在分析文件内容…'
                    : needsRam && !loader.report
                      ? '等待有效的 RAM 引导来源。'
                      : '请核对板型配套资料。'}
            </small>
          )}
        </>
      )}
      {panel && (
        <section
          className="imager-review-page"
          aria-labelledby="imager-review-title"
          data-inspector={panel}
          onKeyDown={(event) => {
            if (event.key === 'Escape' && !event.defaultPrevented) {
              event.preventDefault();
              navigatePanel(null);
            }
          }}
        >
          <header className="imager-review-heading">
            <div>
              <h2 id="imager-review-title" ref={heading} tabIndex={-1}>
                {panel === 'loader' ? '核对 RAM FES 引导来源' : '分析系统镜像'}
              </h2>
              <p>
                {panel === 'loader'
                  ? '核对板型配套包、RAM 引导组件和文件指纹。'
                  : '核对镜像的实际字节、构建清单和内容布局。'}
              </p>
            </div>
            <button type="button" onClick={() => navigatePanel(null)}>
              返回文件选择
            </button>
          </header>
          <div className="imager-review-file">
            <LynxIcon name="document" size={20} />
            <div>
              <strong>{basename(panel === 'loader' ? loaderPath : imagePath)}</strong>
              <small>
                {boardName}
                {detailReport ? ' · ' + detailReport.sizeBytes.toLocaleString() + ' 字节' : ''}
              </small>
            </div>
            <span>
              {panel === 'loader'
                ? '仅加载到 RAM'
                : detailReport?.containerVersion
                  ? 'IMAGEWTY 固件包'
                  : '本地镜像文件'}
            </span>
          </div>
          {panel === 'loader'
            ? loader.report && (
                <>
                  <section className="imager-review-block">
                    <div className="imager-review-band">
                      <h3>引导组件</h3>
                      <span>来源为包内文件目录</span>
                    </div>
                    <table className="imager-loader-table">
                      <thead>
                        <tr>
                          <th>组件</th>
                          <th>包内文件</th>
                          <th>大小</th>
                          <th>用途</th>
                        </tr>
                      </thead>
                      <tbody>
                        {ramComponents(loader.report).map((entry, i) => (
                          <tr key={i}>
                            <th scope="row">{i ? 'U-Boot' : 'FES'}</th>
                            <td>
                              <code>{entry?.name || '缺少组件'}</code>
                            </td>
                            <td>{entry ? entry.sizeBytes.toLocaleString() + ' B' : '—'}</td>
                            <td>{i ? '建立 RAM 烧录会话' : '初始化板上 DRAM'}</td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </section>
                  <section className="imager-review-block">
                    <div className="imager-review-band">
                      <h3>板型与来源</h3>
                    </div>
                    <dl className="imager-image-facts">
                      <div>
                        <dt>运行位置</dt>
                        <dd>板上 RAM · FEL 加载后建立 FES 烧录会话</dd>
                      </div>
                      <div>
                        <dt>板型依据</dt>
                        <dd>
                          {board.id === 'f101' && loader.report.knownF101Loader
                            ? '本包 SHA-256 与工程中已验证的 F101 引导来源一致。'
                            : manifest && loaderMatch
                              ? '引导文件指纹与该板型的发布清单一致。'
                              : '需依据板型及 DRAM 配置核对引导来源。'}
                        </dd>
                      </div>
                      <div>
                        <dt>包 SHA-256</dt>
                        <dd>
                          <code>{loader.report.sha256}</code>
                        </dd>
                      </div>
                    </dl>
                  </section>
                  <ContainerEntries report={loader.report} />
                </>
              )
            : report && (
                <Analysis
                  allwinner={aw}
                  report={report}
                  reference={reference}
                  referenceName={manifest ? '板型资源发布清单' : manual.name || ''}
                  issue={conflict}
                />
              )}
          {!(panel === 'loader' ? loader.report : report) && (
            <p role="status">
              {(panel === 'loader' ? loader.error : image.error) || '正在读取文件…'}
            </p>
          )}
          {panel === 'image' && (
            <section className="imager-review-block">
              <div className="imager-review-band">
                <h3>独立应用文件比对</h3>
                <span>
                  {reference?.applicationOffset != null && !conflict
                    ? '依据清单中的应用起点'
                    : '需要带有效应用偏移的清单'}
                </span>
              </div>
              {reference?.applicationOffset != null && !conflict ? (
                <>
                  <button
                    type="button"
                    disabled={!!comparison.busy}
                    onClick={() => void chooseApplication()}
                  >
                    {comparison.busy ? '正在比对…' : '选择应用文件并比对…'}
                  </button>
                  {comparison.result && (
                    <p role="status">
                      {basename(comparison.path || '')} ·{' '}
                      {comparison.result.comparedBytes.toLocaleString()} 字节 ·{' '}
                      {comparison.result.matches ? '与镜像应用区逐字节一致' : '与镜像应用区不同'}
                      <br />
                      <code>{hexLine(comparison.result.sampleHex)}</code>
                    </p>
                  )}
                  {comparison.error && <p role="alert">{comparison.error}</p>}
                </>
              ) : (
                <p className="imager-review-empty">
                  选择配套清单后，可将独立应用文件与镜像中的对应范围逐字节比较。
                </p>
              )}
            </section>
          )}
        </section>
      )}
    </section>
  );
}
