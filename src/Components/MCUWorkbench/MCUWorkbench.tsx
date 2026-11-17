import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { LynxIcon, type LynxIconName } from '../../CoreUI';
import { ask, open } from '@tauri-apps/plugin-dialog';
import {
  invokeCommand,
  McuBoardConfig,
  McuChipSeries,
  McuChipVendor,
  McuJLinkConnection,
  McuOpenOcdConnection,
  McuPyOcdConnection,
  McuMcpTunnelStatus,
  McuProbeBackend,
  McuStLinkConnection,
  McuStLinkProbe,
  McuTargetStatus,
  McuToolInfo,
  normalizeIpcError,
  subscribeEvent,
} from '../../Platform/IPC';
import { DeviceAiChannel } from '../DeviceAiChannel';
import { appendRecordEvent } from '../RecordCenter';
import { BoardThumbnail } from '../Workbench/BoardThumbnail';
import {
  MCU_CHIP_SERIES as CHIP_SERIES,
  MCU_CHIP_VENDORS as CHIP_VENDORS,
  applyMcuChipSeries as applyChipSeries,
  createDefaultMcuBoard as defaultBoard,
  externalFlashCapacity,
  externalSramCapacity,
  mcuBoardOnline,
  mcuBoardSpecs,
  mcuMcpName,
  normalizeMcuBoardIdentity as normalizeBoardIdentity,
  parseMcuUnsigned as parseUnsigned,
} from './mcuWorkbenchModel';
import './MCUWorkbench.css';
import '../Workbench/Workbench.css';

interface MCUWorkbenchProps {
  isActive: boolean;
}

type McuDeviceView = 'overview' | 'serial' | 'probe' | 'flash' | 'ai';

type SerialPort = Awaited<ReturnType<typeof listSerialPorts>>[number];
type LynxPowerDevice = Awaited<ReturnType<typeof listLynxPowerDevices>>[number];
type LogKind = 'dim' | 'info' | 'ok' | 'warn' | 'error';

interface McuLogLine {
  id: number;
  kind: LogKind;
  text: string;
}

const BAUD_RATES = [9600, 57600, 115200, 460800, 921600, 1500000];
const DEFAULT_OSPI_START = '0x0';
const ACTIVE_BOARD_STORAGE_KEY = 'lynx.mcu.activeBoardId';

async function listSerialPorts() {
  return invokeCommand('serial_list_ports');
}

async function listLynxPowerDevices() {
  return invokeCommand('ch347_list_devices');
}

