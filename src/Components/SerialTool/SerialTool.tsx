import React, { useState, useCallback, useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { FontAwesomeIcon } from '@fortawesome/react-fontawesome';
import { faChevronDown, faChevronUp } from '@fortawesome/free-solid-svg-icons';
import { invokeCommand, subscribeEvent } from '../../Platform/IPC';
import type { UnlistenFn } from '@tauri-apps/api/event';
import { SerialMonitor } from './Components/SerialMonitor';
import { SerialSettings } from './Components/SerialSettings';
import { SendPanel } from './Components/SendPanel';
import { MultiSendPanel } from './Components/MultiSendPanel';
import {
  applyChecksum,
  previewChecksum,
  DEFAULT_CHECKSUM_CONFIG,
  type ChecksumConfig,
} from './checksum';
import { createAnsiConverter, highlightKeywords } from './ansi';
import { save } from '@tauri-apps/plugin-dialog';
import { writeTextFile } from '@tauri-apps/plugin-fs';
import { formatErrorMessage } from '../../Utils/Error';
import './SerialTool.css';

export interface SerialPortConfig {
  port: string;
  baudRate: number;
  dataBits: number;
  stopBits: number;
  parity: string;
  flowControl: string;
}

export interface DisplayOptions {
  showTimestamp: boolean;
  hexDisplay: boolean;
}

interface SerialToolProps {
  isActive?: boolean;
}

/** Extract a human-readable message from any thrown value (Error, IpcError object, or string). */
function getErrorMessage(e: unknown): string {
  return formatErrorMessage(e);
}

function escapeHtml(text: string): string {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

/** Strip or replace invisible control characters for display. */
function sanitizeControlChars(text: string): string {
  let result = '';
  for (const ch of text) {
    const code = ch.charCodeAt(0);
    if (code === 0x08 || code === 0x7f) {
      // BS/DEL: for sent data, was already handled by onDeleteLastChar
      // For received data, device echo of backspace is a terminal control, not display content
      // Just drop it silently
    } else if (code === 0x1b) {
      result += ch; // ESC → pass through (ANSI converter needs it)
    } else if (code >= 0x20 || code === 0x0a || code === 0x0d || code === 0x09) {
      result += ch; // printable, newline, CR, tab
    }
    // Other control chars are dropped
  }
  return result;
}

/** Format current time as [HH:MM:SS.mmm] */
function formatTimestamp(): string {
  const now = new Date();
  const h = String(now.getHours()).padStart(2, '0');
  const m = String(now.getMinutes()).padStart(2, '0');
  const s = String(now.getSeconds()).padStart(2, '0');
  const ms = String(now.getMilliseconds()).padStart(3, '0');
  return `[${h}:${m}:${s}.${ms}]`;
}

/** Convert byte array to space-separated hex string */
function bytesToHex(data: number[]): string {
  return data.map((b) => b.toString(16).padStart(2, '0').toUpperCase()).join(' ');
}

interface RawChunk {
  data: number[];
  isSent: boolean;
  timestamp: string; // captured at arrival time
}

export const SerialTool: React.FC<SerialToolProps> = () => {
  const { t } = useTranslation();
  const [connected, setConnected] = useState(false);
  const [connecting, setConnecting] = useState(false);
  const [receivedText, setReceivedText] = useState('');
  const [errorMsg, setErrorMsg] = useState('');
  const [ports, setPorts] = useState<Array<{ name: string; description: string }>>([]);
  const [showTimestamp, setShowTimestamp] = useState(false);
  const [hexDisplay, setHexDisplay] = useState(false);
  const [checksumConfig, setChecksumConfig] = useState<ChecksumConfig>(DEFAULT_CHECKSUM_CONFIG);
  const [sendText, setSendText] = useState('');
  const [sendHexMode, setSendHexMode] = useState(false);
  const [sendAppendNewline, setSendAppendNewline] = useState(true);
  const [settingsCollapsed, setSettingsCollapsed] = useState(false);
  const [autoRefresh, setAutoRefresh] = useState(false);
  const [clearedText, setClearedText] = useState('');
  const [canRestore, setCanRestore] = useState(false);
  const unlistenRef = useRef<UnlistenFn | null>(null);
  const tRef = useRef(t);
  tRef.current = t;
  const refreshPortsRef = useRef<() => Promise<void>>(async () => {});
  const ansiConverterRef = useRef(createAnsiConverter());
  const txLineBufRef = useRef('');
  const rxLineBufRef = useRef('');
  const txLineActiveRef = useRef(false);
  const rawChunksRef = useRef<RawChunk[]>([]);
  const hexDisplayRef = useRef(hexDisplay);
  const showTimestampRef = useRef(showTimestamp);
  hexDisplayRef.current = hexDisplay;
  showTimestampRef.current = showTimestamp;

  const [config, setConfig] = useState<SerialPortConfig>({
    port: '',
    baudRate: 115200,
    dataBits: 8,
    stopBits: 1,
    parity: 'none',
    flowControl: 'none',
  });

  const displayOptions: DisplayOptions = { showTimestamp, hexDisplay };

  // Compute checksum preview from current send text
  const checksumPreview = (() => {
    if (!checksumConfig.enabled || !sendText) return '';
    let text = sendText;
    if (!sendHexMode && sendAppendNewline) text += '\n';
    const bytes: number[] = sendHexMode
      ? (text
          .replace(/\s/g, '')
          .match(/.{1,2}/g)
          ?.map((s) => parseInt(s, 16))
          .filter((b) => !isNaN(b)) ?? [])
      : Array.from(new TextEncoder().encode(text));
    if (bytes.length === 0) return '';
    return previewChecksum(bytes, checksumConfig);
  })();

  /**
   * Convert a single raw chunk to display text using current options.
   *
   * Sent data (isSent=true) with timestamp: displayed immediately so the
   * user sees characters as they type. Only the first char after a newline
   * gets the prefix; subsequent chars append raw until Enter flushes.
   *
   * Received data (isSent=false) with timestamp: line-buffered by \n so
   * each complete device output line gets one prefix.
   */
  const formatChunk = useCallback(
    (
      chunk: RawChunk,
      ansiConv: ReturnType<typeof createAnsiConverter>,
      buffers: {
        txBuf: { current: string };
        rxBuf: { current: string };
        txActive: { current: boolean };
      }
    ): string => {
      const { data, isSent, timestamp } = chunk;

      // Determine display mode: force hex for binary (non-printable) sent data
      const useHex =
        hexDisplayRef.current ||
        (isSent && data.some((b) => b < 0x20 && b !== 0x09 && b !== 0x0a && b !== 0x0d));

      if (useHex) {
        const hex = bytesToHex(data);
        if (showTimestampRef.current) {
          const prefix = isSent ? `${timestamp}发→◇ ` : `${timestamp}收←◆ `;
          return prefix + hex + '\n';
        }
        return hex + ' ';
      }

      // ASCII mode
      const raw = sanitizeControlChars(String.fromCharCode(...data));
      if (!raw) return '';

      if (showTimestampRef.current) {
        if (isSent) {
          // Sent data: display immediately, no ANSI codes in user input
          let result = '';
          for (const ch of raw) {
            if (ch === '\n') {
              buffers.txActive.current = false;
              result += '\n';
            } else if (!buffers.txActive.current) {
              buffers.txActive.current = true;
              result += `${timestamp}发→◇${escapeHtml(ch)}`;
            } else {
              result += escapeHtml(ch);
            }
          }
          return result;
        }

        // Received data: buffer raw text by \n, then convert to HTML
        const prefix = `${timestamp}收←◆`;
        buffers.rxBuf.current += raw;
        const lines = buffers.rxBuf.current.split('\n');
        const complete = lines.slice(0, -1);
        const remainder = lines[lines.length - 1] ?? '';
        let result = '';
        if (complete.length > 0) {
          result = complete
            .map((line) => prefix + highlightKeywords(ansiConv(line)) + '\n')
            .join('');
        }
        buffers.rxBuf.current = remainder;
        return result;
      }

      // No timestamp: just convert to HTML directly
      return highlightKeywords(ansiConv(raw));
    },
    []
  );

  /**
   * Rebuild the entire display from all raw chunks.
   * Called when display mode (HEX/timestamp) changes.
   */
  const rebuildDisplay = useCallback(() => {
    const ansiStripper = createAnsiConverter();
    const buffers = {
      txBuf: { current: '' },
      rxBuf: { current: '' },
      txActive: { current: false },
    };
    let result = '';
    for (const chunk of rawChunksRef.current) {
      result += formatChunk(chunk, ansiStripper, buffers);
    }
    setReceivedText(result);
  }, [formatChunk]);

  // Toggle handlers — rebuild display on change
  const handleToggleTimestamp = useCallback(() => {
    setShowTimestamp((v) => !v);
  }, []);

  const handleToggleHex = useCallback(() => {
    setHexDisplay((v) => !v);
  }, []);

  // Rebuild display only when HEX mode changes (not timestamp)
  useEffect(() => {
    if (rawChunksRef.current.length > 0) {
      rebuildDisplay();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hexDisplay]);

  /**
   * Process incoming byte data: store raw chunk + append formatted text.
   */
  const processReceived = useCallback(
    (data: number[], isSent: boolean) => {
      const chunk: RawChunk = {
        data,
        isSent,
        timestamp: formatTimestamp(),
      };
      rawChunksRef.current.push(chunk);

      // Append formatted text for this chunk (uses separate tx/rx line buffers)
      const buffers = { txBuf: txLineBufRef, rxBuf: rxLineBufRef, txActive: txLineActiveRef };
      const text = formatChunk(chunk, ansiConverterRef.current, buffers);
      if (text) {
        setReceivedText((prev) => prev + text);
      }
    },
    [formatChunk]
  );

  // Subscribe to serial data events
  const startListening = useCallback(async () => {
    if (unlistenRef.current) {
      unlistenRef.current();
      unlistenRef.current = null;
    }
    try {
      const unlisten = await subscribeEvent('serial-data-received', (payload) => {
        // Empty data signals disconnection
        if (!payload.data || payload.data.length === 0) {
          const portName = config.port;
          // Clean up Rust-side port state so it can be reopened
          invokeCommand('serial_close', { port: portName }).catch(() => {});
          setConnected(false);
          // Refresh port list first, then set error after it completes
          refreshPortsRef.current().finally(() => {
            setErrorMsg(tRef.current('serialTool.error.portDisconnected', { port: portName }));
          });
          return;
        }
        processReceived(payload.data, false);
      });
      unlistenRef.current = unlisten;
    } catch (e) {
      console.error('[SerialTool] Failed to subscribe to serial events:', e);
    }
  }, [processReceived, config.port]);

  const stopListening = useCallback(() => {
    if (unlistenRef.current) {
      unlistenRef.current();
      unlistenRef.current = null;
    }
  }, []);

  useEffect(() => {
    if (connected) {
      startListening();
    } else {
      stopListening();
    }
  }, [connected, startListening, stopListening]);

  useEffect(() => {
    return () => {
      stopListening();
    };
  }, [stopListening]);

  const refreshPorts = useCallback(async () => {
    try {
      const list = await invokeCommand('serial_list_ports');
      setPorts(
        list.map((p) => ({
          name: p.name,
          description: p.description || p.manufacturer || p.name,
        }))
      );
      setConfig((prev) => {
        const lastPort = localStorage.getItem('serial-last-port');
        if (lastPort && list.some((p) => p.name === lastPort)) {
          return { ...prev, port: lastPort };
        }
        return prev;
      });
      if (list.length === 0) {
        setErrorMsg('No serial ports detected. Check device connection.');
      } else {
        setErrorMsg('');
      }
    } catch (e) {
      console.error('Failed to list ports:', e);
      setErrorMsg('Failed to enumerate serial ports.');
    }
  }, []);
  refreshPortsRef.current = refreshPorts;

  useEffect(() => {
    refreshPorts();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Auto-refresh: poll for new serial devices every 2s when enabled and not connected
  useEffect(() => {
    if (!autoRefresh || connected) return;
    let prevPorts: string[] = [];
    const timer = setInterval(async () => {
      try {
        const list = await invokeCommand('serial_list_ports');
        const currentPorts = list.map((p) => p.name);
        const newPorts = currentPorts.filter((p) => !prevPorts.includes(p));
        const lastPort = localStorage.getItem('serial-last-port');
        // Auto-select only: last-used port (if reappeared) or a brand-new port
        setConfig((prev) => {
          if (lastPort && currentPorts.includes(lastPort) && prev.port !== lastPort) {
            return { ...prev, port: lastPort };
          }
          if (newPorts.length > 0) {
            return { ...prev, port: newPorts[0] };
          }
          // If current port disappeared, clear it
          if (!currentPorts.includes(prev.port)) {
            return { ...prev, port: '' };
          }
          return prev;
        });
        prevPorts = currentPorts;
        setPorts(
          list.map((p) => ({
            name: p.name,
            description: p.description || p.manufacturer || p.name,
          }))
        );
      } catch {
        /* ignore */
      }
    }, 2000);
    return () => clearInterval(timer);
  }, [autoRefresh, connected]);

  const handleOpen = useCallback(async () => {
    if (!config.port) return;
    setErrorMsg('');
    setConnecting(true);
    try {
      await invokeCommand('serial_open', {
        port: config.port,
        baudRate: config.baudRate,
        dataBits: config.dataBits,
        stopBits: config.stopBits,
        parity: config.parity,
        flowControl: config.flowControl,
      });
      setConnected(true);
      localStorage.setItem('serial-last-port', config.port);
      txLineBufRef.current = '';
      rxLineBufRef.current = '';
      txLineActiveRef.current = false;
      ansiConverterRef.current = createAnsiConverter();
    } catch (e: unknown) {
      const raw = getErrorMessage(e);
      console.error('Failed to open port:', raw);
      const port = config.port;
      let friendlyMsg: string;
      if (raw.startsWith('PORT_BUSY:')) {
        friendlyMsg = t('serialTool.error.portBusy', { port });
      } else if (raw.startsWith('PORT_GONE:')) {
        friendlyMsg = t('serialTool.error.portGone', { port });
      } else if (raw.startsWith('PORT_ERROR:')) {
        const detail = raw.substring(raw.indexOf(':', 11) + 1);
        friendlyMsg = t('serialTool.error.portError', {
          port,
          detail: detail || t('serialTool.error.unknown'),
        });
      } else {
        friendlyMsg = t('serialTool.error.portGeneric', { port, message: raw });
      }
      setErrorMsg(friendlyMsg);
      setConnected(false);
    } finally {
      setConnecting(false);
    }
  }, [config, t]);

  const handleClose = useCallback(async () => {
    try {
      await invokeCommand('serial_close', { port: config.port });
    } catch {
      // Ignore close errors
    }
    setConnected(false);
    setErrorMsg('');
  }, [config.port]);

  const handleSend = useCallback(
    async (data: number[], skipEcho?: boolean) => {
      if (!connected || !config.port) return;
      try {
        // Apply checksum if enabled
        const finalData = applyChecksum(data, checksumConfig);
        await invokeCommand('serial_write', { port: config.port, data: finalData });
        // Echo sent data when timestamp is ON (shows 发 prefix)
        // When timestamp is OFF, device echo is sufficient & avoids double display
        if (!skipEcho && showTimestampRef.current) {
          processReceived(finalData, true);
        }
      } catch (e: unknown) {
        const msg = getErrorMessage(e);
        console.error('Send failed:', msg);
        setErrorMsg(`Send failed: ${msg}`);
      }
    },
    [connected, config.port, processReceived, checksumConfig]
  );

  /** Local echo — append sent data with optional timestamp/hex formatting */
  const handleLocalEcho = useCallback(
    (data: number[]) => {
      processReceived(data, true);
    },
    [processReceived]
  );

  const handleClear = useCallback(() => {
    if (receivedText) {
      setClearedText(receivedText);
      setCanRestore(true);
    }
    setReceivedText('');
    rawChunksRef.current = [];
    txLineBufRef.current = '';
    rxLineBufRef.current = '';
  }, [receivedText]);

  const handleRestore = useCallback(() => {
    setReceivedText(clearedText);
    setCanRestore(false);
  }, [clearedText]);

  /** Remove the last displayed character (for Backspace visual feedback) */
  const handleDeleteLastChar = useCallback(() => {
    setReceivedText((prev) => {
      if (prev.length === 0) return prev;
      if (prev.endsWith('\n')) return prev.slice(0, -1);
      return prev.slice(0, -1);
    });
  }, []);

  const handleSaveLog = useCallback(async () => {
    const now = new Date();
    const pad = (n: number) => String(n).padStart(2, '0');
    const defaultName = `LYNX_serial_${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}_${pad(now.getHours())}${pad(now.getMinutes())}${pad(now.getSeconds())}.txt`;
    try {
      const path = await save({
        defaultPath: defaultName,
        filters: [{ name: 'Text', extensions: ['txt', 'log'] }],
      });
      if (path) {
        await writeTextFile(path, receivedText);
      }
    } catch (e) {
      console.error('Save log failed:', e);
    }
  }, [receivedText]);

  return (
    <div className="serial-tool">
      <div className="serial-main-row">
        <div className="serial-monitor-section">
          <SerialMonitor
            text={receivedText}
            errorMsg={errorMsg}
            onSend={handleSend}
            onLocalEcho={handleLocalEcho}
            onClear={handleClear}
            onSaveLog={handleSaveLog}
            canRestore={canRestore}
            onRestore={handleRestore}
            onDeleteLastChar={handleDeleteLastChar}
            connected={connected}
            connecting={connecting}
            showTimestamp={showTimestamp}
          />
        </div>
        <MultiSendPanel onSend={handleSend} connected={connected} />
      </div>
      <div className={`serial-settings-section ${settingsCollapsed ? 'collapsed' : ''}`}>
        {settingsCollapsed ? (
          <button
            className="settings-collapse-btn"
            onClick={() => setSettingsCollapsed(false)}
            title="展开功能区"
          >
            <FontAwesomeIcon icon={faChevronUp} />
          </button>
        ) : (
          <>
            <button
              className="settings-collapse-btn"
              onClick={() => setSettingsCollapsed(true)}
              title="折叠功能区"
            >
              <FontAwesomeIcon icon={faChevronDown} />
            </button>
            <SerialSettings
              config={config}
              ports={ports}
              connected={connected}
              connecting={connecting}
              displayOptions={displayOptions}
              onConfigChange={setConfig}
              onOpen={handleOpen}
              onClose={handleClose}
              onRefresh={refreshPorts}
              autoRefresh={autoRefresh}
              onToggleAutoRefresh={() => setAutoRefresh((v) => !v)}
              onToggleTimestamp={handleToggleTimestamp}
              onToggleHex={handleToggleHex}
              checksumConfig={checksumConfig}
              onChecksumChange={setChecksumConfig}
              onChecksumPreview={checksumPreview}
              sendText={sendText}
              sendHexMode={sendHexMode}
              sendAppendNewline={sendAppendNewline}
            />
            <SendPanel
              onSend={handleSend}
              connected={connected}
              sendText={sendText}
              sendHexMode={sendHexMode}
              sendAppendNewline={sendAppendNewline}
              onSendTextChange={setSendText}
              onSendHexModeChange={setSendHexMode}
              onSendAppendNewlineChange={setSendAppendNewline}
            />
          </>
        )}
      </div>
    </div>
  );
};
