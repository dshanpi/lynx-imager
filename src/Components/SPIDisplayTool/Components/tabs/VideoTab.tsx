import React, { useState, useRef, useEffect, useCallback } from 'react';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import { faPlay, faStop, faVideo, faFolderOpen, faXmark } from '@fortawesome/free-solid-svg-icons';
import { readFile } from '@tauri-apps/plugin-fs';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import type { TabContext } from './common';
import { canvasToBytes } from './common';

/**
 * 视频 Tab —— 选择视频/GIF，按用户设置的帧率逐帧下发。
 *
 * 实现：
 *   - 用 HTMLVideoElement 播放（浏览器原生支持 MP4/WEBM 等）
 *   - 用 requestAnimationFrame 节流到目标帧率
 *   - 每一帧绘制到隐藏 canvas → 阈值化 → 调 onSend
 *
 * 注意：CH347 SPI 全速也只能跑到 ~10MHz，128×64 单色一帧 1KB 数据约 800µs，
 * 加上 CS/DC 切换实测稳定帧率 5-10fps；高于此值会丢帧（视觉上不流畅但不报错）。
 *
 * GIF 文件靠浏览器 <video> 元素无法直接播 — 这里仅声明支持但未实现 GIF 解码；
 * 真要支持 GIF 需引入 gifuct-js 之类的库。
 */