export const MCUWorkbench: React.FC<MCUWorkbenchProps> = ({ isActive }) => {
  const [boards, setBoards] = useState<McuBoardConfig[]>([]);
  const [activeBoardId, setActiveBoardId] = useState('');
  const [boardDraft, setBoardDraft] = useState<McuBoardConfig>(defaultBoard);
  const [boardSaving, setBoardSaving] = useState(false);
  const [boardSaveFeedback, setBoardSaveFeedback] = useState('');
  const [ports, setPorts] = useState<SerialPort[]>([]);
  const [powerDevices, setPowerDevices] = useState<LynxPowerDevice[]>([]);
  const [powerBusy, setPowerBusy] = useState(false);
  const [powerResult, setPowerResult] = useState('');
  const [selectedPort, setSelectedPort] = useState('');
  const [baudRate, setBaudRate] = useState(115200);
  const [serialLoading, setSerialLoading] = useState(false);
  const [serialConnected, setSerialConnected] = useState(false);
  const [serialAutoConnectPaused, setSerialAutoConnectPaused] = useState(false);
  const [serialInput, setSerialInput] = useState('');
  const [cliPath, setCliPath] = useState('');
  const [toolInfo, setToolInfo] = useState<McuToolInfo | null>(null);
  const [probes, setProbes] = useState<McuStLinkProbe[]>([]);
  const [selectedProbeKey, setSelectedProbeKey] = useState('');
  const [targetStatus, setTargetStatus] = useState<McuTargetStatus | null>(null);
  const [frequencyKhz, setFrequencyKhz] = useState(1000);
  const [externalLoaderPath, setExternalLoaderPath] = useState('');
  const [ospiStart, setOspiStart] = useState(DEFAULT_OSPI_START);
  const [ospiSizeMiB, setOspiSizeMiB] = useState('');
  const [binStart, setBinStart] = useState(DEFAULT_OSPI_START);
  const [settingsVisible, setSettingsVisible] = useState(false);
  const [activeView, setActiveView] = useState<McuDeviceView>('overview');
  const [boardSearch, setBoardSearch] = useState('');
  const [advancedSettings, setAdvancedSettings] = useState(false);
  const [operationBusy, setOperationBusy] = useState(false);
  const [operationLabel, setOperationLabel] = useState('空闲');
  const [mcpPassword, setMcpPassword] = useState('');
  const [mcuAiProvider, setMcuAiProvider] = useState<'codex' | 'claude_code'>('codex');
  const [mcpBusy, setMcpBusy] = useState(false);
  const [mcpTunnel, setMcpTunnel] = useState<McuMcpTunnelStatus | null>(null);
  const [logsByBoard, setLogsByBoard] = useState<Record<string, McuLogLine[]>>({});
  const [boardsLoaded, setBoardsLoaded] = useState(false);
  const openedSerialsRef = useRef(new Map<string, { port: string; baudRate: number }>());
  const serialFailureFingerprintRef = useRef(new Map<string, string>());
  const pausedSerialBoardsRef = useRef(new Set<string>());
  const serialOpeningRef = useRef(false);
  const serialReconcileRunningRef = useRef(false);
  const autoIdentifyKeyRef = useRef('');
  const serialDecodersRef = useRef(new Map<string, TextDecoder>());
  const terminalRef = useRef<HTMLDivElement>(null);
  const logs = useMemo(() => logsByBoard[activeBoardId] ?? [], [activeBoardId, logsByBoard]);

  const mcuChannelName = `${boardDraft.name || boardDraft.mcuModel} MCU SDK`;
  const activeMcuMcpName = mcuMcpName(mcuChannelName);
  const activeMcuRemotePort = mcpTunnel?.remotePort || boardDraft.mcpRemotePort;
  const mcuRemoteEndpoint = `http://127.0.0.1:${activeMcuRemotePort}/mcp`;
  const mcuClaudeConfig = JSON.stringify({
    mcpServers: { [activeMcuMcpName]: { type: 'http', url: mcuRemoteEndpoint } },
  });
  const mcuCodexCommand = `codex -c 'mcp_servers={"${activeMcuMcpName}"={url="${mcuRemoteEndpoint}"}}'`;
  const mcuClaudeCommand = `claude --strict-mcp-config --mcp-config '${mcuClaudeConfig}'`;
  const mcuLaunchCommand = mcuAiProvider === 'claude_code' ? mcuClaudeCommand : mcuCodexCommand;
  const persistableBoard = useMemo(() => {
    const start = ospiStart.trim() ? parseUnsigned(ospiStart) : 0;
    const size = ospiSizeMiB.trim() ? parseUnsigned(ospiSizeMiB) : 0;
    if (!activeBoardId || !boardDraft.name.trim() || start == null || size == null) return null;
    return {
      ...boardDraft,
      id: activeBoardId,
      serialPort: selectedPort,
      baudRate,
      toolPath: cliPath,
      probeSerial: selectedProbeKey.startsWith('index:') ? '' : selectedProbeKey,
      swdFrequencyKhz: frequencyKhz,
      externalLoaderPath,
      ospiAddressStart: start,
      ospiSizeBytes: size * 1024 * 1024,
      mcpServerName: activeMcuMcpName,
    };
  }, [
    activeBoardId,
    activeMcuMcpName,
    baudRate,
    boardDraft,
    cliPath,
    externalLoaderPath,
    frequencyKhz,
    ospiSizeMiB,
    ospiStart,
    selectedPort,
    selectedProbeKey,
  ]);

  const appendBoardLog = useCallback((boardId: string, kind: LogKind, text: string) => {
    if (!boardId) return;
    setLogsByBoard((current) => ({
      ...current,
      [boardId]: [
        ...(current[boardId] ?? []).slice(-80),
        { id: Date.now() + Math.random(), kind, text },
      ],
    }));
  }, []);

  const appendLog = useCallback(
    (kind: LogKind, text: string) => appendBoardLog(activeBoardId, kind, text),
    [activeBoardId, appendBoardLog]
  );

  const copyMcuCommand = useCallback(
    async (value: string) => {
      await navigator.clipboard.writeText(value);
      appendLog('ok', '[MCU MCP] 配置命令已复制');
    },
    [appendLog]
  );

  const reportError = useCallback(
    (context: string, error: unknown) => {
      const normalized = normalizeIpcError(error);
      appendLog('error', `[${context}] ${normalized.name}: ${normalized.message}`);
    },
    [appendLog]
  );

  useEffect(() => {
    if (!isActive || !activeBoardId) return;
    const refresh = () =>
      void invokeCommand('mcu_mcp_tunnel_status', { boardId: activeBoardId })
        .then(setMcpTunnel)
        .catch(() => setMcpTunnel(null));
    refresh();
    const timer = window.setInterval(refresh, 2000);
    return () => window.clearInterval(timer);
  }, [activeBoardId, isActive]);

  useEffect(() => {
    if (!boardsLoaded || !persistableBoard) return;
    const timer = window.setTimeout(() => {
      void invokeCommand('mcu_save_board', { board: persistableBoard })
        .then((saved) => {
          setBoards((current) => [...current.filter((board) => board.id !== saved.id), saved]);
          setBoardSaveFeedback('配置已自动保存');
        })
        .catch((error) => reportError('BOARD AUTO SAVE', error));
    }, 600);
    return () => window.clearTimeout(timer);
  }, [boardsLoaded, persistableBoard, reportError]);

  const startMcuMcp = useCallback(
    async (targetBoard = boardDraft, targetBoardId = activeBoardId) => {
      if (!targetBoardId || !targetBoard.mcpHostAddress.trim()) return;
      setMcpBusy(true);
      try {
        await invokeCommand('mcu_mcp_gateway_start', {
          request: { boardId: targetBoardId, port: targetBoard.mcpLocalPort },
        });
        const tunnel = await invokeCommand('mcu_mcp_tunnel_start', {
          request: {
            boardId: targetBoardId,
            hostAddress: targetBoard.mcpHostAddress,
            hostPort: targetBoard.mcpHostPort,
            hostUsername: targetBoard.mcpHostUsername,
            authType: targetBoard.mcpAuthType,
            privateKeyPath: targetBoard.mcpPrivateKeyPath,
            hostFingerprint: targetBoard.mcpHostFingerprint,
            password: mcpPassword || undefined,
            localPort: targetBoard.mcpLocalPort,
            remotePort: targetBoard.mcpRemotePort,
            mcpServerName: targetBoard.mcpServerName,
          },
        });
        setMcpTunnel(tunnel);
        appendLog('ok', '[MCU MCP] 专用 Gateway 与 SSH 隧道已启动；Linux/ADB/SDK 工具未加载');
      } catch (error) {
        reportError('MCU MCP START', error);
      } finally {
        setMcpBusy(false);
      }
    },
    [activeBoardId, appendLog, boardDraft, mcpPassword, reportError]
  );

  const stopMcuMcp = useCallback(async () => {
    if (!activeBoardId) return;
    setMcpBusy(true);
    try {
      await invokeCommand('mcu_mcp_tunnel_stop', { boardId: activeBoardId });
      await invokeCommand('mcu_mcp_gateway_stop', { port: boardDraft.mcpLocalPort });
      setMcpTunnel(null);
    } catch (error) {
      reportError('MCU MCP STOP', error);
    } finally {
      setMcpBusy(false);
    }
  }, [activeBoardId, boardDraft.mcpLocalPort, reportError]);

  const applyBoard = useCallback((board: McuBoardConfig) => {
    setBoardDraft({
      ...normalizeBoardIdentity(board),
      powerGpio: board.powerGpio >= 4 && board.powerGpio <= 7 ? board.powerGpio : 4,
    });
    setSelectedPort(board.serialPort);
    setSerialAutoConnectPaused(pausedSerialBoardsRef.current.has(board.id));
    setBaudRate(board.baudRate);
    setCliPath(board.toolPath);
    setSelectedProbeKey(board.probeSerial);
    setFrequencyKhz(board.swdFrequencyKhz);
    setExternalLoaderPath(board.externalLoaderPath);
    setOspiStart(`0x${board.ospiAddressStart.toString(16).toUpperCase()}`);
    setOspiSizeMiB(board.ospiSizeBytes ? String(board.ospiSizeBytes / 1024 / 1024) : '');
    setBinStart(`0x${board.ospiAddressStart.toString(16).toUpperCase()}`);
    setToolInfo(null);
    setTargetStatus(null);
    setSerialConnected(openedSerialsRef.current.has(board.id));
  }, []);

  const loadBoards = useCallback(async () => {
    try {
      const nextBoards = await invokeCommand('mcu_list_boards');
      setBoards(nextBoards);
      const rememberedBoardId =
        activeBoardId || localStorage.getItem(ACTIVE_BOARD_STORAGE_KEY) || '';
      const selected = nextBoards.find((board) => board.id === rememberedBoardId) ?? nextBoards[0];
      if (selected) {
        setActiveBoardId(selected.id);
        localStorage.setItem(ACTIVE_BOARD_STORAGE_KEY, selected.id);
        applyBoard(selected);
      } else {
        setActiveBoardId('');
        localStorage.removeItem(ACTIVE_BOARD_STORAGE_KEY);
        applyBoard(defaultBoard());
      }
    } catch (error) {
      reportError('BOARD', error);
    } finally {
      setBoardsLoaded(true);
    }
  }, [activeBoardId, applyBoard, reportError]);

  const refreshSerialPorts = useCallback(async () => {
    setSerialLoading(true);
    try {
      const nextPorts = await listSerialPorts();
      setPorts(nextPorts);
    } catch (error) {
      reportError('SERIAL', error);
      setPorts([]);
    } finally {
      setSerialLoading(false);
    }
  }, [reportError]);

  const bindSerialPort = useCallback(
    (name: string) => {
      const selected = ports.find((port) => port.name === name);
      setSelectedPort(name);
      setSerialAutoConnectPaused(false);
      setBoardDraft((board) => ({
        ...board,
        serialPort: name,
        serialVid: selected?.vid ?? 0,
        serialPid: selected?.pid ?? 0,
        serialNumber: selected?.serialNumber ?? '',
        serialInterfaceNumber: selected?.interfaceNumber ?? null,
      }));
    },
    [ports]
  );

  const setMcuPower = useCallback(
    async (action: 'on' | 'off' | 'cycle') => {
      if (boardDraft.powerControllerType !== 'ch347_gpio') return;
      setPowerBusy(true);
      const index = boardDraft.powerDeviceIndex;
      try {
        await invokeCommand('mcu_ch347_power_action', {
          request: {
            boardId: activeBoardId,
            deviceIndex: index,
            gpio: boardDraft.powerGpio,
            activeHigh: boardDraft.powerActiveHigh,
            action,
            offTimeMs: action === 'cycle' ? 800 : undefined,
          },
        });
        setPowerResult(action === 'off' ? '电源已关闭' : '电源已开启');
      } catch (error) {
        reportError('LYNX POWER', error);
        setPowerResult('控制失败');
      } finally {
        setPowerBusy(false);
      }
    },
    [
      activeBoardId,
      boardDraft.powerActiveHigh,
      boardDraft.powerControllerType,
      boardDraft.powerDeviceIndex,
      boardDraft.powerGpio,
      reportError,
    ]
  );

  const cycleMcuPower = useCallback(async () => {
    await setMcuPower('cycle');
  }, [setMcuPower]);

  const refreshBackend = useCallback(async () => {
    autoIdentifyKeyRef.current = '';
    setOperationBusy(true);
    setOperationLabel('检测烧写工具');
    try {
      const request = { executablePath: cliPath || undefined };
      if (boardDraft.probeBackend === 'openOcd') {
        const info = await invokeCommand('mcu_openocd_tool_info', { request });
        setToolInfo(info);
        setProbes([]);
        setSelectedProbeKey(boardDraft.probeSerial);
        appendLog(
          'ok',
          `[OPENOCD] ${info.version || 'runtime ready'}${info.bundled ? ' · bundled' : ''}`
        );
        return;
      }
      if (boardDraft.probeBackend === 'pyOcd') {
        const info = await invokeCommand('mcu_pyocd_tool_info', { request });
        setToolInfo(info);
        setProbes([]);
        setSelectedProbeKey(boardDraft.probeSerial);
        appendLog('ok', `[PYOCD] ${info.version || 'pyOCD ready'} · CMSIS-DAP`);
        return;
      }
      if (boardDraft.probeBackend === 'jLink') {
        const info = await invokeCommand('mcu_jlink_tool_info', { request });
        setToolInfo(info);
        setProbes([]);
        setSelectedProbeKey(boardDraft.probeSerial);
        appendLog('ok', `[J-LINK] ${info.version || 'SEGGER J-Link Commander ready'}`);
        return;
      }
      const info = await invokeCommand('mcu_stlink_tool_info', { request });
      setToolInfo(info);
      appendLog('ok', `[TOOL] ${info.version || 'STM32CubeProgrammer CLI ready'}`);
      const nextProbes = await invokeCommand('mcu_stlink_list_probes', { request });
      setProbes(nextProbes);
      setSelectedProbeKey((current) => {
        if (current && nextProbes.some((probe) => probe.serialNumber === current)) return current;
        if (nextProbes.length !== 1) return '';
        const onlyProbe = nextProbes[0];
        return onlyProbe.serialNumber || `index:${onlyProbe.index}`;
      });
      appendLog(
        nextProbes.length ? 'ok' : 'warn',
        `[PROBE] ${nextProbes.length ? `found ${nextProbes.length} ST-Link probe(s)` : 'no ST-Link probe detected'}`
      );
    } catch (error) {
      setToolInfo(null);
      setProbes([]);
      setTargetStatus(null);
      reportError('TOOL', error);
    } finally {
      setOperationBusy(false);
      setOperationLabel('空闲');
    }
  }, [appendLog, boardDraft.probeBackend, boardDraft.probeSerial, cliPath, reportError]);
  const refreshHardwareInterfaces = useCallback(async () => {
    await Promise.all([
      refreshSerialPorts(),
      listLynxPowerDevices()
        .then(setPowerDevices)
        .catch(() => setPowerDevices([])),
      refreshBackend(),
    ]);
  }, [refreshBackend, refreshSerialPorts]);
  const refreshBackendRef = useRef(refreshBackend);
  refreshBackendRef.current = refreshBackend;

  const closeSerial = useCallback(async () => {
    const connection = openedSerialsRef.current.get(activeBoardId);
    if (!connection) return;
    openedSerialsRef.current.delete(activeBoardId);
    setSerialConnected(false);
    serialDecodersRef.current.delete(activeBoardId);
    appendLog('dim', `[SERIAL] 已暂停界面连接，物理串口保持打开 ${connection.port}`);
  }, [activeBoardId, appendLog, boardDraft.name]);

  const openSerialConnection = useCallback(async () => {
    if (!activeBoardId || !selectedPort || serialOpeningRef.current) return false;
    const existing = openedSerialsRef.current.get(activeBoardId);
    if (existing) {
      const isOpen = await invokeCommand('serial_is_open', { port: existing.port }).catch(
        () => false
      );
      if (isOpen && existing.baudRate === baudRate) {
        setSerialConnected(true);
        return true;
      }
      openedSerialsRef.current.delete(activeBoardId);
      serialDecodersRef.current.delete(activeBoardId);
    }
    serialOpeningRef.current = true;
    setSerialLoading(true);
    const auditSessionId = `serial-mcu-${activeBoardId}-${Date.now()}`;
    try {
      const resolved = await invokeCommand('serial_wait_and_open', {
        identity: {
          port: selectedPort,
          vid: boardDraft.serialVid,
          pid: boardDraft.serialPid,
          serialNumber: boardDraft.serialNumber,
          interfaceNumber: boardDraft.serialInterfaceNumber,
        },
        baudRate,
        dataBits: 8,
        stopBits: 1,
        parity: 'none',
        flowControl: 'none',
        timeoutMs: 1200,
        auditContext: {
          actorType: 'user',
          actorId: 'local-user',
          scopeType: 'mcu_board',
          scopeId: `mcu:${activeBoardId}`,
          sessionId: auditSessionId,
          sessionTitle: `${boardDraft.name} · ${selectedPort || '串口'}`,
        },
      });
      openedSerialsRef.current.set(activeBoardId, { port: resolved.name, baudRate });
      if (!serialDecodersRef.current.has(activeBoardId)) {
        serialDecodersRef.current.set(activeBoardId, new TextDecoder());
      }
      if (resolved.name !== selectedPort) setSelectedPort(resolved.name);
      setSerialConnected(true);
      appendLog('ok', `[SERIAL] opened ${resolved.name} @ ${baudRate} 8N1`);
      serialFailureFingerprintRef.current.delete(activeBoardId);
      return true;
    } catch (error) {
      reportError('SERIAL OPEN', error);
      const fingerprint = `${selectedPort}:${baudRate}`;
      if (serialFailureFingerprintRef.current.get(activeBoardId) !== fingerprint) {
        serialFailureFingerprintRef.current.set(activeBoardId, fingerprint);
        void appendRecordEvent({
          actorType: 'service',
          actorId: 'serial-manager',
          category: 'serial',
          action: 'serial.connect_failed',
          scopeType: 'board',
          scopeId: `mcu:${activeBoardId}`,
          result: 'failed',
          errorCode: 'SERIAL_OPEN_FAILED',
          summary: `${boardDraft.name} 串口连接失败`,
          metadata: { requestedPort: selectedPort, baudRate, boardName: boardDraft.name },
        }).catch((recordError) => console.warn('Failed to record MCU serial failure', recordError));
      }
      return false;
    } finally {
      serialOpeningRef.current = false;
      setSerialLoading(false);
    }
  }, [activeBoardId, appendLog, baudRate, boardDraft, reportError, selectedPort]);

  const toggleSerial = useCallback(async () => {
    if (serialConnected) {
      pausedSerialBoardsRef.current.add(activeBoardId);
      setSerialAutoConnectPaused(true);
      await closeSerial();
      return;
    }
    pausedSerialBoardsRef.current.delete(activeBoardId);
    setSerialAutoConnectPaused(false);
    await openSerialConnection();
  }, [activeBoardId, closeSerial, openSerialConnection, serialConnected]);

  const sendSerialInput = useCallback(async () => {
    if (!serialInput || !selectedPort) return;
    if (!openedSerialsRef.current.has(activeBoardId) && !(await openSerialConnection())) return;
    const connection = openedSerialsRef.current.get(activeBoardId);
    if (!connection) return;
    try {
      const data = Array.from(new TextEncoder().encode(`${serialInput}\r\n`));
      await invokeCommand('serial_write', { port: connection.port, data });
      appendLog('dim', `[TX] ${serialInput}`);
      setSerialInput('');
    } catch (error) {
      reportError('SERIAL WRITE', error);
    }
  }, [activeBoardId, appendLog, openSerialConnection, reportError, selectedPort, serialInput]);

  useEffect(() => {
    if (boardsLoaded) return;
    void loadBoards();
  }, [boardsLoaded, loadBoards]);

  useEffect(() => {
    if (!isActive || !boardsLoaded) return;
    void refreshHardwareInterfaces();
  }, [boardsLoaded, isActive, refreshHardwareInterfaces]);

  useEffect(() => {
    if (!isActive || !activeBoardId) return;
    const timer = window.setInterval(() => {
      void listSerialPorts()
        .then(setPorts)
        .catch(() => undefined);
    }, 3000);
    return () => window.clearInterval(timer);
  }, [activeBoardId, isActive]);

  useEffect(() => {
    if (!isActive || !boardsLoaded) return;
    let cancelled = false;
    const reconcileBoardUarts = async () => {
      if (serialReconcileRunningRef.current) return;
      serialReconcileRunningRef.current = true;
      try {
        for (const board of boards) {
          if (cancelled) break;
          try {
            if (
              pausedSerialBoardsRef.current.has(board.id) ||
              (!board.serialPort && !board.serialNumber && !board.serialVid && !board.serialPid)
            ) {
              continue;
            }
            const existing = openedSerialsRef.current.get(board.id);
            if (existing) {
              const open = await invokeCommand('serial_is_open', { port: existing.port }).catch(
                () => false
              );
              if (open) continue;
              openedSerialsRef.current.delete(board.id);
            }
            const auditSessionId = `serial-mcu-${board.id}-${Date.now()}`;
            const resolved = await invokeCommand('serial_wait_and_open', {
              identity: {
                port: board.serialPort,
                vid: board.serialVid,
                pid: board.serialPid,
                serialNumber: board.serialNumber,
                interfaceNumber: board.serialInterfaceNumber,
              },
              baudRate: board.baudRate,
              dataBits: 8,
              stopBits: 1,
              parity: 'none',
              flowControl: 'none',
              timeoutMs: 250,
              auditContext: {
                actorType: 'service',
                actorId: 'mcu-workbench',
                scopeType: 'mcu_board',
                scopeId: `mcu:${board.id}`,
                sessionId: auditSessionId,
                sessionTitle: `${board.name} · ${board.serialPort || '串口'}`,
              },
            });
            if (cancelled) break;
            openedSerialsRef.current.set(board.id, {
              port: resolved.name,
              baudRate: board.baudRate,
            });
            if (!serialDecodersRef.current.has(board.id)) {
              serialDecodersRef.current.set(board.id, new TextDecoder());
            }
            if (board.id === activeBoardId) setSerialConnected(true);
          } catch {
            if (board.id === activeBoardId && !cancelled) setSerialConnected(false);
          }
        }
      } finally {
        serialReconcileRunningRef.current = false;
      }
    };
    void reconcileBoardUarts();
    const timer = window.setInterval(() => {
      if (!cancelled) void reconcileBoardUarts();
    }, 3000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [activeBoardId, boards, boardsLoaded, isActive]);

  useEffect(() => {
    if (!activeBoardId || !selectedPort) return;
    if (boardDraft.serialNumber || boardDraft.serialVid || boardDraft.serialPid) return;
    const exact = ports.find((port) => port.name === selectedPort);
    if (!exact) return;
    setBoardDraft((board) => ({
      ...board,
      serialVid: exact.vid,
      serialPid: exact.pid,
      serialNumber: exact.serialNumber,
      serialInterfaceNumber: exact.interfaceNumber,
    }));
  }, [
    activeBoardId,
    boardDraft.serialNumber,
    boardDraft.serialPid,
    boardDraft.serialVid,
    ports,
    selectedPort,
  ]);

  useEffect(() => {
    if (isActive && activeBoardId) void refreshBackendRef.current();
  }, [activeBoardId, isActive]);

  useEffect(() => {
    let unsubscribe: (() => void) | undefined;
    let cancelled = false;
    void subscribeEvent('serial-data-received', (payload) => {
      const owner = [...openedSerialsRef.current].find(([, item]) => item.port === payload.port);
      if (!owner) return;
      const boardId = owner[0];
      const decoder = serialDecodersRef.current.get(boardId) ?? new TextDecoder();
      serialDecodersRef.current.set(boardId, decoder);
      const text = decoder.decode(Uint8Array.from(payload.data), { stream: true });
      if (text) {
        appendBoardLog(boardId, 'info', text);
      }
    }).then((handler) => {
      if (cancelled) handler();
      else unsubscribe = handler;
    });
    return () => {
      cancelled = true;
      unsubscribe?.();
    };
  }, [appendBoardLog]);

  useEffect(() => {
    if (!isActive || !activeBoardId || !selectedPort || serialAutoConnectPaused) return;
    const matchingPort = ports.find((port) => {
      if (boardDraft.serialNumber) {
        return (
          port.serialNumber === boardDraft.serialNumber &&
          (!boardDraft.serialVid || port.vid === boardDraft.serialVid) &&
          (!boardDraft.serialPid || port.pid === boardDraft.serialPid) &&
          (boardDraft.serialInterfaceNumber == null ||
            port.interfaceNumber === boardDraft.serialInterfaceNumber)
        );
      }
      if (boardDraft.serialVid || boardDraft.serialPid) {
        return (
          (!boardDraft.serialVid || port.vid === boardDraft.serialVid) &&
          (!boardDraft.serialPid || port.pid === boardDraft.serialPid)
        );
      }
      return port.name === selectedPort;
    });
    if (!matchingPort) return;
    const existing = openedSerialsRef.current.get(activeBoardId);
    if (existing?.port === matchingPort.name && existing.baudRate === baudRate) return;
    void (async () => {
      if (existing) await closeSerial();
      await openSerialConnection();
    })();
  }, [
    activeBoardId,
    baudRate,
    boardDraft.serialInterfaceNumber,
    boardDraft.serialNumber,
    boardDraft.serialPid,
    boardDraft.serialVid,
    closeSerial,
    isActive,
    openSerialConnection,
    ports,
    selectedPort,
    serialAutoConnectPaused,
  ]);

  useEffect(() => {
    const terminal = terminalRef.current;
    if (terminal) terminal.scrollTop = terminal.scrollHeight;
  }, [logs]);

  const selectedSerial = useMemo(
    () => ports.find((port) => port.name === selectedPort),
    [ports, selectedPort]
  );

  const selectedProbe = useMemo(
    () =>
      probes.find(
        (probe) =>
          probe.serialNumber === selectedProbeKey || `index:${probe.index}` === selectedProbeKey
      ),
    [probes, selectedProbeKey]
  );

  const connectionFor = useCallback(
    (
      probe: McuStLinkProbe | undefined = selectedProbe,
      connectMode: McuStLinkConnection['connectMode'] = 'hotPlug'
    ): McuStLinkConnection => ({
      executablePath: cliPath || undefined,
      probeSerial: probe?.serialNumber || undefined,
      probeIndex: probe?.serialNumber ? undefined : probe?.index,
      frequencyKhz,
      connectMode,
    }),
    [cliPath, frequencyKhz, selectedProbe]
  );

  const openOcdConnection = useCallback(
    (): McuOpenOcdConnection => ({
      executablePath: cliPath || undefined,
      probeSerial: selectedProbeKey.startsWith('index:')
        ? undefined
        : selectedProbeKey || undefined,
      frequencyKhz,
      interfaceConfig: boardDraft.openocdInterfaceConfig,
      targetConfig: boardDraft.openocdTargetConfig,
      boardConfig: boardDraft.openocdBoardConfig,
    }),
    [
      boardDraft.openocdBoardConfig,
      boardDraft.openocdInterfaceConfig,
      boardDraft.openocdTargetConfig,
      cliPath,
      frequencyKhz,
      selectedProbeKey,
    ]
  );

  const pyOcdConnection = useCallback(
    (): McuPyOcdConnection => ({
      executablePath: cliPath || undefined,
      probeSerial: selectedProbeKey.startsWith('index:')
        ? undefined
        : selectedProbeKey || undefined,
      frequencyKhz,
      target: boardDraft.pyocdTarget,
      packPath: boardDraft.pyocdPackPath,
    }),
    [boardDraft.pyocdPackPath, boardDraft.pyocdTarget, cliPath, frequencyKhz, selectedProbeKey]
  );

  const jLinkConnection = useCallback(
    (): McuJLinkConnection => ({
      executablePath: cliPath || undefined,
      probeSerial: selectedProbeKey.startsWith('index:')
        ? undefined
        : selectedProbeKey || undefined,
      frequencyKhz,
      device: boardDraft.jlinkDevice,
    }),
    [boardDraft.jlinkDevice, cliPath, frequencyKhz, selectedProbeKey]
  );

  const identifyTarget = useCallback(async () => {
    setOperationBusy(true);
    setOperationLabel('识别目标');
    try {
      if (boardDraft.probeBackend === 'openOcd') {
        const status = await invokeCommand('mcu_openocd_target_status', {
          request: openOcdConnection(),
        });
        setTargetStatus(status);
        appendLog(
          'ok',
          `[TARGET] ${status.deviceName || boardDraft.mcuModel} · ${status.coreState}`
        );
        return;
      }
      if (boardDraft.probeBackend === 'pyOcd') {
        const status = await invokeCommand('mcu_pyocd_target_status', {
          request: pyOcdConnection(),
        });
        setTargetStatus(status);
        appendLog(
          'ok',
          `[TARGET] ${status.deviceName || boardDraft.mcuModel} · ${status.coreState}`
        );
        return;
      }
      if (boardDraft.probeBackend === 'jLink') {
        const status = await invokeCommand('mcu_jlink_target_status', {
          boardId: activeBoardId,
          request: jLinkConnection(),
        });
        setTargetStatus(status);
        appendLog(
          status.connected ? 'ok' : 'warn',
          `[TARGET] ${status.deviceName || boardDraft.mcuModel} · ${status.coreState}`
        );
        return;
      }
      const nextProbes = await invokeCommand('mcu_stlink_list_probes', {
        request: { executablePath: cliPath || undefined },
      });
      setProbes(nextProbes);
      const probe =
        nextProbes.find(
          (item) =>
            item.serialNumber === selectedProbeKey || `index:${item.index}` === selectedProbeKey
        ) ?? nextProbes[0];
      if (!probe) throw new Error('未检测到 ST-Link；请检查 USB 连接和驱动');
      setSelectedProbeKey(probe.serialNumber || `index:${probe.index}`);
      const status = await invokeCommand('mcu_stlink_target_status', {
        request: connectionFor(probe),
      });
      setTargetStatus(status);
      appendLog(
        'ok',
        `[TARGET] ${status.deviceName || 'STM32 target'} · ${status.coreState || 'UNKNOWN'} · ${status.voltage?.toFixed(2) ?? '--'} V`
      );
    } catch (error) {
      setTargetStatus(null);
      reportError('IDENTIFY', error);
    } finally {
      setOperationBusy(false);
      setOperationLabel('空闲');
    }
  }, [
    activeBoardId,
    appendLog,
    boardDraft,
    cliPath,
    connectionFor,
    jLinkConnection,
    openOcdConnection,
    pyOcdConnection,
    reportError,
    selectedProbeKey,
  ]);

  useEffect(() => {
    if (!isActive || !activeBoardId || !toolInfo?.available || operationBusy) return;
    const key = [
      activeBoardId,
      boardDraft.probeBackend,
      selectedProbeKey,
      frequencyKhz,
      cliPath,
    ].join('|');
    if (autoIdentifyKeyRef.current === key) return;
    autoIdentifyKeyRef.current = key;
    const timer = window.setTimeout(() => void identifyTarget(), 250);
    return () => window.clearTimeout(timer);
  }, [
    activeBoardId,
    boardDraft.probeBackend,
    cliPath,
    frequencyKhz,
    identifyTarget,
    isActive,
    operationBusy,
    selectedProbeKey,
    toolInfo?.available,
  ]);

  const runTargetAction = useCallback(
    async (action: 'resetRun' | 'resetHalt' | 'run' | 'halt', label: string) => {
      setOperationBusy(true);
      setOperationLabel(label);
      try {
        const result =
          boardDraft.probeBackend === 'openOcd'
            ? await invokeCommand('mcu_openocd_target_action', {
                boardId: activeBoardId,
                request: { connection: openOcdConnection(), action },
              })
            : boardDraft.probeBackend === 'pyOcd'
              ? await invokeCommand('mcu_pyocd_target_action', {
                  boardId: activeBoardId,
                  request: { connection: pyOcdConnection(), action },
                })
              : boardDraft.probeBackend === 'jLink'
                ? await invokeCommand('mcu_jlink_target_action', {
                    boardId: activeBoardId,
                    request: { connection: jLinkConnection(), action },
                  })
                : await invokeCommand('mcu_stlink_target_action', {
                    boardId: activeBoardId,
                    request: { connection: connectionFor(), action },
                  });
        appendLog('ok', `[TARGET] ${label}完成 · exit ${result.exitCode}`);
        const status =
          boardDraft.probeBackend === 'openOcd'
            ? await invokeCommand('mcu_openocd_target_status', { request: openOcdConnection() })
            : boardDraft.probeBackend === 'pyOcd'
              ? await invokeCommand('mcu_pyocd_target_status', { request: pyOcdConnection() })
              : boardDraft.probeBackend === 'jLink'
                ? await invokeCommand('mcu_jlink_target_status', {
                    boardId: activeBoardId,
                    request: jLinkConnection(),
                  })
                : await invokeCommand('mcu_stlink_target_status', { request: connectionFor() });
        setTargetStatus(status);
      } catch (error) {
        reportError('TARGET', error);
      } finally {
        setOperationBusy(false);
        setOperationLabel('空闲');
      }
    },
    [
      appendLog,
      activeBoardId,
      boardDraft.probeBackend,
      connectionFor,
      jLinkConnection,
      openOcdConnection,
      pyOcdConnection,
      reportError,
    ]
  );

  const chooseCli = useCallback(async () => {
    const selected = await open({
      multiple: false,
      directory: false,
      title:
        boardDraft.probeBackend === 'openOcd'
          ? '选择 OpenOCD'
          : boardDraft.probeBackend === 'pyOcd'
            ? '选择 pyOCD'
            : boardDraft.probeBackend === 'jLink'
              ? '选择 SEGGER J-Link Commander'
              : '选择 STM32CubeProgrammer CLI',
    });
    if (typeof selected === 'string') setCliPath(selected);
  }, [boardDraft.probeBackend]);

  const chooseExternalLoader = useCallback(async () => {
    const selected = await open({
      multiple: false,
      directory: false,
      title: '选择 OSPI External Loader',
      filters: [{ name: 'STM32 External Loader', extensions: ['stldr'] }],
    });
    if (typeof selected === 'string') setExternalLoaderPath(selected);
  }, []);

  const chooseInternalFirmware = useCallback(async () => {
    const selected = await open({
      multiple: false,
      directory: false,
      title: '选择 MCU 内部 Flash 固件',
      filters: [{ name: 'MCU Firmware', extensions: ['elf', 'hex', 'bin'] }],
    });
    if (typeof selected === 'string') {
      setBoardDraft((board) => ({ ...board, mcuFirmwarePath: selected }));
    }
  }, []);

  const flashPyOcdFirmware = useCallback(async () => {
    const address = binStart.trim() ? parseUnsigned(binStart) : 0;
    if (address == null) {
      appendLog('error', '[FLASH] BIN 烧写地址格式无效');
      return;
    }
    if (!boardDraft.mcuFirmwarePath) {
      appendLog('error', '[FLASH] 请先选择 .elf、.hex 或 .bin 固件');
      return;
    }
    setOperationBusy(true);
    setOperationLabel('烧写 RA Flash');
    try {
      const result = await invokeCommand('mcu_pyocd_flash_internal', {
        boardId: activeBoardId,
        request: {
          connection: pyOcdConnection(),
          firmwarePath: boardDraft.mcuFirmwarePath,
          binStartAddress: address,
          postAction: 'resetRun',
        },
      });
      appendLog('ok', `[FLASH] RA 内部 Flash 烧写完成 · exit ${result.exitCode}`);
    } catch (error) {
      reportError('FLASH', error);
    } finally {
      setOperationBusy(false);
      setOperationLabel('空闲');
    }
  }, [
    activeBoardId,
    appendLog,
    binStart,
    boardDraft.mcuFirmwarePath,
    pyOcdConnection,
    reportError,
  ]);

  const flashJLinkFirmware = useCallback(async () => {
    const parsedAddress = binStart.trim() ? parseUnsigned(binStart) : null;
    if (binStart.trim() && parsedAddress == null) {
      appendLog('error', '[FLASH] BIN 烧写地址格式无效');
      return;
    }
    const address = parsedAddress ?? undefined;
    if (!boardDraft.mcuFirmwarePath) {
      appendLog('error', '[FLASH] 请先选择 .elf、.hex 或 .bin 固件');
      return;
    }
    setOperationBusy(true);
    setOperationLabel('J-Link 烧写');
    try {
      const result = await invokeCommand('mcu_jlink_flash_internal', {
        boardId: activeBoardId,
        request: {
          connection: jLinkConnection(),
          firmwarePath: boardDraft.mcuFirmwarePath,
          binStartAddress: address,
          postAction: 'resetRun',
        },
      });
      appendLog('ok', `[FLASH] J-Link 内部 Flash 烧写完成 · exit ${result.exitCode}`);
    } catch (error) {
      reportError('FLASH', error);
    } finally {
      setOperationBusy(false);
      setOperationLabel('空闲');
    }
  }, [
    activeBoardId,
    appendLog,
    binStart,
    boardDraft.mcuFirmwarePath,
    jLinkConnection,
    reportError,
  ]);

  const probeOnline = Boolean(targetStatus?.connected);

  const openNewBoard = useCallback(() => {
    const next = defaultBoard();
    applyBoard(next);
    setActiveBoardId('');
    setSettingsVisible(true);
  }, [applyBoard]);

  const selectBoard = useCallback(
    (board: McuBoardConfig) => {
      setActiveBoardId(board.id);
      localStorage.setItem(ACTIVE_BOARD_STORAGE_KEY, board.id);
      applyBoard(board);
    },
    [applyBoard]
  );

  const saveBoard = useCallback(
    async (closeAfterSave = true) => {
      const start = ospiStart.trim() ? parseUnsigned(ospiStart) : 0;
      const size = ospiSizeMiB.trim() ? parseUnsigned(ospiSizeMiB) : 0;
      if (start == null || size == null) {
        appendLog('error', '[BOARD] OSPI 地址或容量格式无效');
        return null;
      }
      setBoardSaving(true);
      setBoardSaveFeedback('');
      try {
        const saved = await invokeCommand('mcu_save_board', {
          board: {
            ...boardDraft,
            serialPort: selectedPort,
            baudRate,
            toolPath: cliPath,
            probeSerial: selectedProbeKey.startsWith('index:') ? '' : selectedProbeKey,
            swdFrequencyKhz: frequencyKhz,
            externalLoaderPath,
            ospiAddressStart: start,
            ospiSizeBytes: size * 1024 * 1024,
            mcpServerName: activeMcuMcpName,
          },
        });
        setBoards((current) => [...current.filter((board) => board.id !== saved.id), saved]);
        setActiveBoardId(saved.id);
        localStorage.setItem(ACTIVE_BOARD_STORAGE_KEY, saved.id);
        applyBoard(saved);
        if (closeAfterSave) setSettingsVisible(false);
        appendLog('ok', `[BOARD] 已保存 ${saved.name}`);
        setBoardSaveFeedback('单板与接口已保存');
        return saved;
      } catch (error) {
        reportError('BOARD SAVE', error);
        setBoardSaveFeedback(normalizeIpcError(error).message);
        return null;
      } finally {
        setBoardSaving(false);
      }
    },
    [
      appendLog,
      activeMcuMcpName,
      applyBoard,
      baudRate,
      boardDraft,
      cliPath,
      externalLoaderPath,
      frequencyKhz,
      ospiSizeMiB,
      ospiStart,
      reportError,
      selectedPort,
      selectedProbeKey,
    ]
  );

  const saveChannelAndConnect = useCallback(async () => {
    const saved = await saveBoard(false);
    if (saved) await startMcuMcp(saved, saved.id);
  }, [saveBoard, startMcuMcp]);

  const deleteBoard = useCallback(async () => {
    if (!boardDraft.id) return;
    const approved = await ask(`确认删除目标板“${boardDraft.name}”？`, {
      title: '删除 MCU 目标板',
      kind: 'warning',
    });
    if (!approved) return;
    try {
      await invokeCommand('mcu_delete_board', { id: boardDraft.id });
      const remaining = boards.filter((board) => board.id !== boardDraft.id);
      setBoards(remaining);
      setSettingsVisible(false);
      if (remaining[0]) selectBoard(remaining[0]);
      else {
        setActiveBoardId('');
        localStorage.removeItem(ACTIVE_BOARD_STORAGE_KEY);
        applyBoard(defaultBoard());
      }
    } catch (error) {
      reportError('BOARD DELETE', error);
    }
  }, [applyBoard, boardDraft, boards, reportError, selectBoard]);

  return (
    <div className="mcu-workbench">
      <aside className="mcu-board-rail board-list">
        <div className="device-board-rail-heading">
          <span>
            <h3>MCU 单板</h3>
            <b>{boards.length}</b>
          </span>
          <button data-testid="mcu-new-board" type="button" onClick={openNewBoard}>
            <LynxIcon name="add" size={16} /> 新建
          </button>
        </div>
        <label className="device-board-search">
          <LynxIcon name="search" size={16} />
          <input
            value={boardSearch}
            placeholder="搜索单板"
            onChange={(event) => setBoardSearch(event.target.value)}
          />
        </label>
        <div className="board-items">
          {boards
            .filter((board) =>
              `${board.name} ${board.mcuModel}`
                .toLowerCase()
                .includes(boardSearch.trim().toLowerCase())
            )
            .map((board) => {
              const specs = mcuBoardSpecs(board);
              const online =
                mcuBoardOnline(board, ports) || (board.id === activeBoardId && probeOnline);
              return (
                <div
                  className={`board-item is-${board.chipVendor}${board.id === activeBoardId ? ' is-active' : ''}`}
                  key={board.id}
                >
                  <button
                    className="board-item-select device-board-card-select"
                    onClick={() => selectBoard(board)}
                  >
                    <BoardThumbnail
                      imagePath={board.boardImagePath}
                      marker={board.chipVendor === 'st' ? 'ST' : 'RA'}
                      tone={board.chipVendor}
                    />
                    <span className="device-board-copy">
                      <span className="device-board-name-row">
                        <strong>{board.name}</strong>
                        <span className={`board-live-row ${online ? 'is-online' : ''}`}>
                          <i /> {online ? '在线' : '离线'}
                        </span>
                      </span>
                      <small>
                        {board.chipVendor === 'st' ? 'STMicroelectronics · ST' : 'Renesas · 瑞萨'}
                      </small>
                      <span className="device-board-specs">
                        <b>{specs[0]}</b>
                        <em>{specs[1]}</em>
                      </span>
                    </span>
                  </button>
                </div>
              );
            })}
          {!boards.length && <div className="board-empty">点击“新建”添加第一块单板</div>}
        </div>
      </aside>

      <main className="mcu-main">
        {!activeBoardId ? (
          <section className="mcu-empty-state" aria-label="尚未添加 MCU 单板">
            <small>MCU WORKBENCH</small>
            <strong>尚未添加单片机设备</strong>
            <p>点击“新建”，选择已验证的芯片厂家和系列，再绑定调试器与串口。</p>
            <button type="button" onClick={openNewBoard}>
              <LynxIcon name="add" size={16} /> 新建
            </button>
          </section>
        ) : (
          <>
            <header className="device-workbench-header mcu-device-header">
              <div className="device-workbench-identity">
                <span className="device-header-icon">
                  <LynxIcon name="board" size={20} />
                </span>
                <div>
                  <span className="device-workbench-kicker">MCU TARGET BOARD</span>
                  <h2>{boardDraft.name}</h2>
                  <p>
                    {boardDraft.mcuModel} · {boardDraft.probeBackend} ·{' '}
                    {selectedPort || '未绑定串口'}
                  </p>
                </div>
              </div>
              <span className={selectedSerial || probeOnline ? 'is-online' : ''}>
                <i /> {selectedSerial || probeOnline ? '设备在线' : '等待设备'}
              </span>
              <button
                data-testid="mcu-open-settings"
                type="button"
                onClick={() => setSettingsVisible(true)}
              >
                单板设置
              </button>
            </header>

            <nav className="device-workbench-tabs" aria-label="MCU 单板功能">
              {(
                [
                  ['overview', '设备概览', 'board'],
                  ['serial', '串口终端', 'terminal'],
                  ['probe', '调试与探针', 'bug'],
                  ['flash', '固件烧录', 'download'],
                  ['ai', 'AI 通道', 'agent'],
                ] as ReadonlyArray<[McuDeviceView, string, LynxIconName]>
              ).map(([id, label, icon]) => (
                <button
                  key={id}
                  data-testid={`mcu-tab-${id}`}
                  className={activeView === id ? 'is-active' : ''}
                  onClick={() => setActiveView(id)}
                >
                  <span className="device-tab-icon">
                    <LynxIcon name={icon} size={16} filled={activeView === id} />
                    <i
                      className={
                        (id === 'ai' && mcpTunnel?.state === 'connected') ||
                        (id === 'serial' && selectedSerial) ||
                        (id === 'probe' && probeOnline) ||
                        (id === 'flash' && probeOnline)
                          ? 'is-online'
                          : ''
                      }
                    />
                  </span>
                  <span>{label}</span>
                </button>
              ))}
            </nav>

            {activeView === 'overview' && (
              <section className="mcu-debug-overview">
                <section className="engineering-panel mcu-debug-target">
                  <header>
                    <div>
                      <small>TARGET STATE</small>
                      <h3>目标状态</h3>
                    </div>
                    <span className={probeOnline ? 'is-online' : ''}>
                      {operationBusy ? operationLabel : probeOnline ? '已连接' : '未连接'}
                    </span>
                  </header>
                  <dl>
                    <div>
                      <dt>MCU 型号</dt>
                      <dd>{targetStatus?.deviceName || boardDraft.mcuModel}</dd>
                    </div>
                    <div>
                      <dt>内核状态</dt>
                      <dd>{targetStatus?.coreState || '未采集'}</dd>
                    </div>
                    <div>
                      <dt>目标电压</dt>
                      <dd>
                        {targetStatus?.voltage == null
                          ? '未采集'
                          : `${targetStatus.voltage.toFixed(2)} V`}
                      </dd>
                    </div>
                    <div>
                      <dt>设备 ID</dt>
                      <dd>{targetStatus?.deviceId || '未采集'}</dd>
                    </div>
                    <div>
                      <dt>调试后端</dt>
                      <dd>{boardDraft.probeBackend}</dd>
                    </div>
                    <div>
                      <dt>当前操作</dt>
                      <dd>{operationBusy ? operationLabel : '空闲'}</dd>
                    </div>
                  </dl>
                </section>

                <section className="engineering-panel mcu-debug-source">
                  <header>
                    <div>
                      <small>DEBUG CONTEXT</small>
                      <h3>源码与反汇编</h3>
                    </div>
                    <button type="button" onClick={() => setActiveView('probe')}>
                      进入调试
                    </button>
                  </header>
                  <div className="mcu-debug-source-tabs">
                    <span className="is-active">源代码</span>
                    <span>反汇编</span>
                  </div>
                  <div className="mcu-debug-source-empty">
                    <LynxIcon name="bug" size={16} />
                    <strong>尚未建立调试上下文</strong>
                    <span>连接探针并识别目标后，在调试页加载真实源码和目标状态。</span>
                  </div>
                </section>

                <section className="engineering-panel mcu-debug-memory">
                  <header>
                    <div>
                      <small>MEMORY MAP</small>
                      <h3>存储器映射</h3>
                    </div>
                  </header>
                  <div className="mcu-memory-table">
                    <span className="is-heading">
                      <b>区域</b>
                      <b>容量</b>
                      <b>来源</b>
                    </span>
                    <span>
                      <strong>Internal Flash</strong>
                      <b>识别后显示</b>
                      <small>目标探针</small>
                    </span>
                    <span>
                      <strong>SRAM</strong>
                      <b>识别后显示</b>
                      <small>目标探针</small>
                    </span>
                    <span>
                      <strong>OSPI Flash</strong>
                      <b>
                        {externalFlashCapacity(
                          boardDraft.externalFlashModel,
                          boardDraft.ospiSizeBytes
                        )}
                      </b>
                      <small>{boardDraft.externalFlashModel || '未配置'}</small>
                    </span>
                    <span>
                      <strong>External SRAM</strong>
                      <b>{externalSramCapacity(boardDraft.externalSramModel)}</b>
                      <small>{boardDraft.externalSramModel || '未配置'}</small>
                    </span>
                  </div>
                </section>

                <section className="engineering-panel mcu-debug-registers">
                  <header>
                    <div>
                      <small>CORE REGISTERS</small>
                      <h3>寄存器</h3>
                    </div>
                    <span>连接后读取</span>
                  </header>
                  <div>
                    {['R0', 'R1', 'R2', 'R3', 'R4', 'R5', 'R6', 'R7', 'SP', 'LR', 'PC', 'xPSR'].map(
                      (register) => (
                        <span key={register}>
                          <b>{register}</b>
                          <code>—</code>
                        </span>
                      )
                    )}
                  </div>
                </section>

                <section className="engineering-panel mcu-debug-breakpoints">
                  <header>
                    <div>
                      <small>BREAKPOINTS</small>
                      <h3>断点列表</h3>
                    </div>
                  </header>
                  <div className="mcu-debug-empty-row">当前没有断点；进入调试页后添加。</div>
                </section>

                <section className="engineering-panel mcu-debug-probe">
                  <header>
                    <div>
                      <small>PROBE &amp; CONNECTION</small>
                      <h3>探针与连接</h3>
                    </div>
                    <i className={probeOnline ? 'is-online' : ''} />
                  </header>
                  <dl>
                    <div>
                      <dt>探针</dt>
                      <dd>{selectedProbeKey || '未绑定'}</dd>
                    </div>
                    <div>
                      <dt>接口</dt>
                      <dd>SWD · {frequencyKhz} kHz</dd>
                    </div>
                    <div>
                      <dt>串口</dt>
                      <dd>
                        {selectedPort || '未绑定'} · {baudRate}
                      </dd>
                    </div>
                    <div>
                      <dt>MCU MCP</dt>
                      <dd className={mcpTunnel?.state === 'connected' ? 'is-online' : ''}>
                        {mcpTunnel?.state === 'connected' ? '已连接' : '未连接'}
                      </dd>
                    </div>
                  </dl>
                </section>

                <section className="engineering-panel mcu-debug-recent">
                  <header>
                    <div>
                      <small>RECENT OPERATIONS</small>
                      <h3>最近操作</h3>
                    </div>
                  </header>
                  <div>
                    {logs
                      .slice(-5)
                      .reverse()
                      .map((line) => (
                        <span key={line.id}>
                          <i className={line.kind === 'ok' ? 'is-online' : ''} />
                          <small>{line.text}</small>
                        </span>
                      ))}
                    {!logs.length && <p>当前单板还没有操作记录。</p>}
                  </div>
                </section>

                <section className="engineering-panel mcu-debug-console">
                  <header>
                    <div>
                      <small>DEBUG CONSOLE</small>
                      <h3>调试控制台</h3>
                    </div>
                    <span>{boardDraft.probeBackend}</span>
                  </header>
                  <div>
                    {logs.slice(-10).map((line) => (
                      <code key={line.id} className={`is-${line.kind}`}>
                        {line.text}
                      </code>
                    ))}
                    {!logs.length && <code className="is-dim">等待真实探针或串口日志…</code>}
                  </div>
                </section>
              </section>
            )}

            {(activeView === 'serial' || activeView === 'probe') && (
              <section className={`mcu-console-grid is-${activeView}`}>
                <article className="mcu-panel mcu-serial-panel">
                  <header className="mcu-panel-header">
                    <div className="mcu-panel-title">
                      <i className={selectedSerial ? 'is-online' : ''} />
                      <div>
                        <small>SERIAL TERMINAL</small>
                        <strong>启动日志与测试串口</strong>
                      </div>
                    </div>
                    <span>
                      {operationBusy
                        ? operationLabel.toUpperCase()
                        : serialConnected
                          ? 'CONNECTED'
                          : selectedSerial
                            ? 'READY'
                            : 'NO PORT'}
                    </span>
                  </header>
                  <div className="mcu-serial-controls">
                    <label>
                      <span>串口</span>
                      <select
                        value={selectedPort}
                        disabled={serialConnected}
                        onChange={(event) => bindSerialPort(event.target.value)}
                      >
                        <option value="">不绑定</option>
                        {selectedPort && !ports.some((port) => port.name === selectedPort) && (
                          <option value={selectedPort}>{selectedPort} · 等待设备接入</option>
                        )}
                        {ports.map((port) => (
                          <option key={port.name} value={port.name}>
                            {port.name}
                          </option>
                        ))}
                      </select>
                    </label>
                    <label>
                      <span>波特率</span>
                      <select
                        value={baudRate}
                        disabled={serialConnected}
                        onChange={(event) => setBaudRate(Number(event.target.value))}
                      >
                        {BAUD_RATES.map((rate) => (
                          <option key={rate} value={rate}>
                            {rate}
                          </option>
                        ))}
                      </select>
                    </label>
                    <button
                      type="button"
                      onClick={() => void refreshSerialPorts()}
                      disabled={serialLoading}
                    >
                      {serialLoading ? '检测中' : '刷新'}
                    </button>
                    <button
                      type="button"
                      disabled={!selectedPort || serialLoading}
                      onClick={() => void toggleSerial()}
                    >
                      {serialConnected ? '暂停自动连接' : '立即重连'}
                    </button>
                  </div>
                  <div ref={terminalRef} className="mcu-terminal" aria-label="MCU 后端操作日志">
                    {logs.map((line) => (
                      <div key={line.id} className={`is-${line.kind}`}>
                        {line.text}
                      </div>
                    ))}
                  </div>
                  <div className="mcu-serial-input">
                    <input
                      value={serialInput}
                      disabled={!selectedPort || serialLoading}
                      aria-label="串口发送内容"
                      placeholder={
                        serialConnected
                          ? '输入命令，Enter 发送 CRLF'
                          : selectedPort
                            ? '输入命令，发送时自动连接串口'
                            : '请先绑定调试串口'
                      }
                      onChange={(event) => setSerialInput(event.target.value)}
                      onKeyDown={(event) => {
                        if (event.key === 'Enter' && !event.nativeEvent.isComposing) {
                          event.preventDefault();
                          void sendSerialInput();
                        }
                      }}
                    />
                    <button
                      type="button"
                      disabled={!selectedPort || serialLoading || !serialInput}
                      onClick={() => void sendSerialInput()}
                    >
                      发送
                    </button>
                  </div>
                </article>

                <article className="mcu-panel mcu-probe-panel">
                  <header className="mcu-panel-header">
                    <div className="mcu-panel-title">
                      <i className={probeOnline ? 'is-online' : ''} />
                      <div>
                        <small>DEBUG PROBE</small>
                        <strong>调试探针</strong>
                      </div>
                    </div>
                    <div className="mcu-probe-header-actions">
                      <span className={probeOnline ? '' : 'is-muted'}>
                        {probeOnline ? 'CONNECTED' : toolInfo ? 'READY' : 'TOOL MISSING'}
                      </span>
                      <button
                        type="button"
                        onClick={() => void identifyTarget()}
                        disabled={operationBusy}
                      >
                        识别
                      </button>
                    </div>
                  </header>
                  <div className="mcu-probe-body">
                    <section className="mcu-probe-identity">
                      <strong>
                        {boardDraft.probeBackend === 'pyOcd'
                          ? 'DAPLink'
                          : boardDraft.probeBackend === 'jLink'
                            ? 'SEGGER J-Link'
                            : 'ST-Link V2'}{' '}
                        · SWD {frequencyKhz / 1000} MHz
                      </strong>
                      {probes.length > 0 ? (
                        <select
                          value={selectedProbeKey}
                          onChange={(event) => {
                            setSelectedProbeKey(event.target.value);
                            setTargetStatus(null);
                          }}
                        >
                          {probes.map((probe) => {
                            const key = probe.serialNumber || `index:${probe.index}`;
                            return (
                              <option key={key} value={key}>
                                {probe.serialNumber || `Probe ${probe.index}`} ·{' '}
                                {probe.firmwareVersion || 'FW unknown'}
                              </option>
                            );
                          })}
                        </select>
                      ) : (
                        <span>
                          {boardDraft.probeBackend === 'openOcd'
                            ? toolInfo
                              ? 'OpenOCD 已就绪，点击识别目标'
                              : '请配置 OpenOCD'
                            : boardDraft.probeBackend === 'pyOcd'
                              ? toolInfo
                                ? 'pyOCD 已就绪，点击识别 RA 目标'
                                : '请配置 pyOCD / RA_DFP'
                              : boardDraft.probeBackend === 'jLink'
                                ? toolInfo
                                  ? 'J-Link Commander 已就绪，点击识别目标'
                                  : '请安装或选择 SEGGER J-Link Commander'
                                : toolInfo
                                  ? '未检测到探针'
                                  : '请配置 STM32CubeProgrammer CLI'}
                        </span>
                      )}
                    </section>
                    <div className="mcu-probe-metrics">
                      <div>
                        <span>目标芯片</span>
                        <strong>{targetStatus?.deviceName || boardDraft.mcuModel}</strong>
                      </div>
                      <div>
                        <span>设备 ID</span>
                        <strong>{targetStatus?.deviceId || '--'}</strong>
                      </div>
                      <div>
                        <span>目标电压</span>
                        <strong>{targetStatus?.voltage?.toFixed(2) ?? '--'} V</strong>
                      </div>
                      <div>
                        <span>运行状态</span>
                        <strong>{targetStatus?.coreState || 'UNKNOWN'}</strong>
                      </div>
                    </div>
                    <section className="mcu-chip-resources">
                      <header>
                        <small>CHIP &amp; BOARD RESOURCES</small>
                        <strong>芯片与板级资源</strong>
                      </header>
                      <div>
                        <span>处理器内核</span>
                        <strong>
                          {boardDraft.mcuModel.startsWith('STM32H750')
                            ? 'Arm Cortex-M7 · 480 MHz'
                            : boardDraft.mcuModel}
                        </strong>
                      </div>
                      <div>
                        <span>片内存储</span>
                        <strong>
                          {boardDraft.mcuModel.startsWith('STM32H750')
                            ? 'Flash 128 KiB · RAM 1 MiB'
                            : '由芯片配置识别'}
                        </strong>
                      </div>
                      <div>
                        <span>外部 SPI NOR</span>
                        <strong>
                          {externalFlashCapacity(
                            boardDraft.externalFlashModel,
                            boardDraft.ospiSizeBytes
                          )}{' '}
                          · {boardDraft.externalFlashModel || '未配置'}
                        </strong>
                      </div>
                      <div>
                        <span>外部 SRAM</span>
                        <strong>
                          {externalSramCapacity(boardDraft.externalSramModel)} ·{' '}
                          {boardDraft.externalSramModel || '未配置'}
                        </strong>
                      </div>
                      <div>
                        <span>调试串口</span>
                        <strong>
                          {boardDraft.uartInstance || 'UART'} · {boardDraft.uartTxPin || '--'}/
                          {boardDraft.uartRxPin || '--'} · {baudRate}
                        </strong>
                      </div>
                      <div>
                        <span>本机接口</span>
                        <strong>
                          {selectedPort || '未绑定'} · SWD {frequencyKhz} kHz
                        </strong>
                      </div>
                    </section>
                    <div className="mcu-probe-actions">
                      <button
                        type="button"
                        disabled={operationBusy}
                        onClick={() => void runTargetAction('resetRun', '复位运行')}
                      >
                        复位运行
                      </button>
                      <button
                        type="button"
                        disabled={operationBusy}
                        onClick={() => void runTargetAction('resetHalt', '复位暂停')}
                      >
                        复位暂停
                      </button>
                      <button
                        type="button"
                        disabled={operationBusy}
                        onClick={() => void runTargetAction('run', '运行')}
                      >
                        运行
                      </button>
                      <button
                        type="button"
                        disabled={operationBusy}
                        onClick={() => void runTargetAction('halt', '暂停')}
                      >
                        暂停
                      </button>
                    </div>
                  </div>
                </article>
              </section>
            )}

            {activeView === 'flash' && (
              <section className="mcu-panel mcu-firmware-page">
                <header className="mcu-panel-header">
                  <div className="mcu-panel-title">
                    <i className={boardDraft.mcuFirmwarePath ? 'is-online' : ''} />
                    <div>
                      <small>FIRMWARE FLASH</small>
                      <strong>固件烧录</strong>
                    </div>
                  </div>
                  <span>{operationBusy ? operationLabel.toUpperCase() : 'READY'}</span>
                </header>
                <div className="mcu-firmware-content">
                  <section>
                    <h3>烧录目标</h3>
                    <dl>
                      <div>
                        <dt>目标芯片</dt>
                        <dd>{boardDraft.mcuModel}</dd>
                      </div>
                      <div>
                        <dt>烧录后端</dt>
                        <dd>{boardDraft.probeBackend}</dd>
                      </div>
                      <div>
                        <dt>探针</dt>
                        <dd>{selectedProbeKey || '等待识别'}</dd>
                      </div>
                      <div>
                        <dt>BIN 起始地址</dt>
                        <dd>{binStart}</dd>
                      </div>
                    </dl>
                  </section>
                  <section>
                    <h3>固件文件</h3>
                    <div className="mcu-firmware-picker">
                      <input
                        readOnly
                        value={boardDraft.mcuFirmwarePath}
                        placeholder="选择 .elf / .hex / .bin 固件"
                      />
                      <button type="button" onClick={() => void chooseInternalFirmware()}>
                        选择固件
                      </button>
                    </div>
                    <p>烧录、校验和复位继续使用当前单板保存的探针与固定作用域。</p>
                    <button
                      className="mcu-firmware-run"
                      type="button"
                      disabled={
                        operationBusy ||
                        !['pyOcd', 'jLink'].includes(boardDraft.probeBackend) ||
                        !boardDraft.mcuFirmwarePath
                      }
                      onClick={() =>
                        void (boardDraft.probeBackend === 'jLink'
                          ? flashJLinkFirmware()
                          : flashPyOcdFirmware())
                      }
                    >
                      烧写、校验并运行
                    </button>
                    {!['pyOcd', 'jLink'].includes(boardDraft.probeBackend) && (
                      <button type="button" onClick={() => setSettingsVisible(true)}>
                        配置当前烧录后端
                      </button>
                    )}
                  </section>
                  <section className="mcu-firmware-log">
                    <h3>操作日志</h3>
                    <pre>
                      {logs.length ? logs.map((line) => line.text).join('\n') : '等待烧录操作…'}
                    </pre>
                  </section>
                </div>
              </section>
            )}

            {activeView === 'ai' && (
              <DeviceAiChannel
                kind="mcu"
                boardId={activeBoardId}
                board={boardDraft}
                tunnel={mcpTunnel}
                onOpenSettings={() => setSettingsVisible(true)}
              />
            )}
          </>
        )}
      </main>

      {settingsVisible && (
        <div
          data-testid="mcu-settings-backdrop"
          className="workbench-settings-backdrop mcu-settings-backdrop"
          role="presentation"
          onMouseDown={(event) => {
            if (event.target === event.currentTarget) setSettingsVisible(false);
          }}
        >
          <section
            data-testid="mcu-settings-dialog"
            className="workbench-settings-dialog mcu-settings-dialog"
            role="dialog"
            aria-modal="true"
            aria-label="MCU 目标板设置"
          >
            <header className="workbench-settings-header">
              <div>
                <small>BOARD &amp; MCU MCP AGENT SETTINGS</small>
                <strong>{boardDraft.mcuModel || '新建 MCU 单板'}</strong>
                <span>配置保存一次后，LYNX 会自动恢复探针、串口、Gateway 与 MCU SDK 隧道。</span>
              </div>
              <button
                data-testid="mcu-settings-close"
                type="button"
                onClick={() => setSettingsVisible(false)}
              >
                关闭
              </button>
            </header>
            <div className="workbench-settings-body">
              <section className="workbench-card board-editor mcu-hardware-settings">
                <div className="board-section-title">
                  <div>
                    <small>HARDWARE CHANNELS</small>
                    <strong>{boardDraft.name || boardDraft.mcuModel}</strong>
                  </div>
                  <button
                    type="button"
                    onClick={() => void refreshHardwareInterfaces()}
                    disabled={serialLoading || operationBusy}
                  >
                    {serialLoading || operationBusy ? '扫描中…' : '重新扫描接口'}
                  </button>
                </div>
                <div className="board-live-status">
                  <div className={probeOnline ? 'is-online' : ''}>
                    <span>调试探针</span>
                    <strong>{probeOnline ? '在线' : '离线'}</strong>
                    <small>
                      {boardDraft.probeBackend === 'pyOcd'
                        ? 'DAPLink'
                        : boardDraft.probeBackend === 'jLink'
                          ? 'J-Link'
                          : 'ST-Link'}{' '}
                      · SWD
                    </small>
                  </div>
                  <div className={selectedPort ? 'is-online' : ''}>
                    <span>调试串口</span>
                    <strong>{selectedPort ? '已绑定' : '离线'}</strong>
                    <small>{selectedPort || '未选择串口'}</small>
                  </div>
                  <div>
                    <span>固件目标</span>
                    <strong>{boardDraft.externalFlashModel || boardDraft.mcuModel}</strong>
                    <small>
                      {boardDraft.externalFlashModel ? '已配置外部 Flash' : '片内 Flash'}
                    </small>
                  </div>
                </div>
                <div className="board-form-grid mcu-hardware-grid">
                  <label>
                    <span>产品名称（最多 20 个字符）</span>
                    <input
                      data-testid="mcu-board-name"
                      value={boardDraft.name}
                      maxLength={20}
                      onChange={(event) =>
                        setBoardDraft((board) => ({
                          ...board,
                          name: event.target.value.slice(0, 20),
                        }))
                      }
                    />
                  </label>
                  <div className="board-image-field is-wide">
                    <span>主板图片</span>
                    <div>
                      <button
                        type="button"
                        onClick={async () => {
                          const selected = await open({
                            multiple: false,
                            directory: false,
                            filters: [
                              {
                                name: '主板图片',
                                extensions: ['png', 'jpg', 'jpeg', 'webp'],
                              },
                            ],
                          });
                          if (typeof selected === 'string') {
                            setBoardDraft((board) => ({
                              ...board,
                              boardImagePath: selected,
                            }));
                          }
                        }}
                      >
                        选择图片
                      </button>
                      <span>
                        {boardDraft.boardImagePath.split(/[/\\]/).pop() ||
                          '未设置，将显示 PCB 示意占位'}
                      </span>
                      {boardDraft.boardImagePath && (
                        <button
                          type="button"
                          onClick={() =>
                            setBoardDraft((board) => ({ ...board, boardImagePath: '' }))
                          }
                        >
                          移除
                        </button>
                      )}
                    </div>
                  </div>
                  <label>
                    <span>芯片厂家</span>
                    <select
                      value={boardDraft.chipVendor}
                      onChange={(event) => {
                        const vendor = event.target.value as McuChipVendor;
                        const series = CHIP_SERIES[vendor][0].value;
                        setBoardDraft((board) => applyChipSeries(board, vendor, series));
                      }}
                    >
                      {CHIP_VENDORS.map((vendor) => (
                        <option key={vendor.value} value={vendor.value}>
                          {vendor.label}
                        </option>
                      ))}
                    </select>
                  </label>
                  <label>
                    <span>芯片型号</span>
                    <select
                      value={boardDraft.chipSeries}
                      onChange={(event) => {
                        const series = event.target.value as McuChipSeries;
                        setBoardDraft((board) => applyChipSeries(board, board.chipVendor, series));
                      }}
                    >
                      {CHIP_SERIES[boardDraft.chipVendor].map((series) => (
                        <option key={series.value} value={series.value}>
                          {series.label}
                        </option>
                      ))}
                    </select>
                  </label>
                  <label>
                    <span>调试串口</span>
                    <select
                      value={selectedPort}
                      onChange={(event) => bindSerialPort(event.target.value)}
                    >
                      <option value="">不绑定</option>
                      {selectedPort && !ports.some((port) => port.name === selectedPort) && (
                        <option value={selectedPort}>{selectedPort} · 等待设备接入</option>
                      )}
                      {ports.map((port) => (
                        <option key={port.name} value={port.name}>
                          {port.name} {port.description ? `· ${port.description}` : ''}
                        </option>
                      ))}
                    </select>
                  </label>
                  <label>
                    <span>波特率</span>
                    <select
                      value={baudRate}
                      onChange={(event) => setBaudRate(Number(event.target.value))}
                    >
                      {BAUD_RATES.map((rate) => (
                        <option key={rate} value={rate}>
                          {rate}
                        </option>
                      ))}
                    </select>
                  </label>
                  <label>
                    <span>调试 / 烧录后端</span>
                    <select
                      value={boardDraft.probeBackend}
                      onChange={(event) =>
                        setBoardDraft((board) => ({
                          ...board,
                          probeBackend: event.target.value as McuProbeBackend,
                        }))
                      }
                    >
                      <option value="openOcd">OpenOCD + ST-Link</option>
                      <option value="pyOcd">pyOCD + CMSIS-DAP / DAPLink</option>
                      <option value="jLink">SEGGER J-Link</option>
                      <option value="cubeProgrammer">STM32CubeProgrammer + ST-Link</option>
                    </select>
                  </label>
                  <label>
                    <span>
                      {boardDraft.probeBackend === 'pyOcd'
                        ? 'CMSIS-DAP / DAPLink 调试器'
                        : boardDraft.probeBackend === 'jLink'
                          ? 'SEGGER J-Link 调试器 / 烧录器'
                          : 'ST-Link 调试器 / 烧录器'}
                    </span>
                    <select
                      value={selectedProbeKey}
                      onChange={(event) => setSelectedProbeKey(event.target.value)}
                    >
                      <option value="">自动选择唯一探针</option>
                      {probes.map((probe) => {
                        const key = probe.serialNumber || `index:${probe.index}`;
                        return (
                          <option key={key} value={key}>
                            {probe.serialNumber || `ST-Link #${probe.index}`} ·{' '}
                            {probe.firmwareVersion || '固件未知'}
                          </option>
                        );
                      })}
                    </select>
                  </label>
                </div>
                <div className="mcu-power-card">
                  <div className="mcu-power-heading">
                    <div>
                      <small>LYNX POWER</small>
                      <strong>Lynx Power 电源控制</strong>
                    </div>
                    <span className={boardDraft.powerControllerType ? 'is-online' : ''}>
                      {boardDraft.powerControllerType
                        ? powerDevices.length
                          ? '设备已连接'
                          : '已配置'
                        : '未配置'}
                    </span>
                  </div>
                  <div className="mcu-power-grid">
                    <label>
                      <span>控制器</span>
                      <select
                        value={boardDraft.powerControllerType}
                        onChange={(event) =>
                          setBoardDraft((board) => ({
                            ...board,
                            powerControllerType: event.target.value as '' | 'ch347_gpio',
                            powerDeviceIndex: powerDevices[0]?.index ?? board.powerDeviceIndex,
                          }))
                        }
                      >
                        <option value="">不启用</option>
                        <option value="ch347_gpio">Lynx Power 控制器</option>
                      </select>
                    </label>
                    {boardDraft.powerControllerType === 'ch347_gpio' && (
                      <>
                        <label>
                          <span>Lynx Power 设备</span>
                          <select
                            value={boardDraft.powerDeviceIndex}
                            onChange={(event) =>
                              setBoardDraft((board) => ({
                                ...board,
                                powerDeviceIndex: Number(event.target.value),
                              }))
                            }
                          >
                            {!powerDevices.length && (
                              <option value={boardDraft.powerDeviceIndex}>
                                设备 #{boardDraft.powerDeviceIndex}
                              </option>
                            )}
                            {powerDevices.map((device) => (
                              <option key={device.index} value={device.index}>
                                {device.name} · {device.serialNumber || '无序列号'}
                              </option>
                            ))}
                          </select>
                        </label>
                        <label>
                          <span>电源控制通道</span>
                          <select
                            value={boardDraft.powerGpio}
                            onChange={(event) =>
                              setBoardDraft((board) => ({
                                ...board,
                                powerGpio: Number(event.target.value),
                              }))
                            }
                          >
                            {[4, 5, 6, 7].map((gpio) => (
                              <option key={gpio} value={gpio}>
                                GPIO{gpio}
                              </option>
                            ))}
                          </select>
                        </label>
                        <label>
                          <span>有效电平</span>
                          <select
                            value={boardDraft.powerActiveHigh ? 'high' : 'low'}
                            onChange={(event) =>
                              setBoardDraft((board) => ({
                                ...board,
                                powerActiveHigh: event.target.value === 'high',
                              }))
                            }
                          >
                            <option value="high">高电平上电</option>
                            <option value="low">低电平上电</option>
                          </select>
                        </label>
                      </>
                    )}
                  </div>
                  <div className="mcu-power-actions">
                    <button
                      type="button"
                      disabled={!boardDraft.powerControllerType || powerBusy}
                      onClick={() => void setMcuPower('on')}
                    >
                      上电
                    </button>
                    <button
                      type="button"
                      disabled={!boardDraft.powerControllerType || powerBusy}
                      onClick={() => void setMcuPower('off')}
                    >
                      断电
                    </button>
                    <button
                      type="button"
                      disabled={!boardDraft.powerControllerType || powerBusy}
                      onClick={() => void cycleMcuPower()}
                    >
                      断电重启
                    </button>
                    <button
                      type="button"
                      disabled={!boardDraft.powerControllerType || powerBusy}
                      onClick={() => setPowerResult(powerResult || '状态以最近一次控制结果为准')}
                    >
                      读取状态
                    </button>
                    <output>
                      {powerBusy ? '执行中…' : powerResult || '保存配置后可在这里测试。'}
                    </output>
                  </div>
                </div>
                <div className="mcu-hardware-summary">
                  <span>芯片：{boardDraft.mcuModel}</span>
                  <span>
                    探针：
                    {boardDraft.probeBackend === 'pyOcd'
                      ? 'DAPLink'
                      : boardDraft.probeBackend === 'jLink'
                        ? 'J-Link'
                        : 'ST-Link'}{' '}
                    / SWD
                  </span>
                  <span>串口：{selectedPort || '未绑定'}</span>
                  <span>外部存储：{boardDraft.externalFlashModel || '自动识别'}</span>
                </div>
                <div className="board-save-actions">
                  <button
                    data-testid="mcu-board-save"
                    className="board-primary"
                    type="button"
                    disabled={boardSaving || !boardDraft.name.trim()}
                    onClick={() => void saveBoard(false)}
                  >
                    {boardSaving ? '保存中…' : '保存单板与接口'}
                  </button>
                  {boardSaveFeedback && (
                    <output data-testid="mcu-board-save-feedback">{boardSaveFeedback}</output>
                  )}
                </div>
              </section>
              <section className="workbench-card sdk-editor">
                <div className="board-section-title">
                  <div>
                    <small>SDK CHANNELS</small>
                    <strong>远端 SDK 与 MCU MCP 隧道</strong>
                    <span>工程目录、编译命令、固件产物和板级参数由远端 MCU 开发代理自动识别。</span>
                  </div>
                </div>
                <div className="board-form-grid">
                  <label>
                    <span>SDK 通道名称</span>
                    <input value={mcuChannelName} readOnly />
                  </label>
                  <label>
                    <span>AI 开发工具</span>
                    <select
                      value={mcuAiProvider}
                      onChange={(event) =>
                        setMcuAiProvider(event.target.value as 'codex' | 'claude_code')
                      }
                    >
                      <option value="codex">Codex</option>
                      <option value="claude_code">Claude Code</option>
                    </select>
                  </label>
                  <label>
                    <span>主机名称</span>
                    <input value="MCU SDK 编译主机" readOnly />
                  </label>
                  <label>
                    <span>远端主机 IP / 域名</span>
                    <input
                      value={boardDraft.mcpHostAddress}
                      placeholder="例如：192.168.1.100"
                      onChange={(event) =>
                        setBoardDraft((board) => ({ ...board, mcpHostAddress: event.target.value }))
                      }
                    />
                  </label>
                  <label>
                    <span>SSH 用户</span>
                    <input
                      value={boardDraft.mcpHostUsername}
                      onChange={(event) =>
                        setBoardDraft((board) => ({
                          ...board,
                          mcpHostUsername: event.target.value,
                        }))
                      }
                    />
                  </label>
                  <label>
                    <span>SSH 端口</span>
                    <input
                      type="number"
                      value={boardDraft.mcpHostPort}
                      onChange={(event) =>
                        setBoardDraft((board) => ({
                          ...board,
                          mcpHostPort: Number(event.target.value),
                        }))
                      }
                    />
                  </label>
                  <label className="is-wide">
                    <span>SSH 密码</span>
                    <input
                      type="password"
                      autoComplete="new-password"
                      placeholder="不填则自动使用系统 SSH 密钥"
                      value={mcpPassword}
                      onChange={(event) => {
                        const password = event.target.value;
                        setMcpPassword(password);
                        if (password)
                          setBoardDraft((board) => ({ ...board, mcpAuthType: 'password' }));
                      }}
                    />
                  </label>
                  <label>
                    <span>远端 MCP 端口（自动避让）</span>
                    <input
                      type="number"
                      value={boardDraft.mcpRemotePort}
                      onChange={(event) =>
                        setBoardDraft((board) => ({
                          ...board,
                          mcpRemotePort: Number(event.target.value),
                        }))
                      }
                    />
                  </label>
                  <label>
                    <span>MCP 名称（根据通道名称自动生成）</span>
                    <input value={activeMcuMcpName} readOnly />
                  </label>
                </div>
                <div className="workbench-row">
                  <button
                    className="board-primary"
                    type="button"
                    disabled={mcpBusy || !boardDraft.mcpHostAddress.trim()}
                    onClick={() => void (mcpTunnel ? stopMcuMcp() : saveChannelAndConnect())}
                  >
                    {mcpBusy ? '正在保存并连接…' : mcpTunnel ? '停止通道' : '保存通道并自动连接'}
                  </button>
                </div>
                <output className="channel-save-feedback">
                  {boardDraft.id
                    ? (mcpTunnel?.message ?? '通道未启动')
                    : '保存单板后可启动专用通道'}
                </output>
              </section>
              <section className="workbench-card channel-map">
                <div className="board-section-title">
                  <div>
                    <small>CONNECTION MAP</small>
                    <h3>MCU SDK 通道</h3>
                  </div>
                </div>
                <div className="channel-topology">
                  <div className="channel-node is-local">
                    <small>
                      <i /> LOCAL NODE · 01
                    </small>
                    <strong>LYNX MCU 自动服务</strong>
                    <span>单板：{boardDraft.name || '尚未命名'}</span>
                    <span>探针、烧录与 UART 专用工具</span>
                  </div>
                  <b className="channel-link">
                    <span>ENCRYPTED MCP</span>
                    <i>
                      <em />
                      <em />
                      <em />
                    </i>
                    <span>LYNX MCU ROUTE</span>
                  </b>
                  <div className="channel-node is-remote">
                    <small>
                      <i /> REMOTE NODE · 02
                    </small>
                    <strong>{boardDraft.mcpHostAddress ? 'MCU 开发主机' : '尚未配置'}</strong>
                    <span>IP：{boardDraft.mcpHostAddress || '未填写'}</span>
                    <span>SDK：由远端 AI 当前工作目录决定</span>
                  </div>
                </div>
                <div className="channel-note">
                  填写并保存远端 MCU SDK 主机后，LYNX 会建立当前单板专属通道。
                </div>
                <div className="mcu-ai-connect">
                  <div className="channel-command">
                    <div>
                      <strong>在目标 SDK 目录启动专属 AI</strong>
                      <code>{mcuLaunchCommand}</code>
                      <span>
                        {mcuAiProvider === 'claude_code'
                          ? '配置只对本次 Claude Code 进程生效，并仅挂载当前 MCU 通道。'
                          : '配置只对本次 Codex 进程生效，并仅挂载当前 MCU 通道。'}
                      </span>
                    </div>
                    <button type="button" onClick={() => void copyMcuCommand(mcuLaunchCommand)}>
                      复制
                    </button>
                  </div>
                </div>
              </section>
              <div className="mcu-advanced-toggle">
                <div>
                  <strong>高级配置</strong>
                  <span>探针后端、烧写地址、外部存储和启动判定</span>
                </div>
                <button type="button" onClick={() => setAdvancedSettings((visible) => !visible)}>
                  {advancedSettings ? '收起' : '展开'}
                </button>
              </div>
              {advancedSettings && (
                <>
                  <label>
                    <span>烧录后端</span>
                    <select
                      value={boardDraft.probeBackend}
                      onChange={(event) =>
                        setBoardDraft((board) => ({
                          ...board,
                          probeBackend: event.target.value as McuProbeBackend,
                        }))
                      }
                    >
                      <option value="openOcd">OpenOCD（开源，默认）</option>
                      <option value="pyOcd">pyOCD + CMSIS-DAP（Renesas RA）</option>
                      <option value="jLink">SEGGER J-Link</option>
                      <option value="cubeProgrammer">STM32CubeProgrammer（兼容）</option>
                    </select>
                  </label>
                  <label>
                    <span>
                      {boardDraft.probeBackend === 'pyOcd'
                        ? 'DAP'
                        : boardDraft.probeBackend === 'jLink'
                          ? 'J-Link'
                          : 'ST-Link'}{' '}
                      序列号（可选）
                    </span>
                    <input
                      value={selectedProbeKey}
                      onChange={(event) => setSelectedProbeKey(event.target.value)}
                      placeholder="多探针时填写"
                    />
                  </label>
                  <label className="is-wide">
                    <span>
                      {boardDraft.probeBackend === 'openOcd'
                        ? 'OpenOCD 可执行文件'
                        : boardDraft.probeBackend === 'pyOcd'
                          ? 'pyOCD 可执行文件'
                          : boardDraft.probeBackend === 'jLink'
                            ? 'J-Link Commander 可执行文件'
                            : 'STM32CubeProgrammer CLI'}
                    </span>
                    <div>
                      <input
                        value={cliPath}
                        onChange={(event) => setCliPath(event.target.value)}
                        placeholder={
                          boardDraft.probeBackend === 'openOcd'
                            ? '自动检测或选择 openocd'
                            : boardDraft.probeBackend === 'pyOcd'
                              ? '安装 pyocd 后自动检测，或选择 pyocd'
                              : boardDraft.probeBackend === 'jLink'
                                ? '自动检测或选择 JLink.exe / JLinkExe'
                                : '自动检测或选择 STM32_Programmer_CLI'
                        }
                      />
                      <button type="button" onClick={() => void chooseCli()}>
                        选择
                      </button>
                    </div>
                  </label>
                  {boardDraft.probeBackend === 'openOcd' && (
                    <>
                      <label>
                        <span>接口配置</span>
                        <input
                          value={boardDraft.openocdInterfaceConfig}
                          onChange={(event) =>
                            setBoardDraft((board) => ({
                              ...board,
                              openocdInterfaceConfig: event.target.value,
                            }))
                          }
                        />
                      </label>
                      <label>
                        <span>目标配置</span>
                        <input
                          value={boardDraft.openocdTargetConfig}
                          onChange={(event) =>
                            setBoardDraft((board) => ({
                              ...board,
                              openocdTargetConfig: event.target.value,
                            }))
                          }
                        />
                      </label>
                      <label className="is-wide">
                        <span>板级 OSPI 配置 (.cfg)</span>
                        <div>
                          <input
                            value={boardDraft.openocdBoardConfig}
                            onChange={(event) =>
                              setBoardDraft((board) => ({
                                ...board,
                                openocdBoardConfig: event.target.value,
                              }))
                            }
                            placeholder="必须初始化 OCTOSPI 并声明 stmqspi flash bank"
                          />
                          <button
                            type="button"
                            onClick={async () => {
                              const selected = await open({
                                multiple: false,
                                directory: false,
                                title: '选择 OpenOCD 板级配置',
                                filters: [{ name: 'OpenOCD Config', extensions: ['cfg'] }],
                              });
                              if (typeof selected === 'string')
                                setBoardDraft((board) => ({
                                  ...board,
                                  openocdBoardConfig: selected,
                                }));
                            }}
                          >
                            选择
                          </button>
                        </div>
                      </label>
                      <label>
                        <span>Flash Bank 名称</span>
                        <input
                          value={boardDraft.openocdFlashBank}
                          onChange={(event) =>
                            setBoardDraft((board) => ({
                              ...board,
                              openocdFlashBank: event.target.value,
                            }))
                          }
                        />
                      </label>
                    </>
                  )}
                  {boardDraft.probeBackend === 'pyOcd' && (
                    <>
                      <label>
                        <span>pyOCD 目标型号</span>
                        <input
                          value={boardDraft.pyocdTarget}
                          onChange={(event) =>
                            setBoardDraft((board) => ({
                              ...board,
                              pyocdTarget: event.target.value,
                            }))
                          }
                          placeholder="R7FA6M5BF"
                        />
                      </label>
                      <label className="is-wide">
                        <span>Renesas RA_DFP (.pack)</span>
                        <div>
                          <input
                            value={boardDraft.pyocdPackPath}
                            readOnly
                            placeholder="默认使用内置 RA_DFP，也可手动选择"
                          />
                          <button
                            type="button"
                            onClick={async () => {
                              const selected = await open({
                                multiple: false,
                                directory: false,
                                title: '选择 Renesas RA Device Family Pack',
                                filters: [
                                  { name: 'CMSIS Device Family Pack', extensions: ['pack'] },
                                ],
                              });
                              if (typeof selected === 'string')
                                setBoardDraft((board) => ({
                                  ...board,
                                  pyocdPackPath: selected,
                                }));
                            }}
                          >
                            选择
                          </button>
                        </div>
                      </label>
                      <label className="is-wide">
                        <span>RA 内部 Flash 固件</span>
                        <div>
                          <input
                            value={boardDraft.mcuFirmwarePath}
                            readOnly
                            placeholder=".elf / .hex / .bin"
                          />
                          <button type="button" onClick={() => void chooseInternalFirmware()}>
                            选择
                          </button>
                          <button
                            type="button"
                            disabled={operationBusy || !boardDraft.mcuFirmwarePath}
                            onClick={() => void flashPyOcdFirmware()}
                          >
                            烧写并运行
                          </button>
                        </div>
                      </label>
                    </>
                  )}
                  {boardDraft.probeBackend === 'jLink' && (
                    <>
                      <label>
                        <span>J-Link Device</span>
                        <input
                          value={boardDraft.jlinkDevice}
                          onChange={(event) =>
                            setBoardDraft((board) => ({
                              ...board,
                              jlinkDevice: event.target.value,
                            }))
                          }
                          placeholder="例如 STM32H750ZBT6"
                        />
                      </label>
                      <label className="is-wide">
                        <span>J-Link 内部 Flash 固件</span>
                        <div>
                          <input
                            value={boardDraft.mcuFirmwarePath}
                            readOnly
                            placeholder=".elf / .hex / .bin"
                          />
                          <button type="button" onClick={() => void chooseInternalFirmware()}>
                            选择
                          </button>
                          <button
                            type="button"
                            disabled={operationBusy || !boardDraft.mcuFirmwarePath}
                            onClick={() => void flashJLinkFirmware()}
                          >
                            烧写并运行
                          </button>
                        </div>
                      </label>
                    </>
                  )}
                  {boardDraft.probeBackend === 'cubeProgrammer' && (
                    <label className="is-wide">
                      <span>OSPI External Loader (.stldr)</span>
                      <div>
                        <input
                          value={externalLoaderPath}
                          readOnly
                          placeholder="必须匹配实际 NOR 型号和板级引脚"
                        />
                        <button type="button" onClick={() => void chooseExternalLoader()}>
                          选择
                        </button>
                      </div>
                    </label>
                  )}
                  <label>
                    <span>OSPI 映射起始地址</span>
                    <input
                      value={ospiStart}
                      onChange={(event) => setOspiStart(event.target.value)}
                    />
                  </label>
                  <label>
                    <span>OSPI 容量 (MiB)</span>
                    <input
                      value={ospiSizeMiB}
                      onChange={(event) => setOspiSizeMiB(event.target.value)}
                      placeholder="例如 64"
                      inputMode="numeric"
                    />
                  </label>
                  <label>
                    <span>BIN 烧写起始地址</span>
                    <input value={binStart} onChange={(event) => setBinStart(event.target.value)} />
                  </label>
                  <label>
                    <span>SWD 频率 (kHz)</span>
                    <input
                      type="number"
                      min={5}
                      max={50000}
                      value={frequencyKhz}
                      onChange={(event) => setFrequencyKhz(Number(event.target.value))}
                    />
                  </label>
                  <label>
                    <span>外部 Flash 型号</span>
                    <input
                      value={boardDraft.externalFlashModel}
                      onChange={(event) =>
                        setBoardDraft((board) => ({
                          ...board,
                          externalFlashModel: event.target.value,
                        }))
                      }
                    />
                  </label>
                  <label>
                    <span>外部 SRAM 型号</span>
                    <input
                      value={boardDraft.externalSramModel}
                      onChange={(event) =>
                        setBoardDraft((board) => ({
                          ...board,
                          externalSramModel: event.target.value,
                        }))
                      }
                    />
                  </label>
                  <label>
                    <span>调试串口</span>
                    <input
                      value={boardDraft.uartInstance}
                      onChange={(event) =>
                        setBoardDraft((board) => ({ ...board, uartInstance: event.target.value }))
                      }
                    />
                  </label>
                  <label>
                    <span>UART TX 引脚</span>
                    <input
                      value={boardDraft.uartTxPin}
                      onChange={(event) =>
                        setBoardDraft((board) => ({ ...board, uartTxPin: event.target.value }))
                      }
                    />
                  </label>
                  <label>
                    <span>UART RX 引脚</span>
                    <input
                      value={boardDraft.uartRxPin}
                      onChange={(event) =>
                        setBoardDraft((board) => ({ ...board, uartRxPin: event.target.value }))
                      }
                    />
                  </label>
                  <label>
                    <span>启动成功文本</span>
                    <input
                      value={boardDraft.bootSuccessText}
                      onChange={(event) =>
                        setBoardDraft((board) => ({
                          ...board,
                          bootSuccessText: event.target.value,
                        }))
                      }
                      placeholder="例如 login:"
                    />
                  </label>
                  <label>
                    <span>启动失败文本</span>
                    <input
                      value={boardDraft.bootFailureText}
                      onChange={(event) =>
                        setBoardDraft((board) => ({
                          ...board,
                          bootFailureText: event.target.value,
                        }))
                      }
                      placeholder="例如 Kernel panic"
                    />
                  </label>
                  <label>
                    <span>启动检测超时 (秒)</span>
                    <input
                      type="number"
                      min={1}
                      max={600}
                      value={boardDraft.bootTimeoutMs / 1000}
                      onChange={(event) =>
                        setBoardDraft((board) => ({
                          ...board,
                          bootTimeoutMs: Number(event.target.value) * 1000,
                        }))
                      }
                    />
                  </label>
                </>
              )}
            </div>
            {advancedSettings && (
              <p>
                安全约束：pyOCD 使用 Renesas RA_DFP 的 Flash 算法；OpenOCD 必须选择真实板级 OSPI
                配置，CubeProgrammer 必须选择匹配 NOR 的 .stldr；J-Link 使用已保存的 Device
                与数字序列号固定绑定，BIN 必须填写起始地址；外部 Flash 数据必须完全位于声明范围。
              </p>
            )}
            <footer>
              {boardDraft.id && (
                <button type="button" onClick={() => void deleteBoard()} disabled={boardSaving}>
                  删除目标板
                </button>
              )}
              <button type="button" onClick={() => void refreshBackend()} disabled={operationBusy}>
                检测工具与探针
              </button>
              <button type="button" onClick={() => void saveBoard()} disabled={boardSaving}>
                {boardSaving ? '保存中' : '保存'}
              </button>
            </footer>
          </section>
        </div>
      )}
    </div>
  );
};
