import React, {
  useState,
  useCallback,
  useEffect,
  useRef,
  useMemo,
  useSyncExternalStore,
} from 'react';
import { sharedDevice } from '../SPITool/sharedDevice';
import { loadOledFont } from '../../Library/SSD1306';
import { useModalDialog } from '../../Hooks';
import { useSpiEngine, type DisplayCommandRow, type SpiConfig } from './hooks/useSpiEngine';
import type { ScreenDisplayType } from './Components/tabs/common';
import { LeftPanel } from './Components/LeftPanel';
import { MiddlePanel, type ContentTab } from './Components/MiddlePanel';
import { RightPanel, type LogEntry } from './Components/RightPanel';
import { canvasToRGB565Bytes, getContentBBox } from './Components/tabs/rgb565';
import './SPIDisplayTool.css';

/** 分辨率预设条目类型。 */
export interface ResolutionPreset {
  /** 选项 key，'custom' 为可自定义输入 */
  key: string;
  /** 用户可见的标签 */
  label: string;
  /** 该预设的逻辑宽度（像素） */
  width: number;
  /** 该预设的逻辑高度（像素，需为 8 的倍数 —— SSD1306 page 整除要求） */
  height: number;
}

/** 内置分辨率预设 —— 仅列出常见 SPI 单色/彩屏分辨率，不绑定特定型号。
 *  比如同样 128×64 可能是 SSD1306 / SH1106 / UC1701 等多个驱动 IC，
 *  这里只关心 Canvas 大小，初始化命令由用户的"初始化命令表"提供。 */
const RESOLUTION_PRESETS: ResolutionPreset[] = [
  { key: '128x32', label: '128 × 32', width: 128, height: 32 },
  { key: '128x64', label: '128 × 64', width: 128, height: 64 },
  { key: '128x128', label: '128 × 128', width: 128, height: 128 },
  { key: '240x240', label: '240 × 240', width: 240, height: 240 },
  { key: '240x320', label: '240 × 320', width: 240, height: 320 },
  { key: '320x240', label: '320 × 240', width: 320, height: 240 },
  { key: '320x480', label: '320 × 480', width: 320, height: 480 },
  { key: 'custom', label: '自定义', width: 128, height: 64 },
];

/**
 * SSD1306 (UG-2864TMBEG01) 默认初始化命令表 —— 对照数据手册 SSD1306 Rev 1.1 + 参考代码
 * spi_oled.c (ReferenceCode/04_oled_use_spidev_ok/) 校正后的版本，可点亮屏幕不花屏。
 *
 * 关键值的说明（来自数据手册第 10.1 节）：
 *   AE       — 显示关闭
 *   A8 3F    — MUX 1/64（128×64 屏必须 0x3F；0x1F 是 128×32 屏才用）
 *   D3 00    — 显示偏移 = 0（10.1.15）
 *   40       — 显示起始行 = 0（10.1.6）
 *   8D 14    — 电荷泵开启（使能内部 7-9V 升压，VCC 由内部生成）
 *   A1       — Segment Re-map：列 127→SEG0（与 C8 配合实现 180° 旋转，适配大多数 0.96" 模块）
 *   C8       — COM 输出扫描方向反向（COM[N-1]→COM0）
 *   DA 12    — COM Pins 硬件配置：交替（Alternative）+ 不左右翻转（标准 128×64 模块）
 *   81 CF    — 对比度（参考代码用 CF 更亮；0x7F 会偏暗）
 *   D9 F1    — Pre-charge Period（关键！缺失会导致驱动不足产生花屏；参考代码 F1）
 *   DB 30    — VCOMH 电平 0.83×Vcc（关键！缺失会导致 OLED 闪烁和不稳定显示）
 *   A4       — 输出从 GDDRAM 读（不是强制全亮 A5）
 *   A6       — 正常显示（不反相）
 *   D5 80    — 时钟分频/振荡频率：bit[3:0]=8 是复位默认值，
 *              过去用 D5 00 会让振荡器频率降到最低 → 显示时序乱 → 花屏。改成 80 修复
 *   AF       — 显示开启
 *
 * 顺序参考 spi_oled.c oled_init() 实际能正常点亮 0.96" 黄蓝屏的命令序列。
 */