export const VideoTab: React.FC<TabContext> = ({ width, height, onSend }) => {
  const [fps, setFps] = useState(10);
  const [playing, setPlaying] = useState(false);
  const [videoURL, setVideoURL] = useState<string | null>(null);
  const [videoName, setVideoName] = useState<string>('');
  const [progress, setProgress] = useState(0);
  const [currentTime, setCurrentTime] = useState(0);
  const [duration, setDuration] = useState(0);

  const videoRef = useRef<HTMLVideoElement | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const lastSendRef = useRef(0);
  const rafRef = useRef<number | null>(null);
  // 显式停止标志 —— RAF 在自身闭包里检查，避免老 tick 继续往下排
  const stoppedRef = useRef(true);
  // 把 fps 也用 ref 跟踪，避免 tick 因 fps 变化而 useCallback 重建
  const fpsRef = useRef(10);
  useEffect(() => {
    fpsRef.current = fps;
  }, [fps]);
  // 把 onSend/width/height 也用 ref 跟踪
  const onSendRef = useRef(onSend);
  const widthRef = useRef(width);
  const heightRef = useRef(height);
  useEffect(() => {
    onSendRef.current = onSend;
  }, [onSend]);
  useEffect(() => {
    widthRef.current = width;
  }, [width]);
  useEffect(() => {
    heightRef.current = height;
  }, [height]);

  const handleFile = useCallback((file: File | null | undefined) => {
    if (!file) return;
    setVideoName(file.name);
    setVideoURL(URL.createObjectURL(file));
  }, []);

  /**
   * Tauri 拖放事件 —— 同 ImageTab 的处理：Tauri 2 默认拦截 OS 级文件拖拽，
   * 浏览器原生 onDrop 收不到，必须用 onDragDropEvent。
   *
   * 拖入视频文件后用 readFile 读二进制 → 构造 Blob URL → 给 video src。
   * 注意：监听是窗口级的，用户在其它 Tab 拖文件也会触发；
   * 但只有视频类型的扩展名会被处理，其他静默忽略。
   */
  useEffect(() => {
    const wv = getCurrentWebviewWindow();
    let unlisten: (() => void) | undefined;
    (async () => {
      unlisten = await wv.onDragDropEvent(async ({ payload }) => {
        if (payload.type !== 'drop') return;
        if (!payload.paths.length) return;
        const filePath = payload.paths[0];
        const ext = filePath.split('.').pop()?.toLowerCase();
        // 浏览器 <video> 支持的常见格式 + 扩展名 → MIME 映射
        const mimeMap: Record<string, string> = {
          mp4: 'video/mp4',
          m4v: 'video/mp4',
          webm: 'video/webm',
          ogg: 'video/ogg',
          ogv: 'video/ogg',
          mov: 'video/quicktime',
          avi: 'video/x-msvideo', // 浏览器 <video> 支持度有限
        };
        const mime = mimeMap[ext ?? ''];
        if (!mime) return; // 不是视频文件，留给其他 Tab 处理
        try {
          const data = await readFile(filePath);
          const blob = new Blob([data], { type: mime });
          const fileName = filePath.split(/[/\\]/).pop() || 'video';
          setVideoName(fileName);
          setVideoURL(URL.createObjectURL(blob));
        } catch (e) {
          console.error('Failed to read dropped video:', e);
        }
      });
    })();
    return () => {
      unlisten?.();
    };
  }, []);

  /**
   * 抽帧主循环 —— 用 useRef 跟踪所有外部依赖，避免 useCallback 重建带来的"双循环"问题。
   *
   * 历史 bug：tick 用 useCallback 依赖 [fps, width, height, onSend]，每次 setCurrentTime
   * 触发重渲染 → onSend 引用变 → tick 重建 → 旧 tick 闭包里 rafRef.current 已被新值覆盖
   * → cancelAnimationFrame 无效 → 两个 RAF 同时跑 → 按钮 disabled 状态高频切换 → 闪烁。
   *
   * 修复：tick 改成普通函数，所有外部依赖通过 ref 读取（fpsRef/onSendRef/widthRef/heightRef）。
   * 用 stoppedRef 作为显式终止条件 —— handleStop 设 true 后，下次 tick 检查到就 return，
   * 不再 requestAnimationFrame。
   */
  const tick = useCallback(async () => {
    if (stoppedRef.current) {
      rafRef.current = null;
      return;
    }
    const v = videoRef.current;
    if (!v || v.paused || v.ended) {
      rafRef.current = null;
      stoppedRef.current = true;
      setPlaying(false);
      return;
    }
    const now = performance.now();
    const interval = 1000 / fpsRef.current;
    if (now - lastSendRef.current >= interval) {
      lastSendRef.current = now;
      try {
        const w = widthRef.current,
          h = heightRef.current;
        const c = document.createElement('canvas');
        c.width = w;
        c.height = h;
        const ctx = c.getContext('2d')!;
        ctx.imageSmoothingEnabled = false;
        // 等比 contain 缩放
        const ratio = Math.min(w / v.videoWidth, h / v.videoHeight);
        const dw = Math.round(v.videoWidth * ratio);
        const dh = Math.round(v.videoHeight * ratio);
        ctx.fillStyle = '#000';
        ctx.fillRect(0, 0, w, h);
        ctx.drawImage(v, Math.floor((w - dw) / 2), Math.floor((h - dh) / 2), dw, dh);
        const bytes = canvasToBytes(c);
        await onSendRef.current({
          bytes,
          canvas: c,
          description: `Frame @ ${v.currentTime.toFixed(1)}s`,
        });
      } catch {
        /* 单帧错误不中断播放 */
      }
    }
    setCurrentTime(v.currentTime);
    setDuration(v.duration || 0);
    setProgress(v.duration ? (v.currentTime / v.duration) * 100 : 0);
    // 再次检查停止标志（onSend 可能耗时，期间用户可能已点停止）
    if (stoppedRef.current) {
      rafRef.current = null;
      return;
    }
    rafRef.current = requestAnimationFrame(() => {
      void tick();
    });
  }, []);

  const handlePlay = async () => {
    if (!videoRef.current || !videoURL) return;
    // 防止重复 play：如果当前 RAF 还在跑，先停掉
    if (rafRef.current !== null) {
      cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
    }
    try {
      videoRef.current.muted = true;
      await videoRef.current.play();
      stoppedRef.current = false;
      setPlaying(true);
      lastSendRef.current = 0;
      rafRef.current = requestAnimationFrame(() => {
        void tick();
      });
    } catch (e) {
      console.error('Video play failed:', e);
    }
  };

  const handleStop = () => {
    // 先设标志，让 tick 在下一次检查时退出
    stoppedRef.current = true;
    if (videoRef.current) {
      videoRef.current.pause();
      videoRef.current.currentTime = 0;
    }
    if (rafRef.current !== null) {
      cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
    }
    setPlaying(false);
  };

  /** 清空已加载的视频：先停止播放与抽帧，再释放 Blob URL 并复位进度状态。 */
  const handleClear = () => {
    handleStop();
    if (videoURL) URL.revokeObjectURL(videoURL);
    setVideoURL(null);
    setVideoName('');
    setProgress(0);
    setCurrentTime(0);
    setDuration(0);
  };

  // 卸载时清理
  useEffect(
    () => () => {
      stoppedRef.current = true;
      if (rafRef.current !== null) cancelAnimationFrame(rafRef.current);
    },
    []
  );

  const fmtTime = (s: number) => {
    const m = Math.floor(s / 60),
      sec = Math.floor(s % 60);
    return `${String(m).padStart(2, '0')}:${String(sec).padStart(2, '0')}`;
  };

  return (
    <>
      <div className="sdt-toolbar-row">
        <label>帧率:</label>
        <input
          type="range"
          className="sdt-range"
          min={1}
          max={30}
          value={fps}
          onChange={(e) => setFps(parseInt(e.target.value))}
        />
        <span style={{ width: 36, fontFamily: 'Consolas, monospace' }}>{fps} fps</span>
        <span className="sdt-spacer" />
        {/* 清空按钮：移除已加载的视频（播放中会先停止）。无视频时禁用。 */}
        <button className="sdt-btn" onClick={handleClear} disabled={!videoURL} title="清空视频">
          <FontAwesomeIcon icon={faXmark} /> 清空
        </button>
        {/* 播放/停止同一行排列 */}
        <button className="sdt-btn success" onClick={handlePlay} disabled={!videoURL || playing}>
          <FontAwesomeIcon icon={faPlay} /> 播放
        </button>
        <button className="sdt-btn danger" onClick={handleStop} disabled={!playing}>
          <FontAwesomeIcon icon={faStop} /> 停止
        </button>
      </div>

      <div
        className="sdt-video-pick"
        onClick={() => fileInputRef.current?.click()}
        style={{ cursor: 'pointer' }}
      >
        {videoURL ? (
          <video
            ref={videoRef}
            src={videoURL}
            style={{ maxWidth: '100%', maxHeight: 200, background: '#000' }}
            controls={false}
          />
        ) : (
          <>
            <FontAwesomeIcon icon={faVideo} className="icon" />
            <div style={{ fontSize: 12, color: 'var(--color-text)' }}>
              <FontAwesomeIcon icon={faFolderOpen} /> 选择 MP4 / WEBM / AVI 文件
            </div>
            <div style={{ fontSize: 10, color: 'var(--color-subtext0)', marginTop: 4 }}>
              GIF 暂不支持（需 gifuct-js 解码库）
            </div>
          </>
        )}
        {videoURL && (
          <div style={{ fontSize: 10, color: 'var(--color-subtext0)', marginTop: 4 }}>
            {videoName}
          </div>
        )}
        <input
          ref={fileInputRef}
          type="file"
          accept="video/*"
          hidden
          onChange={(e) => {
            handleFile(e.target.files?.[0]);
            e.target.value = '';
          }}
        />
      </div>

      <div className="sdt-video-progress">
        <div className="sdt-progress-bar">
          <div className="sdt-progress-fill" style={{ width: `${progress}%` }} />
        </div>
        <div className="sdt-progress-info">
          <span>缓冲: {progress.toFixed(0)}%</span>
          <span>
            {fmtTime(currentTime)} / {fmtTime(duration)}
          </span>
        </div>
      </div>
    </>
  );
};
