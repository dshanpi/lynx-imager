import React, { useEffect, useRef, useState } from 'react';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import { faDisplay, faMicrochip, faTrash, faCopy } from '@fortawesome/free-solid-svg-icons';
import type { ResolutionPreset } from '../SPIDisplayTool';

/** 日志条目 — 与父组件 SPIDisplayTool 共用类型。 */
export interface LogEntry {
  time: string;
  message: string;
  level: 'info' | 'success' | 'error' | 'muted';
}

interface RightPanelProps {
  // LCD 预览相关
  online: boolean;
  width: number;
  height: number;
  /** 上次发送的画布（用来在预览区镜像显示）— null 表示尚未发送过 */
  previewCanvas: HTMLCanvasElement | null;

  // 分辨率选择 —— 仅画布尺寸（不耦合屏幕型号），放在 LCD 预览旁边更直观：
  // 用户改了分辨率立刻能看到预览区尺寸跟着变
  resolutionPreset: string;
  resolutionPresets: ResolutionPreset[];
  onResolutionPresetChange: (key: string) => void;
  onWidthChange: (w: number) => void;
  onHeightChange: (h: number) => void;

  // 日志
  logs: LogEntry[];
  onClearLogs: () => void;
}

/**
 * 右侧面板：LCD 模拟预览 + 分辨率选择 + 日志控制台。
 *
 * LCD 预览区会把上次发送的画布按比例镜像绘制，并在右下角加水印。
 * 没有发送过任何内容时显示芯片图标和提示文字。
 *
 * 分辨率控件就放在预览正下方 —— 改尺寸即改预览，逻辑上更连贯
 * （硬件参数面板只放与 CH347/SPI 总线相关的参数）。
 */
