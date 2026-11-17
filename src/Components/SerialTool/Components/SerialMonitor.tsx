import React, { useState, useCallback, useRef, useEffect } from 'react';
import { useTranslation } from 'react-i18next';

interface SerialMonitorProps {
  text: string;
  errorMsg?: string;
  onSend: (data: number[], skipEcho?: boolean) => void;
  onLocalEcho: (data: number[]) => void;
  onClear: () => void;
  onSaveLog: () => void;
  canRestore: boolean;
  onRestore: () => void;
  onDeleteLastChar: () => void;
  connected: boolean;
  connecting: boolean;
  showTimestamp: boolean;
}

export const SerialMonitor: React.FC<SerialMonitorProps> = ({
  text,
  errorMsg,
  onSend,
  onLocalEcho,
  onClear,
  onSaveLog,
  canRestore,
  onRestore,
  onDeleteLastChar,
  connected,
  connecting,
  showTimestamp,
}) => {
  const { t } = useTranslation();
  const [appendNewline, setAppendNewline] = useState(true);
  const [localEcho, setLocalEcho] = useState(false);
  const [autoScroll, setAutoScroll] = useState(true);
  const [fontSize, setFontSize] = useState(13);
  const displayRef = useRef<HTMLDivElement>(null);
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number } | null>(null);
  const savedSelectionRef = useRef('');

  // Auto-scroll when new text arrives
  useEffect(() => {
    if (autoScroll && displayRef.current) {
      displayRef.current.scrollTop = displayRef.current.scrollHeight;
    }
  }, [text, autoScroll]);

  const sendText = useCallback(
    (text: string, skipEcho?: boolean) => {
      const bytes = Array.from(new TextEncoder().encode(text));
      onSend(bytes, skipEcho);
      if (!skipEcho && localEcho && !showTimestamp) {
        onLocalEcho(bytes);
      }
    },
    [onSend, onLocalEcho, localEcho, showTimestamp]
  );

  const sendKey = useCallback((key: string) => sendText(key), [sendText]);

  const handleKeyDown = useCallback(
    async (e: React.KeyboardEvent) => {
      if (!connected) return;

      // Ctrl+V: paste clipboard text as serial input
      if ((e.ctrlKey || e.metaKey) && e.key === 'v') {
        e.preventDefault();
        try {
          const clipText = await navigator.clipboard.readText();
          if (clipText) sendText(clipText);
        } catch {
          // Clipboard access denied
        }
        return;
      }

      // Allow other system shortcuts to pass through (Ctrl+C copy, etc.)
      if (e.ctrlKey || e.metaKey || e.altKey) return;

      if (e.key === 'Enter') {
        e.preventDefault();
        sendKey(appendNewline ? '\n' : '\r');
        return;
      }

      if (e.key === 'Backspace') {
        e.preventDefault();
        sendText('\x7f', true); // send DEL, skip local echo
        onDeleteLastChar(); // remove last char from display
        return;
      }

      if (e.key === 'Escape') {
        e.preventDefault();
        sendKey('\x1b');
        return;
      }

      if (e.key === 'Tab') {
        e.preventDefault();
        sendKey('\t');
        return;
      }

      // Printable single characters
      if (e.key.length === 1) {
        e.preventDefault();
        sendKey(e.key);
      }
    },
    [connected, appendNewline, sendKey, sendText, onDeleteLastChar]
  );

  // Right-click context menu
  const handleContextMenu = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    // Save current selection before showing menu (click on menu item may clear selection)
    const sel = window.getSelection();
    if (sel && sel.toString()) {
      savedSelectionRef.current = sel.toString();
    } else {
      savedSelectionRef.current = '';
    }
    setContextMenu({ x: e.nativeEvent.offsetX, y: e.nativeEvent.offsetY });
  }, []);

  const handleContextCopy = useCallback(async () => {
    setContextMenu(null);
    const text = savedSelectionRef.current || window.getSelection()?.toString() || '';
    if (text) {
      await navigator.clipboard.writeText(text);
    }
  }, []);

  const changeFontSize = useCallback((delta: number) => {
    setFontSize((prev) => Math.max(8, Math.min(36, prev + delta)));
  }, []);

  const handleWheel = useCallback(
    (e: React.WheelEvent) => {
      if (e.ctrlKey || e.metaKey) {
        e.preventDefault();
        changeFontSize(e.deltaY < 0 ? 1 : -1);
      }
    },
    [changeFontSize]
  );

  const handleContextPaste = useCallback(async () => {
    setContextMenu(null);
    if (!connected) return;
    try {
      const clipText = await navigator.clipboard.readText();
      if (clipText) sendText(clipText);
    } catch {
      // Clipboard access denied
    }
  }, [connected, sendText]);

  const statusText = connecting
    ? t('serialTool.status.connecting')
    : connected
      ? t('serialTool.status.connected')
      : t('serialTool.status.disconnected');

  return (
    <div className="serial-monitor">
      <div className="serial-monitor-toolbar">
        <span className="serial-monitor-title">
          <span className={`status-led ${connected ? 'active' : ''}`} />
          {statusText}
        </span>
        <div className="serial-monitor-actions">
          <label className="serial-option-inline" title="Echo typed characters locally">
            <input
              type="checkbox"
              checked={localEcho}
              onChange={(e) => setLocalEcho(e.target.checked)}
            />
            Echo
          </label>
          <label className="serial-option-inline" title="Append newline on Enter">
            <input
              type="checkbox"
              checked={appendNewline}
              onChange={(e) => setAppendNewline(e.target.checked)}
            />
            +\\n
          </label>
          <button
            className="serial-action-btn"
            onClick={onSaveLog}
            title={t('serialTool.contextMenu.saveLog')}
          >
            {t('serialTool.contextMenu.saveLog')}
          </button>
          <button
            className={`serial-action-btn ${autoScroll ? 'active' : ''}`}
            onClick={() => setAutoScroll(!autoScroll)}
          >
            {autoScroll ? t('serialTool.monitor.autoScroll') : t('serialTool.monitor.locked')}
          </button>
          <button className="serial-action-btn" onClick={onClear}>
            {t('serialTool.monitor.clear')}
          </button>
          {canRestore && (
            <button
              className="serial-action-btn"
              onClick={onRestore}
              title={t('serialTool.monitor.restore')}
            >
              {t('serialTool.monitor.restore')}
            </button>
          )}
        </div>
      </div>
      <div
        className="serial-monitor-display interactive"
        ref={displayRef}
        tabIndex={0}
        onKeyDown={handleKeyDown}
        onWheel={handleWheel}
        onContextMenu={handleContextMenu}
        onClick={() => {
          displayRef.current?.focus();
          setContextMenu(null);
        }}
      >
        {errorMsg && <div className="serial-error">{errorMsg}</div>}
        {text ? (
          <pre
            className="serial-output"
            style={{ fontSize: `${fontSize}px` }}
            dangerouslySetInnerHTML={{ __html: text }}
          />
        ) : (
          <div className="serial-placeholder">
            {connected
              ? t('serialTool.monitor.waitingForData')
              : t('serialTool.monitor.openToBegin')}
          </div>
        )}
        {contextMenu && (
          <div className="serial-context-menu" style={{ left: contextMenu.x, top: contextMenu.y }}>
            <button className="context-menu-item" onClick={handleContextCopy}>
              {t('serialTool.contextMenu.copy')}
            </button>
            <button
              className="context-menu-item"
              onClick={handleContextPaste}
              disabled={!connected}
            >
              {t('serialTool.contextMenu.paste')}
            </button>
            <div className="context-menu-sep" />
            <button
              className="context-menu-item"
              onClick={() => {
                setContextMenu(null);
                onSaveLog();
              }}
            >
              {t('serialTool.contextMenu.saveLog')}
            </button>
            <div className="context-menu-sep" />
            <button
              className="context-menu-item"
              onClick={() => {
                setContextMenu(null);
                changeFontSize(1);
              }}
            >
              {t('serialTool.contextMenu.fontIncrease')}
            </button>
            <button
              className="context-menu-item"
              onClick={() => {
                setContextMenu(null);
                changeFontSize(-1);
              }}
            >
              {t('serialTool.contextMenu.fontDecrease')}
            </button>
          </div>
        )}
      </div>
    </div>
  );
};
