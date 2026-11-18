import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { invokeCommand, subscribeEvent, type WorkbenchDevice } from '../../Platform/IPC';
import './ImagerPage.css';

type BasicStep = 1 | 2 | 3 | 4;

type BoardPreset = {
  id: string;
  name: string;
  chipVendor: WorkbenchDevice['chipVendor'];
  chip: string;
  storage: string;
  mode: string;
  route:
    | 'vendor'
    | 'mainline_nand'
    | 'mainline_emmc'
    | 'mainline_spinor'
    | 'rockchip_raw_emmc';
  needsLoader: boolean;
  loaderLabel?: string;
};

const BOARD_PRESETS: BoardPreset[] = [
  {
    id: 't113-pro',
    name: 'T113-Pro',
    chipVendor: 'allwinner',
    chip: '全志 · T113',
    storage: 'SPI NAND',
    mode: 'FEL',
    route: 'vendor',
    needsLoader: false,
  },
  {
    id: 'f101',
    name: 'F101',
    chipVendor: 'allwinner',
    chip: '全志 · F101',
    storage: 'SPI NOR',
    mode: 'FEL',
    route: 'mainline_spinor',
    needsLoader: true,
  },
  {
    id: 'rk3588',
    name: 'RK3588 系列',
    chipVendor: 'rockchip',
    chip: '瑞芯微 · RK3588',
    storage: 'eMMC',
    mode: 'Maskrom / Loader',
    route: 'rockchip_raw_emmc',
    needsLoader: true,
    loaderLabel: 'Rockchip Loader',
  },
  {
    id: 'nxp-imx',
    name: 'i.MX 系列',
    chipVendor: 'nxp',
    chip: 'NXP · i.MX 8 / 9',
    storage: 'eMMC / SD',
    mode: 'SDP / Fastboot',
    route: 'vendor',
    needsLoader: false,
  },
  {
    id: 'stm32mp1',
    name: 'STM32MP1 系列',
    chipVendor: 'st',
    chip: 'ST · STM32MP1',
    storage: 'eMMC / SD',
    mode: 'USB DFU / DfuSe',
    route: 'vendor',
    needsLoader: false,
  },
  {
    id: 'canaan-k230',
    name: 'K230 系列',
    chipVendor: 'kendryte',
    chip: 'Canaan · Kendryte K230',
    storage: 'eMMC / SD',
    mode: 'BROM / U-Boot',
    route: 'vendor',
    needsLoader: false,
  },
];

const STEPS: Array<{ id: BasicStep; label: string; hint: string }> = [
  { id: 1, label: '选择板子', hint: '确认芯片、介质和连接模式' },
  { id: 2, label: '选择系统', hint: '选择本地镜像和 Loader' },
  { id: 3, label: '配置接口', hint: '绑定已保存的目标设备' },
  { id: 4, label: '烧写系统', hint: '预检、确认、写入和校验' },
];

const BOARD_FILTERS: Array<{ id: BoardPreset['chipVendor'] | 'all'; label: string }> = [
  { id: 'all', label: '全部' },
  { id: 'allwinner', label: '全志' },
  { id: 'rockchip', label: '瑞芯微' },
  { id: 'nxp', label: 'NXP' },
  { id: 'st', label: 'ST' },
  { id: 'kendryte', label: 'K230' },
];

export interface ImagerPageProps {
  onOpenAdvancedWorkbench: () => void;
}

function BoardGlyph({ board, large = false }: { board: BoardPreset; large?: boolean }) {
  return (
    <svg
      className={`imager-board-glyph ${large ? 'is-large' : ''}`}
      viewBox="0 0 160 92"
      role="img"
      aria-label={`${board.name} 示意图`}
    >
      <rect x="10" y="12" width="140" height="68" rx="8" />
      <circle cx="20" cy="22" r="4" />
      <circle cx="140" cy="22" r="4" />
      <circle cx="20" cy="70" r="4" />
      <circle cx="140" cy="70" r="4" />
      <rect className="imager-board-glyph-chip" x="62" y="28" width="36" height="30" rx="4" />
      <rect x="30" y="28" width="18" height="8" rx="2" />
      <rect x="108" y="30" width="20" height="8" rx="2" />
      <rect x="34" y="50" width="13" height="8" rx="2" />
      <rect x="105" y="50" width="17" height="8" rx="2" />
      {board.chipVendor === 'rockchip' && <path d="M122 42h12v22h-12z" />}
      {board.chipVendor === 'nxp' && <rect x="48" y="62" width="26" height="7" rx="2" />}
      {board.chipVendor === 'st' && <path d="M112 19v12m6-12v12m6-12v12" />}
      {board.chipVendor === 'kendryte' && <rect x="28" y="40" width="12" height="12" rx="2" />}
    </svg>
  );
}