const DEFAULT_SSD1306_INIT_ROWS: Omit<DisplayCommandRow, 'id'>[] = [
  { type: 'delay', data: '100000' }, // 上电等待
  { type: 'cmd', data: 'AE' }, // 显示关闭
  { type: 'cmd', data: '00' }, // 列低地址 = 0
  { type: 'cmd', data: '10' }, // 列高地址 = 0
  { type: 'cmd', data: '40' }, // 起始行 = 0
  { type: 'cmd', data: 'B0' }, // 页地址 = 0
  { type: 'cmd', data: '81 CF' }, // 对比度（0xCF 亮度合适，过暗用 0x7F）
  { type: 'cmd', data: 'A1' }, // Segment Re-map 列翻转
  { type: 'cmd', data: 'A6' }, // 正常显示（非反相）
  { type: 'cmd', data: 'A8 3F' }, // MUX = 1/64（128×64 屏必须）
  { type: 'cmd', data: 'C8' }, // COM 扫描方向反向
  { type: 'cmd', data: 'D3 00' }, // 显示偏移 = 0
  { type: 'cmd', data: 'D5 80' }, // 时钟分频/振荡频率（复位默认 0x80，原 0x00 会花屏！）
  { type: 'cmd', data: 'D9 F1' }, // Pre-charge Period（关键，缺失会驱动不足）
  { type: 'cmd', data: 'DA 12' }, // COM Pins 硬件配置（交替）
  { type: 'cmd', data: 'DB 30' }, // VCOMH 电平（关键，缺失会闪烁/花屏）
  { type: 'cmd', data: '8D 14' }, // 电荷泵开启

  // 清屏：逐页（B0~B7）写 64+64 字节 0（每页拆两次，避免 CH347 缓冲溢出）。
  // 实测一次性 128 字节在连续发送 8 次时后面会丢包，64 字节更稳。
  // 逐页写，每页拆两半：Bx+00+10 → 64×00 → 64×00
  { type: 'cmd', data: 'B0' },
  { type: 'cmd', data: '00' },
  { type: 'cmd', data: '10' },
  {
    type: 'data',
    data: '00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00',
  },
  {
    type: 'data',
    data: '00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00',
  },
  { type: 'cmd', data: 'B1' },
  { type: 'cmd', data: '00' },
  { type: 'cmd', data: '10' },
  {
    type: 'data',
    data: '00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00',
  },
  {
    type: 'data',
    data: '00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00',
  },
  { type: 'cmd', data: 'B2' },
  { type: 'cmd', data: '00' },
  { type: 'cmd', data: '10' },
  {
    type: 'data',
    data: '00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00',
  },
  {
    type: 'data',
    data: '00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00',
  },
  { type: 'cmd', data: 'B3' },
  { type: 'cmd', data: '00' },
  { type: 'cmd', data: '10' },
  {
    type: 'data',
    data: '00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00',
  },
  {
    type: 'data',
    data: '00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00',
  },
  { type: 'cmd', data: 'B4' },
  { type: 'cmd', data: '00' },
  { type: 'cmd', data: '10' },
  {
    type: 'data',
    data: '00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00',
  },
  {
    type: 'data',
    data: '00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00',
  },
  { type: 'cmd', data: 'B5' },
  { type: 'cmd', data: '00' },
  { type: 'cmd', data: '10' },
  {
    type: 'data',
    data: '00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00',
  },
  {
    type: 'data',
    data: '00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00',
  },
  { type: 'cmd', data: 'B6' },
  { type: 'cmd', data: '00' },
  { type: 'cmd', data: '10' },
  {
    type: 'data',
    data: '00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00',
  },
  {
    type: 'data',
    data: '00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00',
  },
  { type: 'cmd', data: 'B7' },
  { type: 'cmd', data: '00' },
  { type: 'cmd', data: '10' },
  {
    type: 'data',
    data: '00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00',
  },
  {
    type: 'data',
    data: '00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00',
  },

  { type: 'cmd', data: 'AF' }, // 显示开启（此时 GDDRAM 已全清 → 黑屏，不花屏）
  { type: 'delay', data: '100000' }, // 数据手册 tAF：AFh 后 SEG/COM 需 100ms 才完全 ON。延时不足会导致首次发送显示不稳（多发几次才正常）
];

