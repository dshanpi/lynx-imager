import React, { useState, useRef } from 'react';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import { faPaperPlane } from '@fortawesome/free-solid-svg-icons';
import { OLED_FONT_FAMILY, loadOledFont } from '../../../../Library/SSD1306';
import type { TabContext } from './common';
import { canvasToBytes } from './common';

const FONT_SIZES = [10, 12, 14, 16, 18, 20, 24, 28];

/**
 * 文本 Tab —— 多行文字 + TTF 字体渲染。
 *
 * 输入框中的每一行从画布顶部开始，按行高（fontSize+2）依次绘制。
 * 颜色是 1-bit 色（前景/背景），但仍开放取色器以保留 spec 中 UI；
 * 阈值化时将前景视为"亮"（白）、背景视为"暗"（黑）。
 */
export const TextTab: React.FC<TabContext> = ({ width, height, onSend, busy }) => {
  const [fontSize, setFontSize] = useState(16);
  const [fgColor, setFgColor] = useState('#ffffff');
  const [bgColor, setBgColor] = useState('#000000');
  const [text, setText] = useState('Hello, SPI LCD!\n你好，SPI 屏幕\nLine 3');
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  /** 把当前文字绘制到隐藏 canvas，然后转字节并交给父组件发送。 */
  const handleSend = async () => {
    try {
      await loadOledFont();
    } catch {
      /* fallback to system */
    }

    const canvas = canvasRef.current ?? document.createElement('canvas');
    canvas.width = width;
    canvas.height = height;
    canvasRef.current = canvas;

    const ctx = canvas.getContext('2d')!;
    // 背景填充
    ctx.fillStyle = bgColor;
    ctx.fillRect(0, 0, width, height);
    // 文字
    ctx.fillStyle = fgColor;
    ctx.font = `${fontSize}px "${OLED_FONT_FAMILY}", sans-serif`;
    ctx.textBaseline = 'top';
    ctx.textAlign = 'left';
    ctx.imageSmoothingEnabled = false;

    const lineHeight = fontSize + 2;
    text.split('\n').forEach((line, i) => {
      ctx.fillText(line, 2, i * lineHeight);
    });

    const bytes = canvasToBytes(canvas);
    await onSend({
      bytes,
      canvas,
      description: `Text: ${text.split('\n').length} lines @ ${fontSize}px`,
      bgColor,
    });
  };

  return (
    <>
      <div className="sdt-toolbar-row">
        <label>字号:</label>
        <select
          className="sdt-select"
          style={{ width: 70 }}
          value={fontSize}
          onChange={(e) => setFontSize(parseInt(e.target.value))}
        >
          {FONT_SIZES.map((s) => (
            <option key={s} value={s}>
              {s}px
            </option>
          ))}
        </select>
        <label>文字:</label>
        <input
          type="color"
          className="sdt-color-input"
          value={fgColor}
          onChange={(e) => setFgColor(e.target.value)}
        />
        <label>背景:</label>
        <input
          type="color"
          className="sdt-color-input"
          value={bgColor}
          onChange={(e) => setBgColor(e.target.value)}
        />
        <span className="sdt-spacer" />
        <button className="sdt-btn primary" onClick={handleSend} disabled={busy}>
          <FontAwesomeIcon icon={faPaperPlane} /> 发送
        </button>
      </div>
      <textarea
        className="sdt-textarea"
        value={text}
        onChange={(e) => setText(e.target.value)}
        placeholder="输入要显示的文字（支持中英日韩，多行）"
      />
    </>
  );
};