export const RightPanel: React.FC<RightPanelProps> = ({
  online,
  width,
  height,
  previewCanvas,
  resolutionPreset,
  resolutionPresets,
  onResolutionPresetChange,
  onWidthChange,
  onHeightChange,
  logs,
  onClearLogs,
}) => {
  const previewRef = useRef<HTMLCanvasElement | null>(null);
  const logEndRef = useRef<HTMLDivElement | null>(null);

  // 自定义分辨率的宽高输入用本地字符串 state，避免受控于对齐后的值导致"输入与显示不一致"：
  //   - width 是原始值（无对齐），可直接用 width 同步；
  //   - height 实际显示的是 heightAligned（向上对齐到 8 的倍数），若直接 value={heightAligned}
  //     会导致用户输入 100 时显示 104、清空时 || 64 跳变等问题。
  // 改为本地字符串：输入过程中只改字符串，失焦/回车时才解析提交。外部 preset 切换时同步。
  const [wText, setWText] = useState(String(width));
  const [hText, setHText] = useState(String(height));

  // 外部 width/height 变化（切换预设、加载屏幕预设）时，同步本地输入框文本
  useEffect(() => {
    setWText(String(width));
  }, [width]);
  useEffect(() => {
    setHText(String(height));
  }, [height]);

  /** 提交宽度：解析字符串为数字，非法/超出范围则回退到当前值 */
  const commitWidth = () => {
    const n = parseInt(wText, 10);
    if (!isNaN(n) && n >= 8 && n <= 1024) onWidthChange(n);
    else setWText(String(width)); // 回退
  };
  /** 提交高度：解析 + 向上对齐到 8 的倍数（SSD1306 page 整除要求） */
  const commitHeight = () => {
    const n = parseInt(hText, 10);
    if (!isNaN(n) && n >= 8 && n <= 1024) onHeightChange(n);
    else setHText(String(height)); // 回退
  };

  // 把外部传入的 previewCanvas 内容画到预览 canvas 上
  useEffect(() => {
    const out = previewRef.current;
    if (!out) return;
    out.width = width;
    out.height = height;
    const ctx = out.getContext('2d')!;
    ctx.fillStyle = '#000';
    ctx.fillRect(0, 0, width, height);
    if (previewCanvas) {
      ctx.imageSmoothingEnabled = false;
      ctx.drawImage(previewCanvas, 0, 0);
    }
  }, [previewCanvas, width, height]);

  // 新日志自动滚到底部
  useEffect(() => {
    logEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [logs]);

  // "复制"按钮的瞬时反馈文案（复制成功/失败后短暂显示）
  const [copyLabel, setCopyLabel] = useState<'复制' | '已复制' | '失败'>('复制');
  /** 把全部日志格式化后写入剪贴板。格式：[时间] 消息，每条一行 */
  const copyAllLogs = async () => {
    if (!logs.length) return;
    const text = logs.map((l) => `[${l.time}] ${l.message}`).join('\n');
    try {
      await navigator.clipboard.writeText(text);
      setCopyLabel('已复制');
    } catch {
      // clipboard API 在某些 webview 环境可能不可用，回退到 textarea 选中
      const ta = document.createElement('textarea');
      ta.value = text;
      ta.style.position = 'fixed';
      ta.style.opacity = '0';
      document.body.appendChild(ta);
      ta.select();
      try {
        document.execCommand('copy');
        setCopyLabel('已复制');
      } catch {
        setCopyLabel('失败');
      }
      document.body.removeChild(ta);
    }
    setTimeout(() => setCopyLabel('复制'), 1500);
  };

  return (
    <div className="sdt-panel">
      <div className="sdt-section-title">
        <FontAwesomeIcon icon={faDisplay} />
        <span>实时预览 &amp; 调试</span>
      </div>

      {/* LCD 模拟预览 */}
      <div className="sdt-lcd-preview">
        <div className="sdt-lcd-screen">
          {previewCanvas ? (
            <canvas ref={previewRef} />
          ) : (
            <div className="sdt-lcd-placeholder">
              <FontAwesomeIcon icon={faMicrochip} className="icon" />
              <div>LCD 模拟窗口</div>
              <div style={{ fontSize: 9, marginTop: 2 }}>发送内容后此处镜像显示</div>
            </div>
          )}
          <span className="sdt-lcd-watermark">SPI DEBUG v1.0</span>
        </div>
        <div className="sdt-lcd-status">
          <span>
            <span className={`sdt-status-dot ${online ? 'connected' : ''}`} />
            {online ? '已连接' : '未连接'}
          </span>
          <span>
            {width} × {height}
          </span>
        </div>

        {/* 分辨率选择：放在 LCD 预览正下方，与预览尺寸保持视觉关联。
            选 custom 时显示宽高输入；其他预设直接用 width/height 值 */}
        <div className="sdt-resolution-control">
          <label>分辨率</label>
          <select
            className="sdt-select"
            value={resolutionPreset}
            onChange={(e) => onResolutionPresetChange(e.target.value)}
          >
            {resolutionPresets.map((r) => (
              <option key={r.key} value={r.key}>
                {r.label}
              </option>
            ))}
          </select>
          {resolutionPreset === 'custom' && (
            <div className="sdt-resolution-row" style={{ marginTop: 4 }}>
              <input
                className="sdt-input mono"
                type="number"
                value={wText}
                onChange={(e) => setWText(e.target.value)}
                onBlur={commitWidth}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') (e.target as HTMLInputElement).blur();
                }}
                min={8}
                max={1024}
              />
              <span style={{ color: 'var(--color-subtext0)' }}>×</span>
              <input
                className="sdt-input mono"
                type="number"
                value={hText}
                onChange={(e) => setHText(e.target.value)}
                onBlur={commitHeight}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') (e.target as HTMLInputElement).blur();
                }}
                min={8}
                max={1024}
                step={8}
              />
            </div>
          )}
        </div>
      </div>

      {/* 日志控制台 */}
      <div className="sdt-log-console">
        <div className="sdt-log-header">
          <span>输出日志 ({logs.length})</span>
          <div className="sdt-log-header-actions">
            {/* 复制全部日志到剪贴板 —— 方便把日志贴出来排查问题 */}
            <button
              className="sdt-btn small"
              onClick={copyAllLogs}
              disabled={logs.length === 0}
              title="复制全部日志到剪贴板"
            >
              <FontAwesomeIcon icon={faCopy} /> {copyLabel}
            </button>
            <button className="sdt-btn small" onClick={onClearLogs} title="清空日志">
              <FontAwesomeIcon icon={faTrash} /> 清空
            </button>
          </div>
        </div>
        <div className="sdt-log-list">
          {logs.length === 0 && (
            <div style={{ color: 'var(--color-subtext0)', fontSize: 11 }}>暂无日志…</div>
          )}
          {logs.map((l, i) => (
            <div key={i} className="log-line">
              <span className="log-time">[{l.time}]</span>
              <span className={`log-msg ${l.level}`}>{l.message}</span>
            </div>
          ))}
          <div ref={logEndRef} />
        </div>
      </div>
    </div>
  );
};