/** localStorage 中保存用户自定义屏幕预设的 key。 */
const SCREEN_PRESET_STORAGE_KEY = 'spi-display-screen-presets';

/** 屏幕预设的存储结构：屏型号 → 初始化命令表（不含 id，加载时重新分配）。 */
export interface SavedScreenPreset {
  /** 用户起的名字，下拉里显示这个 */
  name: string;
  /** 可选的分辨率提示（仅用于切换时同步画布尺寸；不强制） */
  width?: number;
  height?: number;
  /** 显示类型（用户预设可选；缺省按 monochrome-page 处理） */
  displayType?: ScreenDisplayType;
  /** 初始化命令表，持久化时去掉 id */
  rows: Array<Omit<DisplayCommandRow, 'id'>>;
}

const BUILTIN_SCREEN_PRESETS: Array<{
  key: string;
  name: string;
  width: number;
  height: number;
  displayType: ScreenDisplayType;
  rows: Omit<DisplayCommandRow, 'id'>[];
}> = [
  {
    key: '__builtin:ssd1306-128x64',
    name: 'SSD1306 OLED 128×64',
    width: 128,
    height: 64,
    displayType: 'monochrome-page',
    rows: DEFAULT_SSD1306_INIT_ROWS,
  },
];

/** 从 localStorage 读取用户屏幕预设列表。 */
function loadScreenPresets(): SavedScreenPreset[] {
  try {
    const raw = localStorage.getItem(SCREEN_PRESET_STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(
      (p): p is SavedScreenPreset => p && typeof p.name === 'string' && Array.isArray(p.rows)
    );
  } catch {
    return [];
  }
}

/** 持久化用户屏幕预设列表。 */
function saveScreenPresets(list: SavedScreenPreset[]): void {
  try {
    localStorage.setItem(SCREEN_PRESET_STORAGE_KEY, JSON.stringify(list));
  } catch {
    /* quota or disabled */
  }
}

/** 给当前时间生成 HH:MM:SS.mmm 字符串。 */
function nowTime(): string {
  const d = new Date();
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}:${String(d.getSeconds()).padStart(2, '0')}.${String(d.getMilliseconds()).padStart(3, '0')}`;
}

/**
 * SPI 点屏调试工具主组件。
 *
 * 三栏布局：
 *   - 左：硬件参数 + 初始化命令表 + 操作按钮
 *   - 中：4 Tab 显示内容编辑（文本/图片/视频/绘制）
 *   - 右：LCD 模拟预览 + 日志控制台
 *
 * 与 SPI Tool 共享 sharedDevice 单例（同一 CH347 设备状态），
 * 所有 SPI/GPIO 调用通过 useSpiEngine hook 封装到后端 ch347_* 命令。
 */
export const SPIDisplayTool: React.FC = () => {
  const deviceState = useSyncExternalStore(sharedDevice.subscribe, sharedDevice.getState);

  // 监听自动断开的 effect 在 log 定义之后（log 是依赖）

  // 自定义模态弹窗 —— 取代浏览器原生 prompt/confirm（避免显示 "localhost:3030"）。
  // modalNode 在 JSX 最后渲染；showPrompt/showConfirm/showAlert 是 async API
  const { showPrompt, showConfirm, showAlert, modalNode } = useModalDialog();

  // ─── SPI 配置 ─────────────────────────────────────
  // 全部参数都在 UI 中暴露给用户（参考 SPI Tool），与 useSpiEngine 的 SpiConfig 字段一一对应
  const [spiMode, setSpiMode] = useState(0);
  const [frequencyHz, setFrequencyHz] = useState(8_000_000);
  const [spiSpeed, setSpiSpeed] = useState('7.5'); // 下拉值（MHz 字符串），8MHz 最接近 7.5MHz 档
  const [spiCs, setSpiCs] = useState('0');
  const [spiBits, setSpiBits] = useState('8');
  const [spiBitOrder, setSpiBitOrder] = useState('1'); // 1=MSB, 0=LSB
  // 用 useMemo 保证 spiCfg 引用稳定，避免 useCallback 依赖每次都失效
  const spiCfg: SpiConfig = useMemo(
    () => ({
      mode: spiMode,
      frequencyHz,
      cs: parseInt(spiCs) || 0,
      dataBits: (parseInt(spiBits) === 16 ? 16 : 8) as 8 | 16,
      byteOrder: (parseInt(spiBitOrder) === 0 ? 0 : 1) as 0 | 1,
    }),
    [spiMode, frequencyHz, spiCs, spiBits, spiBitOrder]
  );

  // ─── 分辨率 ─────────────────────────────────────
  // 默认 128×64（最常见的 SSD1306/SH1106 OLED 尺寸）
  const [resolutionKey, setResolutionKey] = useState('128x64');
  const [width, setWidth] = useState(128);
  const [height, setHeight] = useState(64);

  /** 切换分辨率预设时同步宽高（不再绑定推荐频率 —— 用户自己选 SPI Speed）。 */
  const handleResolutionPresetChange = useCallback((key: string) => {
    setResolutionKey(key);
    const preset = RESOLUTION_PRESETS.find((r) => r.key === key);
    if (preset && key !== 'custom') {
      setWidth(preset.width);
      setHeight(preset.height);
    }
  }, []);

  // 把高度强制对齐到 8 的倍数（SSD1306 page 整除要求）
  const heightAligned = Math.ceil(height / 8) * 8;

  // ─── 命令表 + 屏幕预设 state ──────────────────
  // state 在这里，callback 在 log 定义之后（log 是依赖）。
  // 命令表默认留空 —— 用户需要时从屏幕预设下拉里点 Load 加载内置 SSD1306 预设
  const [cmdRows, setCmdRows] = useState<DisplayCommandRow[]>([]);
  // 屏幕预设系统：内置 + 用户保存。持久化到 localStorage，与 SPI Tool 的预设系统风格一致。
  const [userScreenPresets, setUserScreenPresets] = useState<SavedScreenPreset[]>(() =>
    loadScreenPresets()
  );
  const [selectedScreenPreset, setSelectedScreenPreset] = useState<string>(
    BUILTIN_SCREEN_PRESETS[0]?.key ?? ''
  );
  // 当前预设的显示类型（决定推送通路：monochrome-page=pushFramebuffer, rgb565=pushRgbRegion）
  const [displayType, setDisplayType] = useState<ScreenDisplayType>(
    BUILTIN_SCREEN_PRESETS[0]?.displayType ?? 'monochrome-page'
  );
  // 推送范围：full=整屏，partial=只推内容外接矩形（仅 rgb565 用；ST7796 整屏 307KB≈100s 太慢，默认局刷）
  const [refreshMode, setRefreshMode] = useState<'full' | 'partial'>('partial');

  // ─── Tab / 预览 / 日志 ──────────────────────
  const [activeTab, setActiveTab] = useState<ContentTab>('text');
  const [previewCanvas, setPreviewCanvas] = useState<HTMLCanvasElement | null>(null);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [busy, setBusy] = useState(false);

  /** 追加日志。autoLevel 根据消息内容自动判断（"error" 字样 → error）。 */
  const log = useCallback((message: string, isError = false) => {
    let level: LogEntry['level'] = 'info';
    if (isError || /error|fail|失败|错误/i.test(message)) level = 'error';
    else if (/ok|success|成功|completed|done/i.test(message)) level = 'success';
    setLogs((prev) => {
      const next = [...prev, { time: nowTime(), message, level }];
      if (next.length > 200) next.shift();
      return next;
    });
  }, []);

  // 监听自动断开：设备被拔掉时 sharedDevice 会把 autoDisconnected 置 true + online 置 false。
  // 用 ref 记录上一次状态，避免重复 log。
  const lastAutoDisconnected = useRef(false);
  useEffect(() => {
    if (deviceState.autoDisconnected && !lastAutoDisconnected.current) {
      log('⚠ 设备已断开连接（USB 被拔出或设备消失）', true);
    }
    lastAutoDisconnected.current = deviceState.autoDisconnected;
  }, [deviceState.autoDisconnected, log]);

  // ─── SPI 引擎 ───────────────────────────────
  const engine = useSpiEngine({
    deviceIndex: deviceState.deviceIndex ?? 0,
    connected: deviceState.online,
    log,
  });

  // ─── 屏幕预设回调 ─────────────────────────
  // 这些 callback 依赖 log，所以必须放在 log 定义之后

  /** 应用屏幕预设（内置或用户）到命令表。 */
  const applyScreenPreset = useCallback(
    (key: string) => {
      if (!key) return;
      let rows: Omit<DisplayCommandRow, 'id'>[] | undefined;
      let presetName = '';
      let presetWidth: number | undefined;
      let presetHeight: number | undefined;
      let presetDisplayType: ScreenDisplayType = 'monochrome-page';
      if (key.startsWith('__builtin:')) {
        const b = BUILTIN_SCREEN_PRESETS.find((p) => p.key === key);
        if (!b) return;
        rows = b.rows;
        presetName = b.name;
        presetWidth = b.width;
        presetHeight = b.height;
        presetDisplayType = b.displayType;
      } else if (key.startsWith('user:')) {
        const name = key.slice('user:'.length);
        const p = userScreenPresets.find((u) => u.name === name);
        if (!p) {
          log(`Preset not found: ${name}`, true);
          return;
        }
        rows = p.rows;
        presetName = name;
        presetWidth = p.width;
        presetHeight = p.height;
        presetDisplayType = p.displayType ?? 'monochrome-page';
      }
      if (!rows) return;
      const newRows: DisplayCommandRow[] = rows.map((r, i) => ({ ...r, id: i + 1 }));
      setCmdRows(newRows);
      setDisplayType(presetDisplayType);
      // 如果预设指定了分辨率，同步改画布大小
      if (presetWidth && presetHeight) {
        const w = presetWidth,
          h = presetHeight;
        setWidth(w);
        setHeight(h);
        // 自动匹配分辨率下拉：若有预设 key 匹配就选它，否则选 custom
        const match = RESOLUTION_PRESETS.find((r) => r.width === w && r.height === h);
        setResolutionKey(match ? match.key : 'custom');
      }
      log(`屏幕预设加载: ${presetName} (${newRows.length} 行)`);
    },
    [userScreenPresets, log]
  );

  /** 把当前命令表另存为用户屏幕预设。 */
  const saveCurrentAsScreenPreset = useCallback(async () => {
    if (!cmdRows.length) {
      log('命令表为空', true);
      return;
    }
    const name = (
      await showPrompt(
        '另存为屏幕预设',
        `屏幕预设 ${userScreenPresets.length + 1}`,
        `将下方 ${cmdRows.length} 行命令保存为可重用的屏幕预设`
      )
    )?.trim();
    if (!name) return;
    const exists = userScreenPresets.some((p) => p.name === name);
    if (
      exists &&
      !(await showConfirm(`预设 "${name}" 已存在`, '是否覆盖原有内容？', {
        okText: '覆盖',
        okDanger: true,
      }))
    )
      return;
    const stripped: SavedScreenPreset = {
      name,
      width,
      height,
      rows: cmdRows.map(({ type, data }) => ({ type, data })),
    };
    const next = exists
      ? userScreenPresets.map((p) => (p.name === name ? stripped : p))
      : [...userScreenPresets, stripped];
    setUserScreenPresets(next);
    saveScreenPresets(next);
    setSelectedScreenPreset(`user:${name}`);
    log(`屏幕预设已保存: ${name} (${stripped.rows.length} 行)`);
  }, [cmdRows, userScreenPresets, width, height, log, showPrompt, showConfirm]);

  /** 删除当前选中的用户屏幕预设。 */
  const deleteScreenPreset = useCallback(async () => {
    if (!selectedScreenPreset.startsWith('user:')) {
      log('内置预设不能删除', true);
      return;
    }
    const name = selectedScreenPreset.slice('user:'.length);
    if (
      !(await showConfirm(`删除屏幕预设 "${name}"`, '此操作无法撤销。', {
        okText: '删除',
        okDanger: true,
      }))
    )
      return;
    const next = userScreenPresets.filter((p) => p.name !== name);
    setUserScreenPresets(next);
    saveScreenPresets(next);
    setSelectedScreenPreset(
      BUILTIN_SCREEN_PRESETS[0]?.key ?? (next[0] ? `user:${next[0].name}` : '')
    );
    log(`屏幕预设已删除: ${name}`);
  }, [selectedScreenPreset, userScreenPresets, log, showConfirm]);

  // 设备连接由侧边栏的 DeviceConnectButton 统一管理（sharedDevice 单例 + 自动连接），
  // useSpiEngine 内部已监听 connected 变化在断开时复位 configuredRef，因此这里不再需要
  // 手动 markUnconfigured。仅监听 online 变化补一条日志（自动断开由上面的
  // autoDisconnected 监听器单独提示"USB 被拔出"）。
  const prevOnline = useRef(false);
  useEffect(() => {
    if (!prevOnline.current && deviceState.online) {
      log(`已连接 Lynx Power 设备 #${deviceState.deviceIndex}`);
    } else if (prevOnline.current && !deviceState.online && !deviceState.autoDisconnected) {
      log('已断开 Lynx Power 设备');
    }
    prevOnline.current = deviceState.online;
  }, [deviceState.online, deviceState.deviceIndex, deviceState.autoDisconnected, log]);

  // 预加载 OLED TTF 字体（fire-and-forget）
  useEffect(() => {
    loadOledFont().catch(() => {
      /* ignore */
    });
  }, []);

  // 当任一 SPI 参数变化时，标记需要重新 init（下次操作会自动 init）。
  // 跟 SPI Tool 一致：mode/speed/cs/bits/bitOrder 全部纳入监听
  const cfgRef = useRef<string>('');
  useEffect(() => {
    const sig = `${spiMode}|${frequencyHz}|${spiCs}|${spiBits}|${spiBitOrder}`;
    if (cfgRef.current && cfgRef.current !== sig) {
      engine.markUnconfigured();
      log('SPI 配置已修改，下次操作时会重新初始化');
    }
    cfgRef.current = sig;
  }, [spiMode, frequencyHz, spiCs, spiBits, spiBitOrder, engine, log]);

  // ─── 操作处理 ───────────────────────────────

  /** 手动 Init SPI —— 把当前 spiCfg 立即下发到 CH347。改完参数立刻按可看到日志反馈。 */
  const handleInitSpi = useCallback(async () => {
    if (busy) return;
    setBusy(true);
    try {
      await engine.initSpi(spiCfg);
    } finally {
      setBusy(false);
    }
  }, [engine, spiCfg, busy]);

  const handleSendInit = useCallback(async () => {
    if (busy) return;
    setBusy(true);
    log(`开始执行初始化序列 (${cmdRows.length} 行)...`);
    try {
      await engine.runCommandTable(cmdRows, spiCfg);
    } finally {
      setBusy(false);
    }
  }, [cmdRows, spiCfg, engine, log, busy]);

  const handleReset = useCallback(async () => {
    if (busy) return;
    setBusy(true);
    try {
      await engine.hardReset();
    } catch (e) {
      log(`复位失败：${(e as Error).message}`, true);
    } finally {
      setBusy(false);
    }
  }, [engine, log, busy]);

  /**
   * Tab 触发的发送回调 —— 接到 1-bit 字节后通过 pushFramebuffer 推到屏幕，
   * 同时把 canvas 拷给 LCD 预览区做镜像显示。
   */
  /**
   * Tab 触发的发送回调 —— 接到 1-bit 字节后通过 pushFramebuffer 推到屏幕，
   * 同时把 canvas 拷给 LCD 预览区做镜像显示。
   *
   * 这里用 busyRef 做并发锁而不是 setBusy/state —— 视频 Tab 每帧调这个函数，
   * 用 React state 会让父组件每帧重渲染，左栏的"设置/发送初始化/复位"三个
   * 按钮 disabled 状态高频切换 → 闪烁。
   * 真正"用户感知"的 busy 状态（点发送初始化/复位时）仍用 setBusy。
   */
  const sendingRef = useRef(false);
  const handleTabSend = useCallback(
    async (info: {
      bytes: number[];
      canvas: HTMLCanvasElement;
      description: string;
      bgColor?: string;
    }) => {
      if (sendingRef.current) return;
      sendingRef.current = true;
      try {
        // 镜像到预览：复制 canvas（避免后续修改影响预览）
        const mirror = document.createElement('canvas');
        mirror.width = info.canvas.width;
        mirror.height = info.canvas.height;
        mirror.getContext('2d')!.drawImage(info.canvas, 0, 0);
        setPreviewCanvas(mirror);

        if (!deviceState.online) {
          log(`(未连接，仅本地预览) ${info.description}`);
          return;
        }

        if (displayType === 'rgb565') {
          // ST7796 等 RGB 屏：canvas -> RGB565 -> pushRgbRegion
          const W = info.canvas.width,
            H = info.canvas.height;
          if (refreshMode === 'partial') {
            // 局刷：只推内容外接矩形（避开整屏 307KB 慢推）
            const bbox = getContentBBox(info.canvas, info.bgColor ?? '#000000');
            if (!bbox) {
              log(`${info.description} -> 内容为空，跳过`);
              return;
            }
            const rgb = canvasToRGB565Bytes(info.canvas, bbox.x, bbox.y, bbox.w, bbox.h);
            log(
              `${info.description} -> 局刷 ${bbox.w}×${bbox.h} @ (${bbox.x},${bbox.y}), ${rgb.length}B...`
            );
            await engine.pushRgbRegion(rgb, bbox.x, bbox.y, bbox.w, bbox.h, spiCfg);
          } else {
            // 全刷：整屏（307KB，慢）
            const rgb = canvasToRGB565Bytes(info.canvas, 0, 0, W, H);
            log(
              `${info.description} -> 全刷 ${W}×${H}, ${rgb.length}B（预计 ${Math.round(rgb.length / 3000)}s）...`
            );
            await engine.pushRgbRegion(rgb, 0, 0, W, H, spiCfg);
          }
        } else {
          // SSD1306 等单色屏：1-bit page 推送（原逻辑，不动）
          log(`${info.description} -> 推送 ${info.bytes.length}B 显存...`);
          const pageCount = info.canvas.height / 8;
          await engine.pushFramebuffer(info.bytes, info.canvas.width, pageCount, spiCfg);
        }
      } finally {
        sendingRef.current = false;
      }
    },
    [deviceState.online, displayType, refreshMode, engine, spiCfg, log]
  );

  return (
    <div className="spi-display-tool">
      <LeftPanel
        online={deviceState.online}
        frequencyHz={frequencyHz}
        onFrequencyChange={setFrequencyHz}
        spiSpeed={spiSpeed}
        onSpiSpeedChange={setSpiSpeed}
        spiMode={spiMode}
        onSpiModeChange={setSpiMode}
        spiCs={spiCs}
        onSpiCsChange={setSpiCs}
        spiBits={spiBits}
        onSpiBitsChange={setSpiBits}
        spiBitOrder={spiBitOrder}
        onSpiBitOrderChange={setSpiBitOrder}
        onInitSpi={handleInitSpi}
        width={width}
        height={height}
        rows={cmdRows}
        onRowsChange={setCmdRows}
        builtinScreenPresets={BUILTIN_SCREEN_PRESETS}
        userScreenPresets={userScreenPresets}
        selectedScreenPreset={selectedScreenPreset}
        onSelectedScreenPresetChange={setSelectedScreenPreset}
        onApplyScreenPreset={applyScreenPreset}
        onSaveAsScreenPreset={saveCurrentAsScreenPreset}
        onDeleteScreenPreset={deleteScreenPreset}
        showPrompt={showPrompt}
        showConfirm={showConfirm}
        showAlert={showAlert}
        onSendInit={handleSendInit}
        onReset={handleReset}
        busy={busy}
      />

      <MiddlePanel
        activeTab={activeTab}
        onTabChange={setActiveTab}
        width={width}
        height={heightAligned}
        onSend={handleTabSend}
        busy={busy}
        displayType={displayType}
        refreshMode={refreshMode}
        onRefreshModeChange={setRefreshMode}
      />

      <RightPanel
        online={deviceState.online}
        width={width}
        height={heightAligned}
        previewCanvas={previewCanvas}
        resolutionPreset={resolutionKey}
        resolutionPresets={RESOLUTION_PRESETS}
        onResolutionPresetChange={handleResolutionPresetChange}
        onWidthChange={setWidth}
        onHeightChange={setHeight}
        logs={logs}
        onClearLogs={() => setLogs([])}
      />

      {/* 自定义模态弹窗 —— useModalDialog 内部管理状态，挂这一个节点即可 */}
      {modalNode}
    </div>
  );
};
