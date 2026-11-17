import React, { useState, useRef, useCallback, useEffect } from 'react';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import { faCloudArrowUp, faPaperPlane, faImage, faXmark } from '@fortawesome/free-solid-svg-icons';
import { readFile } from '@tauri-apps/plugin-fs';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import type { TabContext } from './common';
import { canvasToBytes } from './common';

type ScaleMode = 'stretch' | 'contain' | 'tile';

const SCALE_LABELS: Record<ScaleMode, string> = {
  stretch: '全屏拉伸',
  contain: '等比居中',
  tile: '平铺',
};

/**
 * 图片 Tab —— 拖拽/选择图片，按缩放模式绘制到 1-bit 位图后下发。
 *
 * 支持任意浏览器可解码的格式（PNG/JPG/BMP/WEBP/GIF 第一帧）。
 * 实际取模时统一阈值化（红通道 ≥128 视为点亮）；如需更精细的灰度抖动，
 * 可在 canvasToBytes 加入 Floyd-Steinberg 步骤。
 *
 * 关于拖拽：Tauri 2 默认拦截 OS 级的文件拖拽事件（dragDropEnabled = true），
 * 导致浏览器原生的 onDrop 永远收不到。因此这里同时注册 Tauri 的
 * onDragDropEvent 监听器来处理文件路径拖入。
 */
