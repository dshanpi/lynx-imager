import React, { useState, useRef, useEffect, useCallback } from 'react';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import { faEraser, faPaperPlane } from '@fortawesome/free-solid-svg-icons';
import type { TabContext } from './common';
import { canvasToBytes } from './common';

/**
 * 绘制 Tab —— 用户在画布上拖拽绘制，然后把整个画布作为显存下发。
 *
 * 画布的物理尺寸 = 屏幕分辨率（width × height 像素），
 * 显示尺寸（CSS）按宽高比等比放大到容器可用空间。
 *
 * 这样做的好处：
 *   1. 用户能直观看到画布"形状跟屏幕一致"——宽屏/方屏/竖屏都正确反映
 *   2. 鼠标点击位置 → 逻辑像素的换算与显示缩放比例无关，准确
 *   3. 改分辨率时保留已绘内容（用 ImageData 暂存再 put 回新尺寸 canvas）
 *
 * 笔刷颜色虽然开放取色器，但最终阈值化时只有"亮/暗"两态。
 */
export const DrawTab: React.FC<TabContext> = ({ width, height, onSend, busy }) => {
  const [brushColor, setBrushColor] = useState('#ffffff');
  const [brushSize, setBrushSize] = useState(2);
  const [coords, setCoords] = useState<{ x: number; y: number } | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const wrapRef = useRef<HTMLDivElement | null>(null);
  /** canvas 的 CSS 显示尺寸（按容器和宽高比计算），null=未计算 */
  const [displaySize, setDisplaySize] = useState<{ w: number; h: number } | null>(null);
  const drawingRef = useRef(false);

  /**
   * 改分辨率时：
   *   - 保存当前画布内容到 ImageData
   *   - 调整 canvas 物理尺寸到新 width × height
   *   - 填黑背景
   *   - 把旧内容 put 回去（左上角对齐，超出部分裁剪，不足部分留黑）
   * 这样用户改分辨率不会丢失已绘内容。
   */
  useEffect(() => {
    const c = canvasRef.current;
    if (!c) return;
    const ctx = c.getContext('2d')!;
    // 保存旧内容（如果旧尺寸非零）
    let oldData: ImageData | null = null;
    if (c.width > 0 && c.height > 0) {
      try {
        oldData = ctx.getImageData(0, 0, c.width, c.height);
      } catch {
        /* ignore */
      }
    }
    c.width = width;
    c.height = height;
    ctx.fillStyle = '#000';
    ctx.fillRect(0, 0, width, height);
    if (oldData) {
      // putImageData 不会缩放，直接左上角对齐贴回。超出新边界的部分自动丢失
      ctx.putImageData(oldData, 0, 0);
    }
  }, [width, height]);

  /**
   * 监听容器尺寸变化 + 分辨率变化，计算 canvas 的 CSS 显示尺寸。
   * 目标：在容器内尽量大，同时保持 width:height 比例。
   * 用 ResizeObserver 监听 wrap 尺寸（用户改窗口大小也会触发）。
   */
  useEffect(() => {
    const wrap = wrapRef.current;
    if (!wrap) return;
    const calc = () => {
      const cw = wrap.clientWidth;
      const ch = wrap.clientHeight;
      if (cw <= 0 || ch <= 0 || width <= 0 || height <= 0) return;
      const ratio = width / height;
      const containerRatio = cw / ch;
      let dw, dh;
      if (containerRatio > ratio) {
        // 容器更宽 —— 以高度为基准
        dh = ch;
        dw = dh * ratio;
      } else {
        // 容器更高 —— 以宽度为基准
        dw = cw;
        dh = dw / ratio;
      }
      setDisplaySize({ w: Math.floor(dw), h: Math.floor(dh) });
    };
    calc();
    const ro = new ResizeObserver(calc);
    ro.observe(wrap);
    return () => ro.disconnect();
  }, [width, height]);

  /** 把客户端鼠标坐标转换成画布逻辑坐标。 */
  const getCanvasPos = useCallback((e: React.MouseEvent<HTMLCanvasElement>) => {
    const c = canvasRef.current!;
    const rect = c.getBoundingClientRect();
    // 客户端 px → 画布逻辑 px（按缩放比例换算）
    return {
      x: Math.floor((e.clientX - rect.left) * (c.width / rect.width)),
      y: Math.floor((e.clientY - rect.top) * (c.height / rect.height)),
    };
  }, []);

  const drawDot = useCallback(
    (x: number, y: number) => {
      const ctx = canvasRef.current!.getContext('2d')!;
      ctx.fillStyle = brushColor;
      // 用方块（更适合 1-bit 屏幕）
      const r = Math.max(1, Math.floor(brushSize / 2));
      ctx.fillRect(x - r, y - r, brushSize, brushSize);
    },
    [brushColor, brushSize]
  );

  const handleMouseDown = (e: React.MouseEvent<HTMLCanvasElement>) => {
    drawingRef.current = true;
    const p = getCanvasPos(e);
    drawDot(p.x, p.y);
    setCoords(p);
  };
  const handleMouseMove = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const p = getCanvasPos(e);
    setCoords(p);
    if (drawingRef.current) drawDot(p.x, p.y);
  };
  const handleMouseUp = () => {
    drawingRef.current = false;
  };

  const handleClear = () => {
    const c = canvasRef.current!;
    const ctx = c.getContext('2d')!;
    ctx.fillStyle = '#000';
    ctx.fillRect(0, 0, c.width, c.height);
    setCoords(null);
  };

  const handleSend = async () => {
    if (!canvasRef.current) return;
    const bytes = canvasToBytes(canvasRef.current);
    await onSend({
      bytes,
      canvas: canvasRef.current,
      description: `Drawn framebuffer ${width}×${height}`,
    });
  };

  return (
    <>
      <div className="sdt-toolbar-row">
        <label>颜色:</label>
        <input
          type="color"
          className="sdt-color-input"
          value={brushColor}
          onChange={(e) => setBrushColor(e.target.value)}
        />
        <label>笔刷:</label>
        <input
          type="range"
          className="sdt-range"
          min={1}
          max={20}
          value={brushSize}
          onChange={(e) => setBrushSize(parseInt(e.target.value))}
        />
        <span style={{ width: 28, fontFamily: 'Consolas, monospace' }}>{brushSize}px</span>
        <button className="sdt-btn" onClick={handleClear}>
          <FontAwesomeIcon icon={faEraser} /> 清屏
        </button>
        <span className="sdt-spacer" />
        <button className="sdt-btn primary" onClick={handleSend} disabled={busy}>
          <FontAwesomeIcon icon={faPaperPlane} /> 发送显存
        </button>
      </div>

      {/* wrap 是黑色背景容器，canvas 按分辨率比例居中放大显示。
          displaySize 由 ResizeObserver 计算后注入 canvas 的 style.width/height，
          这样不同分辨率（128×64 横屏、240×320 竖屏等）都能正确反映屏幕形状。 */}
      <div className="sdt-canvas-wrap" ref={wrapRef}>
        <canvas
          ref={canvasRef}
          style={displaySize ? { width: displaySize.w, height: displaySize.h } : undefined}
          onMouseDown={handleMouseDown}
          onMouseMove={handleMouseMove}
          onMouseUp={handleMouseUp}
          onMouseLeave={handleMouseUp}
        />
      </div>

      <div className="sdt-canvas-status">
        <span>
          画布 {width}×{height}（按屏幕分辨率等比显示，鼠标拖拽绘制）
        </span>
        <span>{coords ? `(${coords.x}, ${coords.y})` : '(--, --)'}</span>
      </div>
    </>
  );
};
