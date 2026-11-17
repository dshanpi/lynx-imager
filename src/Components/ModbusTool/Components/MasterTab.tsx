import React, { useState, useCallback, useRef, useEffect, useMemo } from 'react';
import { invokeCommand, subscribeEvent } from '../../../Platform/IPC';
import type { UnlistenFn } from '@tauri-apps/api/event';
import {
  buildRTUFrame,
  buildASCIIFrame,
  buildTCPFrame,
  parseRTUResponse,
  parseTCPResponse,
  buildPDUData,
  isReadFunc,
  isModbusError,
  formatRTUHex,
  responseToRegisters,
  responseToBits,
  getPLCAddressPrefix,
  FUNC_CODES,
  type ModbusResponse,
} from '../lib/ModbusProtocol';
import type { SlaveSimulator } from '../lib/SlaveSimulator';
import { formatErrorMessage } from '../../../Utils/Error';

interface MasterTabProps {
  slaveSimRef: React.MutableRefObject<SlaveSimulator | null>;
  slaveEnabled: boolean;
}

interface LogEntry {
  time: string;
  message: string;
  isError: boolean;
}

function formatTime(): string {
  const now = new Date();
  const h = String(now.getHours()).padStart(2, '0');
  const m = String(now.getMinutes()).padStart(2, '0');
  const s = String(now.getSeconds()).padStart(2, '0');
  const ms = String(now.getMilliseconds()).padStart(3, '0');
  return `${h}:${m}:${s}.${ms}`;
}