export const ImageTab: React.FC<TabContext> = ({ width, height, onSend, busy }) => {
  const [scaleMode, setScaleMode] = useState<ScaleMode>('contain');
  const [imageURL, setImageURL] = useState<string | null>(null);
  const [imageName, setImageName] = useState<string>('');
  const [dragging, setDragging] = useState(false);
  const fileInputRef = useRef<HTMLInputElement | null>(null);

  /** 处理用户选择/拖入的文件（通过浏览器 File API）。 */
  const handleFile = useCallback((file: File | null | undefined) => {
    if (!file) return;
    if (!file.type.startsWith('image/')) {
      window.alert(`不是图片文件：${file.type}`);
      return;
    }
    setImageName(file.name);
    const url = URL.createObjectURL(file);
    setImageURL(url);
  }, []);

  /** 处理 Tauri 拖放事件（文件路径）。 */
  useEffect(() => {
    const wv = getCurrentWebviewWindow();
    let unlisten: (() => void) | undefined;
    (async () => {
      unlisten = await wv.onDragDropEvent(async ({ payload }) => {
        if (payload.type === 'over') {
          setDragging(true);
          return;
        }
        if (payload.type === 'leave') {
          setDragging(false);
          return;
        }
        // drop：从文件路径读取二进制后构造 Blob URL
        if (payload.type === 'drop') {
          setDragging(false);
          if (!payload.paths.length) return;
          const filePath = payload.paths[0];
          const ext = filePath.split('.').pop()?.toLowerCase();
          const mimeMap: Record<string, string> = {
            png: 'image/png',
            jpg: 'image/jpeg',
            jpeg: 'image/jpeg',
            bmp: 'image/bmp',
            webp: 'image/webp',
            gif: 'image/gif',
          };
          const mime = mimeMap[ext ?? ''];
          if (!mime) return; // 不是图片，留给 VideoTab 等其他 Tab 处理
          try {
            const data = await readFile(filePath);
            const blob = new Blob([data], { type: mime });
            const fileName = filePath.split(/[/\\]/).pop() || 'image';
            setImageName(fileName);
            setImageURL(URL.createObjectURL(blob));
          } catch (e) {
            console.error('Failed to read dropped file:', e);
          }
        }
      });
    })();
    return () => {
      unlisten?.();
    };
  }, []);

  /**
   * 把图片按 scaleMode 绘制到目标 canvas。
   * stretch: 直接填满，比例可能失真
   * contain: 等比缩放并居中，超出部分留黑
   * tile:    左上角原始尺寸平铺
   */
  const renderToCanvas = (img: HTMLImageElement, w: number, h: number): HTMLCanvasElement => {
    const c = document.createElement('canvas');
    c.width = w;
    c.height = h;
    const ctx = c.getContext('2d')!;
    ctx.fillStyle = '#000';
    ctx.fillRect(0, 0, w, h);
    ctx.imageSmoothingEnabled = false;

    if (scaleMode === 'stretch') {
      ctx.drawImage(img, 0, 0, w, h);
    } else if (scaleMode === 'contain') {
      const ratio = Math.min(w / img.width, h / img.height);
      const dw = Math.round(img.width * ratio);
      const dh = Math.round(img.height * ratio);
      ctx.drawImage(img, Math.floor((w - dw) / 2), Math.floor((h - dh) / 2), dw, dh);
    } else {
      // tile：从左上角原始尺寸开始重复
      for (let y = 0; y < h; y += img.height) {
        for (let x = 0; x < w; x += img.width) {
          ctx.drawImage(img, x, y);
        }
      }
    }
    return c;
  };

  const handleSend = async () => {
    if (!imageURL) {
      window.alert('请先选择一张图片');
      return;
    }
    const img = new Image();
    img.src = imageURL;
    await img.decode();

    const canvas = renderToCanvas(img, width, height);
    const bytes = canvasToBytes(canvas);
    await onSend({
      bytes,
      canvas,
      description: `Image: ${imageName} (${SCALE_LABELS[scaleMode]})`,
    });
  };

  /** 清空已加载的图片：释放 Blob URL 并复位状态，拖拽区回到初始提示。 */
  const handleClear = useCallback(() => {
    if (imageURL) URL.revokeObjectURL(imageURL);
    setImageURL(null);
    setImageName('');
  }, [imageURL]);

  return (
    <>
      <div className="sdt-toolbar-row">
        <label>缩放模式:</label>
        <select
          className="sdt-select"
          style={{ width: 110 }}
          value={scaleMode}
          onChange={(e) => setScaleMode(e.target.value as ScaleMode)}
        >
          {(Object.keys(SCALE_LABELS) as ScaleMode[]).map((k) => (
            <option key={k} value={k}>
              {SCALE_LABELS[k]}
            </option>
          ))}
        </select>
        <span className="sdt-spacer" />
        {/* 清空按钮：移除已加载的图片，拖拽区回到初始提示。无图片时禁用。 */}
        <button className="sdt-btn" onClick={handleClear} disabled={!imageURL} title="清空图片">
          <FontAwesomeIcon icon={faXmark} /> 清空
        </button>
        <button className="sdt-btn primary" onClick={handleSend} disabled={busy || !imageURL}>
          <FontAwesomeIcon icon={faPaperPlane} /> 取模并发送
        </button>
      </div>

      <div
        className={`sdt-image-drop ${dragging ? 'dragging' : ''}`}
        onClick={() => fileInputRef.current?.click()}
        onDragEnter={(e) => {
          e.preventDefault();
          setDragging(true);
        }}
        onDragOver={(e) => {
          e.preventDefault();
          setDragging(true);
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={(e) => {
          e.preventDefault();
          setDragging(false);
          handleFile(e.dataTransfer.files?.[0]);
        }}
      >
        {imageURL ? (
          <>
            <img className="preview" src={imageURL} alt={imageName} />
            <div className="hint">{imageName}</div>
            <div className="sub-hint">点击或拖拽以更换图片</div>
          </>
        ) : (
          <>
            <FontAwesomeIcon icon={faCloudArrowUp} className="icon" />
            <div className="hint">拖拽图片至此，或点击选择</div>
            <div className="sub-hint">支持 PNG / JPG / BMP / WEBP / GIF 第一帧</div>
            <FontAwesomeIcon icon={faImage} style={{ display: 'none' }} />
          </>
        )}
        <input
          ref={fileInputRef}
          type="file"
          accept="image/*"
          hidden
          onChange={(e) => {
            handleFile(e.target.files?.[0]);
            e.target.value = '';
          }}
        />
      </div>
    </>
  );
};