function deviceMatchesBoard(device: WorkbenchDevice, board: BoardPreset) {
  return device.chipVendor === board.chipVendor;
}

export const ImagerPage: React.FC<ImagerPageProps> = ({
  onOpenAdvancedWorkbench,
}) => {
  const [step, setStep] = useState<BasicStep>(1);
  const [boardId, setBoardId] = useState('t113-pro');
  const [boardSearch, setBoardSearch] = useState('');
  const [boardFilter, setBoardFilter] = useState<BoardPreset['chipVendor'] | 'all'>('all');
  const [imagePath, setImagePath] = useState('');
  const [loaderPath, setLoaderPath] = useState('');
  const [devices, setDevices] = useState<WorkbenchDevice[]>([]);
  const [selectedDeviceId, setSelectedDeviceId] = useState<number | null>(null);
  const [confirmChecked, setConfirmChecked] = useState(false);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState('');
  const [error, setError] = useState('');
  const [taskId, setTaskId] = useState('');
  const [taskStatus, setTaskStatus] = useState('');

  const board = useMemo(
    () => BOARD_PRESETS.find((candidate) => candidate.id === boardId) ?? BOARD_PRESETS[0],
    [boardId]
  );

  const visibleBoards = useMemo(() => {
    const query = boardSearch.trim().toLowerCase();
    return BOARD_PRESETS.filter((candidate) => {
      const matchesFilter = boardFilter === 'all' || candidate.chipVendor === boardFilter;
      const matchesQuery =
        !query ||
        `${candidate.name} ${candidate.chip} ${candidate.storage}`.toLowerCase().includes(query);
      return matchesFilter && matchesQuery;
    });
  }, [boardFilter, boardSearch]);

  const loadSavedDevices = useCallback(async () => {
    setBusy(true);
    setError('');
    setNotice('正在读取已保存的设备配置…');
    try {
      const result = await invokeCommand('workbench_list_devices', undefined, { timeoutMs: 8000 });
      const compatible = result.filter((device) => deviceMatchesBoard(device, board));
      setDevices(compatible);
      setSelectedDeviceId((current) =>
        compatible.some((device) => device.id === current) ? current : (compatible[0]?.id ?? null)
      );
      setNotice(
        compatible.length > 0
          ? `已找到 ${compatible.length} 个与 ${board.name} 匹配的已保存设备`
          : `没有找到与 ${board.name} 匹配的已保存设备`
      );
    } catch (cause) {
      setDevices([]);
      setSelectedDeviceId(null);
      setError(cause instanceof Error ? cause.message : '无法读取设备配置，请在 LYNX 桌面版中重试');
      setNotice('设备读取失败');
    } finally {
      setBusy(false);
    }
  }, [board]);

  useEffect(() => {
    if (!taskId) return;
    let unlisten: (() => void) | undefined;
    void subscribeEvent('workbench-task-event', (event) => {
      if (event.taskId !== taskId) return;
      const progress =
        event.current != null && event.total ? ` · ${event.current}/${event.total}` : '';
      setTaskStatus(`${event.status || event.phase}${progress}`);
      if (event.status === 'success' || event.status === 'failed' || event.status === 'cancelled') {
        setBusy(false);
      }
    }).then((cleanup) => {
      unlisten = cleanup;
    });
    return () => unlisten?.();
  }, [taskId]);

  const pickPath = useCallback(async (kind: 'image' | 'loader') => {
    try {
      const selected = await open({
        multiple: false,
        directory: false,
        title: kind === 'image' ? '选择系统镜像' : '选择匹配板型的 Loader',
        filters:
          kind === 'image'
            ? [{ name: '镜像文件', extensions: ['img', 'bin', 'zip'] }]
            : [{ name: 'Loader', extensions: ['fex', 'bin', 'img'] }],
      });
      if (typeof selected !== 'string') return;
      if (kind === 'image') setImagePath(selected);
      else setLoaderPath(selected);
      setError('');
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : '文件选择失败');
    }
  }, []);

  const canContinue =
    step === 1 ||
    (step === 2 && Boolean(imagePath) && (!board.needsLoader || Boolean(loaderPath))) ||
    (step === 3 && selectedDeviceId != null) ||
    (step === 4 && confirmChecked);

  const startFlash = useCallback(async () => {
    if (selectedDeviceId == null || !imagePath || !confirmChecked) return;
    setBusy(true);
    setError('');
    setNotice('正在提交烧录任务，后端会在擦写前再次执行安全预检…');
    try {
      const usesSavedVendorRecipe = board.route === 'vendor' && board.chipVendor !== 'allwinner';
      const started = await invokeCommand('workbench_start_flash', {
        deviceId: selectedDeviceId,
        firmwarePath: imagePath,
        options: usesSavedVendorRecipe
          ? { systemType: 'vendor' }
          : {
              systemType: board.route === 'vendor' ? 'vendor' : 'mainline',
              loaderPath: loaderPath || undefined,
              mainlineRoute: board.route === 'vendor' ? undefined : board.route,
              mode: 'full_erase',
              verify: true,
              postFlashAction: 'reboot',
            },
      });
      setTaskId(started);
      setTaskStatus('任务已创建，等待后端事件');
      setNotice('烧录任务已提交。请等待任务终态，界面不会自动重试。');
    } catch (cause) {
      setBusy(false);
      setError(cause instanceof Error ? cause.message : '烧录任务提交失败');
      setNotice('任务未开始，设备保持不动');
    }
  }, [board.chipVendor, board.route, confirmChecked, imagePath, loaderPath, selectedDeviceId]);

  return (
    <section className="imager-page" aria-label="LYNX Imager">
      <div className="imager-basic-layout">
          <aside className="imager-stepper" aria-label="烧录步骤">
            <div className="imager-stepper-title">基础烧录</div>
            <div className="imager-stepper-subtitle">四步完成系统写入</div>
            <ol>
              {STEPS.map((item) => (
                <li
                  key={item.id}
                  className={step === item.id ? 'is-active' : step > item.id ? 'is-complete' : ''}
                >
                  <button
                    onClick={() => item.id <= step && setStep(item.id)}
                    disabled={item.id > step}
                  >
                    <span className="imager-step-number">{step > item.id ? '✓' : item.id}</span>
                    <span>
                      <strong>{item.label}</strong>
                      <small>{item.hint}</small>
                    </span>
                  </button>
                </li>
              ))}
            </ol>
            <div className="imager-safe-note">
              <span>安全门禁</span>
              <p>设备、镜像和 Loader 会在后端再次核对。失败不会自动重试。</p>
            </div>
          </aside>

          <main className="imager-content">
            {step === 1 && (
              <div className="imager-panel-stack">
                <div className="imager-page-heading">
                  <span>步骤 1 / 4</span>
                  <h2>选择你的开发板</h2>
                  <p>先选择型号，LYNX 会显示对应的存储介质和连接方式。</p>
                </div>
                <label className="imager-board-search">
                  <span aria-hidden="true">⌕</span>
                  <input
                    value={boardSearch}
                    onChange={(event) => setBoardSearch(event.target.value)}
                    placeholder="搜索板型、芯片或厂商"
                    aria-label="搜索板型、芯片或厂商"
                  />
                  <kbd>Ctrl</kbd><kbd>K</kbd>
                </label>
                <div className="imager-board-filters" role="tablist" aria-label="板型厂商筛选">
                  {BOARD_FILTERS.map((filter) => (
                    <button
                      key={filter.id}
                      type="button"
                      className={boardFilter === filter.id ? 'is-active' : ''}
                      onClick={() => setBoardFilter(filter.id)}
                      role="tab"
                      aria-selected={boardFilter === filter.id}
                    >
                      {filter.label}
                    </button>
                  ))}
                </div>
                <div className="imager-board-picker">
                  <div className="imager-board-list">
                    {visibleBoards.length ? visibleBoards.map((candidate) => (
                      <button
                        key={candidate.id}
                        className={`imager-board-row ${board.id === candidate.id ? 'is-selected' : ''}`}
                        onClick={() => {
                          setBoardId(candidate.id);
                          setStep(1);
                          setDevices([]);
                          setSelectedDeviceId(null);
                        }}
                      >
                        <BoardGlyph board={candidate} />
                        <span className="imager-board-row-copy">
                          <strong>{candidate.name}</strong>
                          <small>{candidate.chip} · {candidate.storage}</small>
                        </span>
                        <span className="imager-board-row-check" aria-hidden="true">
                          {board.id === candidate.id ? '✓' : ''}
                        </span>
                      </button>
                    )) : (
                      <div className="imager-board-no-results">没有匹配的板型</div>
                    )}
                    <div className="imager-supported-link">未找到板型？ <button type="button" onClick={onOpenAdvancedWorkbench}>查看支持列表 ↗</button></div>
                  </div>
                  <aside className="imager-board-detail">
                    <span className="imager-detail-label">已选板型</span>
                    <h3>{board.name}</h3>
                    <BoardGlyph board={board} large />
                    <dl>
                      <div><dt>平台</dt><dd>{board.chipVendor === 'allwinner' ? '全志' : board.chipVendor === 'rockchip' ? '瑞芯微' : board.chipVendor.toUpperCase()}</dd></div>
                      <div><dt>连接</dt><dd>{board.mode}</dd></div>
                      <div><dt>存储</dt><dd>{board.storage}</dd></div>
                    </dl>
                    <button type="button" className="imager-detail-link" onClick={() => setStep(2)}>查看板型说明 ↗</button>
                  </aside>
                </div>
              </div>
            )}

            {step === 2 && (
              <div className="imager-panel-stack">
                <div className="imager-page-heading">
                  <span>步骤 2 / 4</span>
                  <h2>选择系统镜像</h2>
                  <p>只填写当前板型需要的文件，路径会传给现有烧录后端。</p>
                </div>
                <label className="imager-field">
                  <span>系统镜像路径</span>
                  <div className="imager-input-row">
                    <input
                      value={imagePath}
                      onChange={(event) => setImagePath(event.target.value)}
                      placeholder="选择 .img / .bin 镜像，或粘贴完整路径"
                    />
                    <button onClick={() => void pickPath('image')}>选择文件</button>
                  </div>
                </label>
                {board.needsLoader && (
                  <label className="imager-field">
                    <span>匹配 Loader</span>
                    <div className="imager-input-row">
                      <input
                        value={loaderPath}
                        onChange={(event) => setLoaderPath(event.target.value)}
                        placeholder="选择与板型匹配的 Loader 文件"
                      />
                      <button onClick={() => void pickPath('loader')}>选择文件</button>
                    </div>
                    <small>Loader 不匹配或设备未进入正确模式时，后端预检会拒绝擦写。</small>
                  </label>
                )}
                <div className="imager-check-list">
                  <div>✓ 镜像路径由用户明确选择</div>
                  <div>✓ 目标介质：{board.storage}</div>
                  <div>✓ 烧录模式：{board.mode}</div>
                </div>
                {board.route === 'vendor' && board.chipVendor !== 'allwinner' && (
                  <div className="imager-route-note">
                    {board.name} 使用已保存设备配置中的专用烧录脚本和参数。请在“配置接口”中选择对应设备，向导不会覆盖其厂商参数。
                  </div>
                )}
              </div>
            )}

            {step === 3 && (
              <div className="imager-panel-stack">
                <div className="imager-page-heading">
                  <span>步骤 3 / 4</span>
                  <h2>连接并选择设备</h2>
                  <p>仅使用已保存的板卡配置，不会自动切换到其他 USB 设备。</p>
                </div>
                <div className="imager-toolbar">
                  <button
                    className="imager-primary-button"
                    onClick={() => void loadSavedDevices()}
                    disabled={busy}
                  >
                    只读扫描已保存设备
                  </button>
                  <span>{devices.length ? `已匹配 ${devices.length} 台` : '尚未扫描'}</span>
                </div>
                <div className="imager-device-list">
                  {devices.length ? (
                    devices.map((device) => (
                      <button
                        key={device.id ?? device.boardUid}
                        className={`imager-device-row ${selectedDeviceId === device.id ? 'is-selected' : ''}`}
                        onClick={() => setSelectedDeviceId(device.id)}
                      >
                        <span className="imager-device-dot" />
                        <span>
                          <strong>{device.name || device.boardUid}</strong>
                          <small>
                            {device.chipVendor} ·{' '}
                            {device.usbBinding || device.serialPort || '未绑定 USB'}
                          </small>
                        </span>
                        <span>{selectedDeviceId === device.id ? '已选择' : '选择'}</span>
                      </button>
                    ))
                  ) : (
                    <div className="imager-empty-state">
                      <strong>还没有可用的已保存设备</strong>
                      <span>请先在原有工作台中保存板卡与 USB/FEL 或 Maskrom 绑定。</span>
                      <button onClick={onOpenAdvancedWorkbench}>打开高级烧录</button>
                    </div>
                  )}
                </div>
              </div>
            )}

            {step === 4 && (
              <div className="imager-panel-stack">
                <div className="imager-page-heading">
                  <span>步骤 4 / 4</span>
                  <h2>确认并烧写系统</h2>
                  <p>开始前逐项核对，烧录任务启动后由 Rust 后端负责安全门禁和终态报告。</p>
                </div>
                <div className="imager-summary-grid">
                  <div>
                    <span>目标板</span>
                    <strong>{board.name}</strong>
                  </div>
                  <div>
                    <span>存储介质</span>
                    <strong>{board.storage}</strong>
                  </div>
                  <div>
                    <span>系统镜像</span>
                    <strong title={imagePath}>{imagePath || '未选择'}</strong>
                  </div>
                  <div>
                    <span>Loader</span>
                    <strong title={loaderPath}>
                      {board.needsLoader ? loaderPath || '未选择' : '不需要'}
                    </strong>
                  </div>
                </div>
                <label className="imager-confirm">
                  <input
                    type="checkbox"
                    checked={confirmChecked}
                    onChange={(event) => setConfirmChecked(event.target.checked)}
                  />
                  <span>
                    我已确认目标设备、镜像路径和存储介质，允许后端执行擦除、写入、校验并按配置重启。
                  </span>
                </label>
                <button
                  className="imager-danger-button"
                  onClick={() => void startFlash()}
                  disabled={!canContinue || busy}
                >
                  {busy ? '任务运行中…' : '开始烧写并校验'}
                </button>
                {taskId && (
                  <div className="imager-task-state">
                    <span>任务 {taskId}</span>
                    <strong>{taskStatus || '已提交'}</strong>
                  </div>
                )}
              </div>
            )}

            {(notice || error) && (
              <div
                className={`imager-feedback ${error ? 'is-error' : ''}`}
                role={error ? 'alert' : 'status'}
              >
                {error || notice}
              </div>
            )}
            <footer className="imager-actions">
              <span>
                {step === 1
                  ? `当前选择：${board.name}`
                  : step === 2
                    ? '镜像和 Loader 只在本地使用'
                    : step === 3
                      ? '设备选择由已保存配置决定'
                      : '写入、校验、重启、启动验证分开记录'}
              </span>
              <div>
                <button
                  className="imager-secondary-button"
                  onClick={() => setStep((current) => Math.max(1, current - 1) as BasicStep)}
                  disabled={step === 1 || busy}
                >
                  上一步
                </button>
                {step < 4 && (
                  <button
                    className="imager-primary-button"
                    onClick={() => setStep((current) => Math.min(4, current + 1) as BasicStep)}
                    disabled={!canContinue}
                  >
                    下一步
                  </button>
                )}
              </div>
            </footer>
          </main>
      </div>
    </section>
  );
};