export const MasterTab: React.FC<MasterTabProps> = (_props) => {
  // Transport config
  const [protocol, setProtocol] = useState<'rtu' | 'tcp'>('rtu');
  const [serialPort, setSerialPort] = useState('');
  const [baudRate, setBaudRate] = useState('115200');
  const [_dataBits, _setDataBits] = useState('8');
  const [_stopBits, _setStopBits] = useState('1');
  const [parity, setParity] = useState('none');
  const [tcpHost, setTcpHost] = useState('127.0.0.1');
  const [tcpPort, setTcpPort] = useState('502');

  // Modbus read/write definition
  const [slaveId, setSlaveId] = useState('1');
  const [funcCode, setFuncCode] = useState('3');
  const [addrMode, setAddrMode] = useState<'dec' | 'hex'>('dec');
  const [startAddr, setStartAddr] = useState('0');
  const [quantity, setQuantity] = useState('10');
  const [writeDataHex, setWriteDataHex] = useState('');
  const [singleWriteVal, setSingleWriteVal] = useState('0');
  const [scanRate, setScanRate] = useState('1000');
  const [rwDisabled, setRwDisabled] = useState(false);
  const [disableOnError, setDisableOnError] = useState(false);
  const [scanning, setScanning] = useState(false);
  const scanTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // View options
  const [rowCount, setRowCount] = useState<'10' | '20' | '50' | '100' | 'fit'>('fit');
  const [hideNameCol, setHideNameCol] = useState(false);
  const [plcAddressMode, setPlcAddressMode] = useState(true);
  const [_enronMode, _setEnronMode] = useState(false);

  // Response data
  const [respRegisters, setRespRegisters] = useState<number[]>([]);
  const [respBits, setRespBits] = useState<number[]>([]);
  const [rtuRespHex, setRtuRespHex] = useState('');

  // Connection state
  const [connected, setConnected] = useState(false);
  const [connecting, setConnecting] = useState(false);
  const [ports, setPorts] = useState<Array<{ name: string; description: string }>>([]);
  const [autoRefresh, setAutoRefresh] = useState(false);
  const connIdRef = useRef<string>('');

  // Log
  const [logs, setLogs] = useState<LogEntry[]>(() => [
    {
      time: '--:--:--',
      message: 'Modbus Master ready.',
      isError: false,
    },
  ]);
  const logEndRef = useRef<HTMLDivElement>(null);
  const logListRef = useRef<HTMLDivElement>(null);

  const isNearBottom = useCallback(() => {
    const el = logListRef.current;
    if (!el) return true;
    return el.scrollHeight - el.scrollTop - el.clientHeight < 40;
  }, []);

  const addLog = useCallback(
    (message: string, isError = false) => {
      const entry: LogEntry = { time: formatTime(), message, isError };
      setLogs((prev) => {
        const next = [...prev, entry];
        if (next.length > 500) next.shift();
        return next;
      });
      setTimeout(() => {
        if (isNearBottom()) {
          logEndRef.current?.scrollIntoView({ behavior: 'smooth' });
        }
      }, 10);
    },
    [isNearBottom]
  );

  // Collapsible config
  const [configCollapsed, setConfigCollapsed] = useState(false);
  const [msgCollapsed, setMsgCollapsed] = useState(false);

  // ─── Serial port management ─────────────────────────

  const scanPorts = useCallback(async () => {
    try {
      const list = await invokeCommand('serial_list_ports');
      const mapped = list.map(
        (p: { name: string; description?: string; manufacturer?: string }) => ({
          name: p.name,
          description: p.description || p.manufacturer || p.name,
        })
      );
      setPorts(mapped);
      if (!serialPort) {
        const lastPort = localStorage.getItem('modbus-last-port');
        if (lastPort && list.some((p: { name: string }) => p.name === lastPort)) {
          setSerialPort(lastPort);
        } else if (list.length > 0) {
          setSerialPort(list[0].name);
        }
      }
    } catch {
      /* no ports */
    }
  }, [serialPort]);

  useEffect(() => {
    scanPorts();
  }, []);

  useEffect(() => {
    if (!autoRefresh || connected) return;
    const timer = setInterval(async () => {
      try {
        const list = await invokeCommand('serial_list_ports');
        const mapped = list.map(
          (p: { name: string; description?: string; manufacturer?: string }) => ({
            name: p.name,
            description: p.description || p.manufacturer || p.name,
          })
        );
        setPorts(mapped);
        const lastPort = localStorage.getItem('modbus-last-port');
        setSerialPort((prev) => {
          if (
            lastPort &&
            list.some((p: { name: string }) => p.name === lastPort) &&
            prev !== lastPort
          )
            return lastPort;
          if (list.length > 0 && !list.some((p: { name: string }) => p.name === prev))
            return list[0].name;
          return prev;
        });
      } catch {
        /* ignore */
      }
    }, 2000);
    return () => clearInterval(timer);
  }, [autoRefresh, connected]);

  const handleConnect = useCallback(async () => {
    if (protocol === 'rtu' && !serialPort) {
      addLog('No serial port selected', true);
      return;
    }
    setConnecting(true);
    let tcpId: string | undefined;
    try {
      if (protocol === 'rtu') {
        await invokeCommand('serial_open', {
          port: serialPort,
          baudRate: parseInt(baudRate, 10),
          dataBits: parseInt(_dataBits, 10),
          stopBits: parseInt(_stopBits, 10),
          parity,
          flowControl: 'none',
        });
        localStorage.setItem('modbus-last-port', serialPort);
        connIdRef.current = serialPort;
      } else {
        tcpId = `modbus-tcp-${Date.now()}`;
        await invokeCommand('tcp_connect', {
          id: tcpId,
          host: tcpHost,
          port: parseInt(tcpPort, 10),
        });
        // Record the id before tcp_start_read so a failure there can still be cleaned up
        connIdRef.current = tcpId;
        await invokeCommand('tcp_start_read', { id: tcpId });
      }
      // Clear rx buffers so stale bytes from a previous session don't get parsed
      rxBufRef.current = [];
      tcpRxBufRef.current = [];
      setConnected(true);
      addLog(
        `${protocol.toUpperCase()} connected — ${protocol === 'rtu' ? serialPort : `${tcpHost}:${tcpPort}`}`
      );
    } catch (e: unknown) {
      // Clean up a half-open TCP connection if tcp_start_read failed after tcp_connect
      if (protocol === 'tcp' && tcpId) {
        invokeCommand('tcp_close', { id: tcpId }).catch(() => {});
        if (connIdRef.current === tcpId) connIdRef.current = '';
      }
      const raw = formatErrorMessage(e);
      if (raw.includes('PORT_BUSY:') || raw.includes('already open')) {
        addLog(
          `Port ${serialPort} is occupied by another tool or application. Close it first.`,
          true
        );
      } else if (raw.includes('PORT_GONE:') || raw.includes('not found')) {
        addLog(`Port ${serialPort} not found. Check device connection and refresh.`, true);
      } else {
        addLog(`Connect failed: ${raw}`, true);
      }
    }
    setConnecting(false);
  }, [protocol, serialPort, baudRate, _dataBits, _stopBits, parity, tcpHost, tcpPort, addLog]);

  const handleDisconnect = useCallback(async () => {
    try {
      if (protocol === 'rtu') {
        await invokeCommand('serial_close', { port: connIdRef.current });
      } else {
        await invokeCommand('tcp_close', { id: connIdRef.current });
      }
    } catch {
      /* ignore */
    }
    setConnected(false);
    if (scanTimerRef.current) {
      clearInterval(scanTimerRef.current);
      scanTimerRef.current = null;
    }
    setScanning(false);
    addLog('Disconnected');
  }, [protocol, addLog]);

  // ─── Receive buffer (accumulate fragmented Modbus RTU frames) ──

  const rxBufRef = useRef<number[]>([]);
  const rxTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  // TCP is a byte stream: a single read() can return a partial MBAP frame or
  // multiple frames glued together. Accumulate here and split by MBAP length.
  const tcpRxBufRef = useRef<number[]>([]);

  const processRxBuffer = useCallback(() => {
    const buf = rxBufRef.current;
    rxBufRef.current = [];
    if (buf.length < 4) return; // Too short for a valid RTU frame
    const data = new Uint8Array(buf);
    const result = parseRTUResponse(data);
    const rawHex = formatRTUHex(data);
    addLog(`RX: ${rawHex}`);
    setRtuRespHex(rawHex);

    if (isModbusError(result)) {
      addLog(`RX Error: ${result.error}`, true);
      if (disableOnError) setRwDisabled(true);
      return;
    }
    const resp = result as ModbusResponse;
    const fc = parseInt(funcCode, 10);

    if (resp.funcCode === fc) {
      if (fc === 1 || fc === 2) {
        const bits = responseToBits(resp.data, parseInt(quantity, 10));
        setRespBits(bits);
        setRespRegisters([]);
      } else if (fc === 3 || fc === 4) {
        const regs = responseToRegisters(resp.data);
        setRespRegisters(regs);
        setRespBits([]);
      }
    }
  }, [funcCode, quantity, addLog, disableOnError]);

  // ─── Event listeners ────────────────────────────────

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let unlistenTcp: UnlistenFn | undefined;
    let unlistenTcpDisc: UnlistenFn | undefined;
    if (connected) {
      subscribeEvent('serial-data-received', (payload: { port: string; data: number[] }) => {
        if (!payload.data || payload.data.length === 0) {
          if (payload.port === connIdRef.current && protocol === 'rtu') {
            setConnected(false);
            if (scanTimerRef.current) {
              clearInterval(scanTimerRef.current);
              scanTimerRef.current = null;
            }
            setScanning(false);
            addLog(`Port ${payload.port} disconnected`, true);
          }
          return;
        }
        if (payload.port !== connIdRef.current || protocol !== 'rtu') return;
        // Buffer incoming bytes, parse after 50ms silence (Modbus RTU 3.5-char gap)
        rxBufRef.current.push(...payload.data);
        if (rxTimerRef.current) clearTimeout(rxTimerRef.current);
        rxTimerRef.current = setTimeout(() => processRxBuffer(), 50);
      }).then((fn) => {
        unlisten = fn;
      });
      subscribeEvent('tcp-data-received', (payload: { id: string; data: number[] }) => {
        if (!payload.data || payload.data.length === 0) return;
        if (payload.id !== connIdRef.current || protocol !== 'tcp') return;
        // TCP is a stream: accumulate bytes and split into complete MBAP frames by
        // length. A single read() can return a partial frame or several frames at once.
        tcpRxBufRef.current.push(...payload.data);
        while (tcpRxBufRef.current.length >= 8) {
          const buf = tcpRxBufRef.current;
          const len = (buf[4] << 8) | buf[5]; // MBAP length = bytes following the 6-byte header
          if (buf.length < 6 + len) break; // incomplete frame, wait for more bytes
          const frameBytes = buf.slice(0, 6 + len);
          tcpRxBufRef.current = buf.slice(6 + len);
          const data = new Uint8Array(frameBytes);
          const result = parseTCPResponse(data);
          const rawHex = formatRTUHex(data);
          addLog(`RX: ${rawHex}`);
          setRtuRespHex(rawHex);
          if (isModbusError(result)) {
            addLog(`RX Error: ${result.error}`, true);
            if (disableOnError) setRwDisabled(true);
            continue;
          }
          const resp = result as ModbusResponse;
          const fc = parseInt(funcCode, 10);
          if (resp.funcCode === fc) {
            if (fc === 1 || fc === 2) {
              setRespBits(responseToBits(resp.data, parseInt(quantity, 10)));
              setRespRegisters([]);
            } else if (fc === 3 || fc === 4) {
              setRespRegisters(responseToRegisters(resp.data));
              setRespBits([]);
            }
          }
        }
      }).then((fn) => {
        unlistenTcp = fn;
      });
      // TCP connection lost — reflect in UI. (RTU detects this via empty serial-data-received above.)
      subscribeEvent('tcp-disconnected', (payload: string) => {
        if (payload !== connIdRef.current || protocol !== 'tcp') return;
        setConnected(false);
        if (scanTimerRef.current) {
          clearInterval(scanTimerRef.current);
          scanTimerRef.current = null;
        }
        setScanning(false);
        addLog('TCP connection closed by remote', true);
      }).then((fn) => {
        unlistenTcpDisc = fn;
      });
    }
    return () => {
      unlisten?.();
      unlistenTcp?.();
      unlistenTcpDisc?.();
      if (rxTimerRef.current) clearTimeout(rxTimerRef.current);
      tcpRxBufRef.current = [];
    };
  }, [protocol, connected, processRxBuffer, funcCode, quantity, addLog, disableOnError]);

  // ─── Send/Receive ────────────────────────────────────

  const parseAddr = useCallback(
    (s: string): number => {
      const str = s.trim();
      if (addrMode === 'hex') {
        let h = str;
        if (h.toLowerCase().startsWith('0x')) h = h.slice(2);
        return parseInt(h, 16) || 0;
      }
      return parseInt(str, 10) || 0;
    },
    [addrMode]
  );

  // ─── Live preview: compute request frames from current params ──

  const previewFrames = useMemo(() => {
    const sid = parseInt(slaveId, 10) || 1;
    const fc = parseInt(funcCode, 10) || 3;
    const addr = parseAddr(startAddr);
    const qty = parseInt(quantity, 10) || 1;
    const read = isReadFunc(fc);
    let writeBytes: number[] = [];
    if (!read) {
      if (fc === 5 || fc === 6) {
        const val = parseInt(singleWriteVal, 10) || 0;
        writeBytes = [(val >> 8) & 0xff, val & 0xff];
      } else if (fc === 15 || fc === 16) {
        const hex = writeDataHex.replace(/\s/g, '');
        for (let i = 0; i < hex.length; i += 2) {
          const b = parseInt(hex.substring(i, i + 2), 16);
          if (!isNaN(b)) writeBytes.push(b);
        }
      }
    }
    const pduData = buildPDUData(fc, addr, qty, writeBytes);
    const rtuFrame = buildRTUFrame(sid, fc, pduData);
    const asciiFrame = buildASCIIFrame(sid, fc, pduData);
    const tcpFrame = buildTCPFrame(1, sid, fc, pduData);
    // Also compute hex bytes of the ASCII frame for display
    const asciiBytes = new TextEncoder().encode(asciiFrame);
    return {
      rtu: formatRTUHex(rtuFrame),
      ascii: formatRTUHex(asciiBytes),
      tcp: formatRTUHex(tcpFrame),
    };
  }, [slaveId, funcCode, startAddr, quantity, writeDataHex, singleWriteVal, addrMode, parseAddr]);

  const getDisplayRows = useCallback((): number => {
    if (rowCount === 'fit') return parseInt(quantity, 10) || 10;
    return parseInt(rowCount, 10);
  }, [rowCount, quantity]);

  const execReadWrite = useCallback(async () => {
    if (!connected || rwDisabled) return;
    try {
      const sid = parseInt(slaveId, 10);
      const fc = parseInt(funcCode, 10);
      const addr = parseAddr(startAddr);
      const qty = parseInt(quantity, 10);
      const read = isReadFunc(fc);

      // Build write bytes for write function codes
      let writeBytes: number[] = [];
      if (!read) {
        if (fc === 5 || fc === 6) {
          const val = parseInt(singleWriteVal, 10) || 0;
          writeBytes = [(val >> 8) & 0xff, val & 0xff];
        } else if (fc === 15 || fc === 16) {
          const hex = writeDataHex.replace(/\s/g, '');
          for (let i = 0; i < hex.length; i += 2) {
            const b = parseInt(hex.substring(i, i + 2), 16);
            if (!isNaN(b)) writeBytes.push(b);
          }
        }
      }

      const pduData = buildPDUData(fc, addr, qty, writeBytes);
      const frame = buildRTUFrame(sid, fc, pduData);

      // Update message preview
      setRtuRespHex('');

      const frameHex = formatRTUHex(frame);
      addLog(`TX: ${frameHex}`);

      if (protocol === 'rtu') {
        await invokeCommand('serial_write', { port: connIdRef.current, data: Array.from(frame) });
      } else {
        const tcpFrame = (await import('../lib/ModbusProtocol')).buildTCPFrame(
          Math.floor(Math.random() * 65535),
          sid,
          fc,
          pduData
        );
        await invokeCommand('tcp_send', { id: connIdRef.current, data: Array.from(tcpFrame) });
      }

      // Mirror the write into the local slave simulator so the Slave tab stays in sync.
      // processPDU applies the exact same decode a real slave would (0xFF00 coil value,
      // page-aligned multi-writes, address bounds checks) — reusing it avoids drift
      // between the frame we put on the wire and the sim's internal state.
      if (!read && _props.slaveSimRef.current && _props.slaveEnabled) {
        const sim = _props.slaveSimRef.current;
        if (sim.slaveId === sid) {
          sim.processPDU(new Uint8Array([sid, fc, ...pduData]));
        }
      }
    } catch (e: unknown) {
      const msg = formatErrorMessage(e);
      addLog(`TX Error: ${msg}`, true);
      if (disableOnError) setRwDisabled(true);
    }
  }, [
    connected,
    rwDisabled,
    slaveId,
    funcCode,
    startAddr,
    quantity,
    writeDataHex,
    singleWriteVal,
    protocol,
    parseAddr,
    addLog,
    disableOnError,
    _props,
  ]);

  // Keep a ref to the latest execReadWrite so the polling interval always invokes
  // the version with current parameter values (startAddr/quantity/writeData/...).
  // Without this, setInterval captures a stale closure and edits made mid-scan are
  // silently ignored until the scan is stopped and restarted.
  const execReadWriteRef = useRef(execReadWrite);
  useEffect(() => {
    execReadWriteRef.current = execReadWrite;
  }, [execReadWrite]);

  // ─── Scan / Polling ──────────────────────────────────

  const startScan = useCallback(() => {
    if (!connected || rwDisabled) return;
    setScanning(true);
    execReadWriteRef.current();
    scanTimerRef.current = setInterval(
      () => {
        execReadWriteRef.current();
      },
      Math.max(100, parseInt(scanRate, 10) || 1000)
    );
  }, [connected, rwDisabled, scanRate]);

  const stopScan = useCallback(() => {
    if (scanTimerRef.current) {
      clearInterval(scanTimerRef.current);
      scanTimerRef.current = null;
    }
    setScanning(false);
  }, []);

  useEffect(() => {
    return () => {
      if (scanTimerRef.current) clearInterval(scanTimerRef.current);
    };
  }, []);

  // ─── Data table edit ─────────────────────────────────

  const editRegister = useCallback((idx: number, val: number) => {
    setRespRegisters((prev) => {
      const next = [...prev];
      next[idx] = val & 0xffff;
      return next;
    });
  }, []);

  const editBit = useCallback((idx: number, val: number) => {
    setRespBits((prev) => {
      const next = [...prev];
      next[idx] = val ? 1 : 0;
      return next;
    });
  }, []);

  // ─── Log utilities ───────────────────────────────────

  const [clearedText, setClearedText] = useState('');
  const handleClearLog = useCallback(() => {
    const text = logs.map((e) => `[${e.time}] ${e.message}`).join('\n');
    if (text) setClearedText(text);
    setLogs([]);
  }, [logs]);

  const handleRestore = useCallback(() => {
    setLogs(
      clearedText.split('\n').map((line) => {
        const m = line.match(/^\[([^\]]+)\] (.*)/);
        return { time: m?.[1] ?? '--:--:--', message: m?.[2] ?? line, isError: false };
      })
    );
    setClearedText('');
  }, [clearedText]);

  // ─── Render helpers ──────────────────────────────────

  const fc = parseInt(funcCode, 10);
  const isRead = isReadFunc(fc);
  const plcPrefix = getPLCAddressPrefix(fc);
  const displayRows = getDisplayRows();
  const hasCoilData = fc === 1 || fc === 2;
  const hasRegisterData = fc === 3 || fc === 4;
  const addrVal = parseAddr(startAddr);

  const logText = logs.map((e) => `[${e.time}] ${e.message}`).join('\n');

  return (
    <div className="modbus-master-layout">
      {/* Middle area: Definition (left) + Data Table (right) */}
      <div className="modbus-master-middle">
        {/* ─── Left: Read/Write Definition ─────────────── */}
        <div className="modbus-def-panel">
          <div className="modbus-section-title">Read/Write Definition</div>

          <div className="modbus-def-grid">
            <div className="modbus-def-field">
              <label>Slave ID</label>
              <input
                className="modbus-def-input"
                type="number"
                value={slaveId}
                onChange={(e) => setSlaveId(e.target.value)}
                min={1}
                max={247}
                disabled={connected && scanning}
              />
            </div>
            <div className="modbus-def-field">
              <label>Function Code</label>
              <select
                className="modbus-def-input"
                value={funcCode}
                onChange={(e) => setFuncCode(e.target.value)}
                disabled={connected && scanning}
              >
                {Object.entries(FUNC_CODES).map(([k, v]) => (
                  <option key={k} value={k}>
                    {v}
                  </option>
                ))}
              </select>
            </div>
            <div className="modbus-def-field">
              <label>Address Mode</label>
              <select
                className="modbus-def-input"
                value={addrMode}
                onChange={(e) => setAddrMode(e.target.value as 'dec' | 'hex')}
              >
                <option value="dec">Decimal</option>
                <option value="hex">Hexadecimal</option>
              </select>
            </div>
            <div className="modbus-def-field">
              <label>Start Address ({addrMode === 'hex' ? 'Hex' : 'Dec'})</label>
              <input
                className="modbus-def-input"
                type="text"
                value={startAddr}
                onChange={(e) => setStartAddr(e.target.value)}
                placeholder={addrMode === 'hex' ? '0x0000' : '0'}
              />
            </div>
            <div className="modbus-def-field">
              <label>Quantity</label>
              <input
                className="modbus-def-input"
                type="number"
                value={quantity}
                onChange={(e) => setQuantity(e.target.value)}
                min={1}
                max={125}
              />
            </div>
          </div>

          <div className="modbus-scan-row">
            <div className="modbus-def-field" style={{ flex: 1 }}>
              <label>Scan Rate (ms)</label>
              <div style={{ display: 'flex', gap: 6 }}>
                <input
                  className="modbus-def-input"
                  type="number"
                  value={scanRate}
                  onChange={(e) => setScanRate(e.target.value)}
                  min={100}
                  max={60000}
                  step={100}
                  style={{ flex: 1 }}
                  disabled={scanning}
                />
              </div>
            </div>
          </div>

          {/* Write data input — shown for write function codes */}
          {!isRead && (
            <div className="modbus-def-grid">
              {fc === 5 ? (
                <div className="modbus-def-field">
                  <label>Coil Value</label>
                  <div style={{ display: 'flex', gap: 8 }}>
                    <button
                      className={`i2c-btn ${singleWriteVal === '1' ? 'primary' : ''}`}
                      onClick={() => setSingleWriteVal('1')}
                      style={{ flex: 1 }}
                    >
                      ON (FF 00)
                    </button>
                    <button
                      className={`i2c-btn ${singleWriteVal === '0' ? 'primary' : ''}`}
                      onClick={() => setSingleWriteVal('0')}
                      style={{ flex: 1 }}
                    >
                      OFF (00 00)
                    </button>
                  </div>
                </div>
              ) : fc === 6 ? (
                <div className="modbus-def-field">
                  <label>Register Value (0-65535)</label>
                  <input
                    className="modbus-def-input"
                    type="number"
                    value={singleWriteVal}
                    onChange={(e) => setSingleWriteVal(e.target.value)}
                    min={0}
                    max={65535}
                  />
                </div>
              ) : (
                <div className="modbus-def-field">
                  <label>Write Data (hex bytes, space-separated)</label>
                  <textarea
                    className="modbus-def-input"
                    style={{
                      resize: 'vertical',
                      fontFamily: "'Cascadia Code','Fira Code',Consolas,monospace",
                    }}
                    rows={2}
                    value={writeDataHex}
                    onChange={(e) => setWriteDataHex(e.target.value)}
                    placeholder="FF 00 01 02"
                  />
                </div>
              )}
            </div>
          )}

          <div className="modbus-def-checks">
            <label className="modbus-check-label">
              <input
                type="checkbox"
                checked={rwDisabled}
                onChange={(e) => {
                  setRwDisabled(e.target.checked);
                  if (e.target.checked) stopScan();
                }}
              />
              Read/Write Disabled
            </label>
            <label className="modbus-check-label">
              <input
                type="checkbox"
                checked={disableOnError}
                onChange={(e) => setDisableOnError(e.target.checked)}
              />
              Disable on Error
            </label>
          </div>

          <div className="modbus-def-actions">
            {isRead ? (
              <button
                className="i2c-btn primary"
                onClick={execReadWrite}
                disabled={!connected || rwDisabled || scanning}
              >
                Read
              </button>
            ) : (
              <button
                className="i2c-btn primary"
                onClick={execReadWrite}
                disabled={!connected || rwDisabled || scanning}
              >
                Write
              </button>
            )}
            {!scanning ? (
              <button
                className="i2c-btn primary"
                onClick={startScan}
                disabled={!connected || rwDisabled}
              >
                Poll
              </button>
            ) : (
              <button className="i2c-btn danger" onClick={stopScan}>
                Stop
              </button>
            )}
          </div>
        </div>

        {/* ─── Right: Data Table ────────────────────────── */}
        <div className="modbus-data-panel">
          <div
            className="modbus-section-title"
            style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}
          >
            <span>Data View</span>
            <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
              <select
                className="modbus-view-select"
                value={rowCount}
                onChange={(e) => setRowCount(e.target.value as typeof rowCount)}
              >
                <option value="10">10 rows</option>
                <option value="20">20 rows</option>
                <option value="50">50 rows</option>
                <option value="100">100 rows</option>
                <option value="fit">Fit to Quantity</option>
              </select>
              <label className="modbus-check-label">
                <input
                  type="checkbox"
                  checked={hideNameCol}
                  onChange={(e) => setHideNameCol(e.target.checked)}
                />
                Hide Names
              </label>
              <label className="modbus-check-label">
                <input
                  type="checkbox"
                  checked={plcAddressMode}
                  onChange={(e) => setPlcAddressMode(e.target.checked)}
                />
                PLC Addr (1-based)
              </label>
            </div>
          </div>

          <div className="modbus-data-table-wrap">
            {hasRegisterData || hasCoilData ? (
              <table className="modbus-data-table">
                <thead>
                  <tr>
                    <th className="modbus-tbl-addr">Address</th>
                    {!hideNameCol && <th className="modbus-tbl-name">Name</th>}
                    <th className="modbus-tbl-val">
                      {hasRegisterData ? 'Value (Hex/Dec)' : 'Value (0/1)'}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {Array.from({ length: displayRows }, (_, i) => {
                    const absAddr = addrVal + i;
                    const plcAddr = plcPrefix
                      ? `${plcPrefix}${String(absAddr + 1).padStart(4, '0')}`
                      : String(absAddr);
                    const regVal = respRegisters[i];
                    const bitVal = respBits[i];
                    return (
                      <tr key={i} className={scanning && i === 0 ? 'modbus-tbl-row-active' : ''}>
                        <td className="modbus-tbl-addr">
                          {plcAddressMode
                            ? plcAddr
                            : addrMode === 'hex'
                              ? `0x${absAddr.toString(16).toUpperCase().padStart(4, '0')}`
                              : String(absAddr)}
                        </td>
                        {!hideNameCol && (
                          <td className="modbus-tbl-name">
                            {hasRegisterData ? `Holding Reg ${absAddr}` : `Coil ${absAddr}`}
                          </td>
                        )}
                        <td className="modbus-tbl-val">
                          {hasRegisterData ? (
                            i < respRegisters.length ? (
                              <input
                                className="modbus-tbl-input"
                                type="number"
                                value={regVal ?? 0}
                                onChange={(e) => editRegister(i, parseInt(e.target.value, 10) || 0)}
                                min={0}
                                max={65535}
                              />
                            ) : (
                              <span className="modbus-tbl-empty">—</span>
                            )
                          ) : i < respBits.length ? (
                            <select
                              className="modbus-tbl-input"
                              value={bitVal ?? 0}
                              onChange={(e) => editBit(i, parseInt(e.target.value, 10))}
                            >
                              <option value={0}>0</option>
                              <option value={1}>1</option>
                            </select>
                          ) : (
                            <span className="modbus-tbl-empty">—</span>
                          )}
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            ) : (
              <div className="modbus-data-empty">
                Select a read function code (01-04) and click Read to view data
              </div>
            )}
          </div>

          {connected && scanning && (
            <div className="modbus-scan-status">
              <span className="status-led active" />
              Scanning every {scanRate}ms — {isRead ? 'Reading' : 'Writing'}{' '}
              {fc === 1 || fc === 2 ? 'Coils' : 'Registers'}
            </div>
          )}
        </div>
      </div>

      {/* ─── Message Preview (live-updating, selectable) ─── */}
      <div className={`modbus-msg-panel ${msgCollapsed ? 'collapsed' : ''}`}>
        <div
          className="modbus-msg-header"
          onClick={() => setMsgCollapsed(!msgCollapsed)}
          style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}
        >
          <span>{msgCollapsed ? '▶' : '▼'} Request / Response Messages</span>
          {!msgCollapsed && (
            <button
              className="serial-action-btn"
              onClick={(e) => {
                e.stopPropagation();
                const lines = [
                  `RTU:  ${previewFrames.rtu}`,
                  `ASCII: ${previewFrames.ascii}`,
                  `TCP:  ${previewFrames.tcp}`,
                  `Resp: ${rtuRespHex || '—'}`,
                ];
                navigator.clipboard.writeText(lines.join('\n')).catch(() => {});
              }}
            >
              Copy All
            </button>
          )}
        </div>
        {!msgCollapsed && (
          <div className="modbus-msg-grid">
            <div className="modbus-msg-field">
              <span className="modbus-msg-label">RTU Request:</span>
              <pre
                className="modbus-msg-pre"
                onMouseDown={() => {}} // Allow native text selection
              >
                {previewFrames.rtu || '—'}
              </pre>
            </div>
            <div className="modbus-msg-field">
              <span className="modbus-msg-label">ASCII Request:</span>
              <pre className="modbus-msg-pre">{previewFrames.ascii || '—'}</pre>
            </div>
            <div className="modbus-msg-field">
              <span className="modbus-msg-label">TCP Request:</span>
              <pre className="modbus-msg-pre">{previewFrames.tcp || '—'}</pre>
            </div>
            <div className="modbus-msg-field">
              <span className="modbus-msg-label">
                {protocol === 'tcp' ? 'TCP Response:' : 'RTU Response:'}
              </span>
              <pre
                className="modbus-msg-pre"
                style={{ color: rtuRespHex ? 'var(--color-text)' : 'var(--color-overlay0)' }}
              >
                {rtuRespHex || '— Waiting for response —'}
              </pre>
            </div>
          </div>
        )}
      </div>

      {/* ─── Bottom: Log + Connection ─────────────────────── */}
      <div className={`i2c-master-config ${configCollapsed ? 'collapsed' : ''}`}>
        {configCollapsed ? (
          <button
            className="i2c-config-collapse-btn"
            onClick={() => setConfigCollapsed(false)}
            title="Expand"
          >
            &#9650;
          </button>
        ) : (
          <>
            <button
              className="i2c-config-collapse-btn"
              style={{ position: 'absolute', top: 4, right: 8, zIndex: 1 }}
              onClick={() => setConfigCollapsed(true)}
              title="Collapse"
            >
              &#9660;
            </button>
            <div className="i2c-master-config-body">
              {/* Connection bar */}
              <div className="serial-config-row">
                <div className="config-group">
                  <label className="config-label">Protocol</label>
                  <select
                    className="config-select"
                    value={protocol}
                    onChange={(e) => setProtocol(e.target.value as 'rtu' | 'tcp')}
                    disabled={connected}
                  >
                    <option value="rtu">Modbus RTU (Serial)</option>
                    <option value="tcp">Modbus TCP</option>
                  </select>
                </div>
                <div className="config-group">
                  <label className="config-label">Port</label>
                  <div className="config-port-row">
                    <span
                      className={`status-led ${connected ? 'active' : ''}`}
                      style={{ marginRight: 4 }}
                    />
                    {protocol === 'rtu' ? (
                      <>
                        <select
                          className="config-select"
                          value={serialPort}
                          onChange={(e) => setSerialPort(e.target.value)}
                          disabled={connected}
                        >
                          <option value="">-- Select Port --</option>
                          {ports.map((p) => (
                            <option key={p.name} value={p.name}>
                              {p.name}
                              {p.description && p.description !== p.name
                                ? ` (${p.description})`
                                : ''}
                            </option>
                          ))}
                        </select>
                        <button
                          className="config-btn-icon"
                          onClick={scanPorts}
                          disabled={connected}
                          title="Refresh"
                        >
                          ↻
                        </button>
                        <button
                          className={`config-btn-icon ${autoRefresh ? 'active' : ''}`}
                          onClick={() => setAutoRefresh((v) => !v)}
                          disabled={connected}
                          title="Auto"
                        >
                          A
                        </button>
                      </>
                    ) : (
                      <>
                        <input
                          className="config-select"
                          type="text"
                          value={tcpHost}
                          onChange={(e) => setTcpHost(e.target.value)}
                          disabled={connected}
                          placeholder="Host"
                          style={{ width: 110 }}
                        />
                        <span
                          style={{ fontSize: 11, color: 'var(--color-overlay1)', margin: '0 4px' }}
                        >
                          :
                        </span>
                        <input
                          className="config-select"
                          type="number"
                          value={tcpPort}
                          onChange={(e) => setTcpPort(e.target.value)}
                          disabled={connected}
                          placeholder="502"
                          style={{ width: 65 }}
                          min={1}
                          max={65535}
                        />
                      </>
                    )}
                  </div>
                </div>
                {protocol === 'rtu' && (
                  <>
                    <div className="config-group">
                      <label className="config-label">Baud</label>
                      <select
                        className="config-select"
                        value={baudRate}
                        onChange={(e) => setBaudRate(e.target.value)}
                        disabled={connected}
                      >
                        {[
                          '300',
                          '1200',
                          '2400',
                          '4800',
                          '9600',
                          '19200',
                          '38400',
                          '57600',
                          '115200',
                          '230400',
                          '460800',
                          '921600',
                        ].map((b) => (
                          <option key={b} value={b}>
                            {b}
                          </option>
                        ))}
                      </select>
                    </div>
                    <div className="config-group">
                      <label className="config-label">Parity</label>
                      <select
                        className="config-select"
                        value={parity}
                        onChange={(e) => setParity(e.target.value)}
                        disabled={connected}
                      >
                        <option value="none">8N1</option>
                        <option value="even">8E1</option>
                        <option value="odd">8O1</option>
                      </select>
                    </div>
                  </>
                )}
                <div className="config-group config-connect">
                  <label className="config-label">&nbsp;</label>
                  {!connected ? (
                    <button
                      className="config-btn-open"
                      onClick={handleConnect}
                      disabled={connecting || (protocol === 'rtu' && !serialPort)}
                    >
                      {connecting ? '...' : 'Connect'}
                    </button>
                  ) : (
                    <button className="config-btn-close" onClick={handleDisconnect}>
                      Disconnect
                    </button>
                  )}
                </div>
              </div>

              {/* Log */}
              <div className="i2c-log-area">
                <div className="i2c-log-title">
                  <span className="status-led active" />
                  Bus Log
                  <div className="i2c-log-actions">
                    <button className="serial-action-btn" onClick={handleClearLog}>
                      Clear
                    </button>
                    {clearedText && (
                      <button className="serial-action-btn" onClick={handleRestore}>
                        Restore
                      </button>
                    )}
                  </div>
                </div>
                <div className="i2c-log-list i2c-log-interactive" ref={logListRef}>
                  <pre className="i2c-log-pre">{logText || 'Ready'}</pre>
                  <div ref={logEndRef} />
                </div>
              </div>
            </div>
          </>
        )}
      </div>
    </div>
  );
};
