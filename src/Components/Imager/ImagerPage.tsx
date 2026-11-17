import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { invokeCommand, subscribeEvent, type WorkbenchDevice } from '../../Platform/IPC';
import './ImagerPage.css';

type ImagerWorkspace = 'basic' | 'advanced' | 'ai';
type BasicStep = 1 | 2 | 3 | 4;

type BoardPreset = {
  id: string;
  name: string;
  chipVendor: WorkbenchDevice['chipVendor'];
  chip: string;
  storage: string;
  mode: string;
  route: 'vendor' | 'mainline_nand' | 'mainline_emmc' | 'mainline_spinor';
  needsLoader: boolean;
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
    route: 'mainline_emmc',
    needsLoader: true,
  },
];

const STEPS: Array<{ id: BasicStep; label: string; hint: string }> = [
  { id: 1, label: '选择板子', hint: '确认芯片、介质和连接模式' },
  { id: 2, label: '选择系统', hint: '选择本地镜像和 Loader' },
  { id: 3, label: '配置接口', hint: '绑定已保存的目标设备' },
  { id: 4, label: '烧写系统', hint: '预检、确认、写入和校验' },
];

export interface ImagerPageProps {
  onOpenAdvanced: () => void;
  onOpenAi: () => void;
  onOpenLegacyWorkbench: () => void;
}

function deviceMatchesBoard(device: WorkbenchDevice, board: BoardPreset) {
  return device.chipVendor === board.chipVendor;
}

export const ImagerPage: React.FC<ImagerPageProps> = ({
  onOpenAdvanced,
  onOpenAi,
  onOpenLegacyWorkbench,
}) => {
  const [workspace, setWorkspace] = useState<ImagerWorkspace>('basic');
  const [step, setStep] = useState<BasicStep>(1);
  const [boardId, setBoardId] = useState('f101');
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
      const started = await invokeCommand('workbench_start_flash', {
        deviceId: selectedDeviceId,
        firmwarePath: imagePath,
        options: {
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
  }, [board.route, confirmChecked, imagePath, loaderPath, selectedDeviceId]);

  const openWorkspace = (next: ImagerWorkspace) => {
    setWorkspace(next);
    if (next === 'advanced') onOpenAdvanced();
    if (next === 'ai') onOpenAi();
  };

  return (
    <section className="imager-page" aria-label="LYNX Imager">
      <header className="imager-header">
        <div>
          <div className="imager-kicker">LYNX IMAGER</div>
          <h1>系统镜像烧录</h1>
          <p>把适配的系统镜像安全写入你的开发板。</p>
        </div>
        <div className="imager-workspaces" role="tablist" aria-label="工作区">
          <button
            className={workspace === 'basic' ? 'is-active' : ''}
            onClick={() => setWorkspace('basic')}
            role="tab"
            aria-selected={workspace === 'basic'}
          >
            基础烧录
          </button>
          <button
            className={workspace === 'advanced' ? 'is-active' : ''}
            onClick={() => openWorkspace('advanced')}
            role="tab"
            aria-selected={workspace === 'advanced'}
          >
            高级功能
          </button>
          <button
            className={workspace === 'ai' ? 'is-active' : ''}
            onClick={() => openWorkspace('ai')}
            role="tab"
            aria-selected={workspace === 'ai'}
          >
            连接 AI
          </button>
        </div>
      </header>

      {workspace === 'basic' ? (
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
                <div className="imager-board-grid">
                  {BOARD_PRESETS.map((candidate) => (
                    <button
                      key={candidate.id}
                      className={`imager-board-card ${board.id === candidate.id ? 'is-selected' : ''}`}
                      onClick={() => {
                        setBoardId(candidate.id);
                        setStep(1);
                        setDevices([]);
                        setSelectedDeviceId(null);
                      }}
                    >
                      <span className="imager-board-mark">
                        {candidate.chipVendor === 'rockchip' ? 'RK' : 'AW'}
                      </span>
                      <span className="imager-board-name">{candidate.name}</span>
                      <span className="imager-board-meta">
                        {candidate.chip} · {candidate.storage}
                      </span>
                      <span className="imager-board-meta">{candidate.mode}</span>
                      {board.id === candidate.id && (
                        <span className="imager-selected-label">已选择</span>
                      )}
                    </button>
                  ))}
                </div>
                <div className="imager-info-card">
                  <strong>{board.name}</strong>
                  <span>
                    {board.needsLoader
                      ? '该路线需要匹配板型的 RAM Loader。'
                      : '可直接使用厂家镜像路线。'}
                  </span>
                  <button onClick={onOpenLegacyWorkbench}>查看原有工作台</button>
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
                        placeholder="F101: fes1.fex；RK: 对应 SPL/Loader"
                      />
                      <button onClick={() => void pickPath('loader')}>选择文件</button>
                    </div>
                    <small>Loader 不匹配时，后端预检会拒绝擦写。</small>
                  </label>
                )}
                <div className="imager-check-list">
                  <div>✓ 镜像路径由用户明确选择</div>
                  <div>✓ 目标介质：{board.storage}</div>
                  <div>✓ 烧录模式：{board.mode}</div>
                </div>
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
                      <button onClick={onOpenLegacyWorkbench}>打开设备设置</button>
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
      ) : (
        <div className="imager-special-workspace">
          <div className="imager-special-hero">
            <span className="imager-kicker">
              {workspace === 'advanced' ? 'ADVANCED WORKSPACE' : 'AI CONNECTOR'}
            </span>
            <h2>{workspace === 'advanced' ? '专业烧录工作台' : '连接 AI 工具'}</h2>
            <p>
              {workspace === 'advanced'
                ? '继续使用现有 OpenixCLI、FEL/FES、SPI NOR、NAND 和 eMMC 后端；这里负责更细的路线、Loader、校验和结束动作。'
                : 'AI 只负责提出请求，LYNX 负责设备作用域、策略、确认和确定性结果。'}
            </p>
            <div className="imager-special-actions">
              <button
                className="imager-primary-button"
                onClick={workspace === 'advanced' ? onOpenAdvanced : onOpenAi}
              >
                {workspace === 'advanced' ? '打开专业工作台' : '打开 AI 配置中心'}
              </button>
              <button className="imager-secondary-button" onClick={() => setWorkspace('basic')}>
                返回基础烧录
              </button>
            </div>
          </div>
          <div className="imager-special-grid">
            <div>
              <span>现有后端</span>
              <strong>OpenixCLI / Tauri IPC</strong>
              <small>不重写烧录协议，所有高风险动作仍经过后端门禁。</small>
            </div>
            <div>
              <span>安全原则</span>
              <strong>设备作用域 + 明确确认</strong>
              <small>AI 请求不能绕过板卡绑定、预检和人工确认。</small>
            </div>
            <div>
              <span>结果报告</span>
              <strong>写入 · 校验 · 重启 · 启动</strong>
              <small>四个状态分别记录，不用 100% 进度冒充启动成功。</small>
            </div>
          </div>
        </div>
      )}
    </section>
  );
};
