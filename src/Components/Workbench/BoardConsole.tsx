import React, {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useRef,
  useState,
} from 'react';
import { FitAddon } from '@xterm/addon-fit';
import { Terminal } from '@xterm/xterm';
import { save } from '@tauri-apps/plugin-dialog';
import '@xterm/xterm/css/xterm.css';
import { invokeCommand, subscribeEvent } from '../../Platform/IPC';
import type {
  LinuxDeviceOverview,
  WorkbenchDevice,
  WorkbenchTaskHistory,
} from '../../Platform/IPC/Commands';
import { formatErrorMessage } from '../../Utils';
import { appendRecordEvent } from '../RecordCenter';
import LinuxTerminalWorkspace from './LinuxTerminalWorkspace';
import FirmwareFlashPanel from './FirmwareFlashPanel';
import { useTheme } from '../../Themes';
import {
  flashOutcomeLabel,
  deriveFlashPhase,
  FLASH_PHASE_LABELS,
  formatFlashPhase,
  formatFlashRate,
  formatPostFlashState,
  formatUsbSpeed,
  formatVerifyState,
  restoreFlashOutputTail,
} from './flashTelemetryPresentation';
import './BoardConsole.css';

type BoardRuntime = {
  serialConnected: boolean;
  serialOpening: boolean;
  serialPort: string;
  serialData: number[];
  serialSessionId: string;
  serialCapturedBytes: number;
  serialSynchronizedBytes: number;
  serialCaptureError: string;
  serialConnectionError: string;
  serialEmitErrors: number;
  flashLines: string[];
  flashProgress: number;
  flashState: string;
  flashPhase: string;
  flashPartition: string;
  flashPartitionProgress: number | null;
  flashRate: number | null;
  flashUsbSpeed: string;
  flashUsbTheoreticalMbps: number | null;
  flashCommittedBytes: number | null;
  flashTotalBytes: number | null;
  flashStorageType: string;
  flashPhysicalVerify: string;
  flashBootVerify: string;
  flashVerify: string;
  flashPostActionKind: string;
  flashPostAction: string;
  flashTaskStatus: string;
};

function sameBoardRuntime(left: BoardRuntime, right: BoardRuntime) {
  const keys = Object.keys(left) as Array<keyof BoardRuntime>;
  return keys.every((key) => left[key] === right[key]);
}

interface BoardConsoleProps {
  applicationCandidateId?: string;
  view: 'serial' | 'flash' | 'workspace';
  devices: WorkbenchDevice[];
  selectedDevice?: WorkbenchDevice;
  isActive: boolean;
  taskHistory: WorkbenchTaskHistory[];
  serialDetected: boolean;
  adbDetected: boolean;
  flashDetected: boolean;
  flashBound: boolean;
  flashProtocol: string;
  savedSerialPort: string;
  aiChannel: React.ReactNode;
  onOpenSettings(): void;
  workspace?: {
    projectName: string;
    hostLabel: string;
    primaryTerminalOnline: boolean;
    overview?: LinuxDeviceOverview;
    primaryTerminal: React.ReactNode;
    onRequestExit(): void;
  };
}

type TerminalHandle = {
  write(data: Uint8Array): void;
  replace(data: Uint8Array): void;
  clear(): void;
};

const SERIAL_WRITE_BATCH_MS = 8;
const SERIAL_WRITE_CHUNK_BYTES = 4 * 1024;
const TERMINAL_HISTORY_BYTES = 256 * 1024;

interface TerminalSurfaceProps {
  initialData: number[];
  connected: boolean;
  placeholder: string;
  variant: 'serial';
  appearance?: 'dark' | 'light';
  onData(data: string): void;
}

const TerminalSurface = forwardRef<TerminalHandle, TerminalSurfaceProps>(
  ({ initialData, connected, placeholder, variant, appearance = 'dark', onData }, ref) => {
    const hostRef = useRef<HTMLDivElement>(null);
    const terminalRef = useRef<Terminal | null>(null);
    const terminalWriteQueueRef = useRef<Uint8Array[]>([]);
    const terminalWriteInFlightRef = useRef(false);
    const terminalReplacePendingRef = useRef<Uint8Array | null>(null);
    const drainTerminalWritesRef = useRef<() => void>(() => undefined);
    const onDataRef = useRef(onData);
    // The parent keys this surface by board id, so these values are the
    // immutable mount snapshot. Live connection changes are handled by the
    // dedicated effect below and must not recreate xterm or lose scrollback.
    const mountConfigRef = useRef({ initialData, connected, placeholder, variant, appearance });
    onDataRef.current = onData;

    drainTerminalWritesRef.current = () => {
      const terminal = terminalRef.current;
      if (!terminal || terminalWriteInFlightRef.current) return;

      const replacement = terminalReplacePendingRef.current;
      if (replacement) {
        terminalReplacePendingRef.current = null;
        terminalWriteQueueRef.current = replacement.length ? [replacement] : [];
        terminal.reset();
      }

      const queued = terminalWriteQueueRef.current.splice(0);
      if (!queued.length) return;
      const batch =
        queued.length === 1
          ? queued[0]
          : (() => {
              const combined = new Uint8Array(
                queued.reduce((length, chunk) => length + chunk.length, 0)
              );
              let offset = 0;
              for (const chunk of queued) {
                combined.set(chunk, offset);
                offset += chunk.length;
              }
              return combined;
            })();
      terminalWriteInFlightRef.current = true;
      terminal.write(batch, () => {
        terminalWriteInFlightRef.current = false;
        drainTerminalWritesRef.current();
      });
    };

    const enqueueTerminalWrite = useCallback((data: Uint8Array) => {
      if (!data.length) return;
      terminalWriteQueueRef.current.push(data.slice());
      drainTerminalWritesRef.current();
    }, []);

    useEffect(() => {
      const host = hostRef.current;
      if (!host) return;
      host.replaceChildren();
      const { initialData, connected, placeholder, variant, appearance } = mountConfigRef.current;
      const isSerial = variant === 'serial';
      const isLight = appearance === 'light';
      const terminal = new Terminal({
        allowProposedApi: false,
        // Boot logs and many serial consoles emit a bare LF. Treat it as a
        // real newline so the cursor does not keep its previous column and
        // overwrite/interleave subsequent output.
        convertEol: true,
        cursorBlink: connected,
        cursorStyle: 'block',
        disableStdin: !connected,
        fontFamily: '"JetBrains Mono", Consolas, monospace',
        fontSize: 13,
        // Keep a useful live viewport. The complete byte stream lives in the
        // backend capture and can be replayed/exported without growing xterm.
        scrollback: 10000,
        theme: {
          background: isLight ? '#f8fbff' : '#10141c',
          foreground: isLight ? '#24364d' : isSerial ? '#bce8c6' : '#c9d7ff',
          cursor: isLight ? '#087ff5' : isSerial ? '#6ee7b7' : '#a78bfa',
          selectionBackground: isLight ? '#cfe6ff' : isSerial ? '#294b42' : '#3b4167',
          black: isLight ? '#1f2937' : '#1e2430',
          red: isLight ? '#c62828' : '#f38ba8',
          green: isLight ? '#16813f' : isSerial ? '#86efac' : '#a6e3a1',
          yellow: isLight ? '#9a6100' : '#f9e2af',
          blue: isLight ? '#006fde' : isSerial ? '#93c5fd' : '#82aaff',
          magenta: isLight ? '#7c3aed' : isSerial ? '#d8b4fe' : '#c4b5fd',
          cyan: isLight ? '#087c8f' : isSerial ? '#67e8f9' : '#7dd3fc',
          white: isLight ? '#52657c' : '#cdd6f4',
          brightBlack: isLight ? '#7b8da3' : isSerial ? '#6f8f7a' : '#7784aa',
          brightGreen: isLight ? '#0f9f4a' : isSerial ? '#bbf7d0' : '#b7f7b2',
          brightBlue: isLight ? '#087ff5' : isSerial ? '#bfdbfe' : '#adc6ff',
          brightMagenta: isLight ? '#9333ea' : isSerial ? '#e9d5ff' : '#ddd6fe',
          brightCyan: isLight ? '#0891b2' : isSerial ? '#a5f3fc' : '#bae6fd',
          brightWhite: isLight ? '#0d1830' : '#ffffff',
        },
      });
      const fit = new FitAddon();
      terminal.loadAddon(fit);
      terminal.open(host);
      terminalRef.current = terminal;
      if (initialData.length) {
        enqueueTerminalWrite(new Uint8Array(initialData.slice(-TERMINAL_HISTORY_BYTES)));
      } else terminal.writeln(`\x1b[90m${placeholder}\x1b[0m`);
      const dataSubscription = terminal.onData((data) => onDataRef.current(data));
      let fitFrame = 0;
      const scheduleFit = () => {
        window.cancelAnimationFrame(fitFrame);
        fitFrame = window.requestAnimationFrame(() => {
          const host = hostRef.current;
          // BoardConsole is also mounted behind the overview page. Do not ask
          // xterm to fit a hidden/zero-sized surface; ResizeObserver will run
          // again as soon as the dedicated terminal view becomes visible.
          if (!host || host.clientWidth === 0 || host.clientHeight === 0) return;
          fit.fit();
        });
      };
      const resizeObserver = new ResizeObserver(scheduleFit);
      resizeObserver.observe(host);
      scheduleFit();
      window.addEventListener('resize', scheduleFit);
      return () => {
        window.cancelAnimationFrame(fitFrame);
        resizeObserver.disconnect();
        window.removeEventListener('resize', scheduleFit);
        dataSubscription.dispose();
        terminalRef.current = null;
        terminalWriteQueueRef.current = [];
        terminalWriteInFlightRef.current = false;
        terminalReplacePendingRef.current = null;
        terminal.dispose();
        host.replaceChildren();
      };
    }, [enqueueTerminalWrite]);

    useEffect(() => {
      if (terminalRef.current) {
        terminalRef.current.options.disableStdin = !connected;
        terminalRef.current.options.cursorBlink = connected;
      }
    }, [connected]);

    useImperativeHandle(
      ref,
      () => ({
        write: enqueueTerminalWrite,
        replace: (data) => {
          terminalWriteQueueRef.current = [];
          terminalReplacePendingRef.current = data.slice();
          drainTerminalWritesRef.current();
        },
        clear: () => terminalRef.current?.clear(),
      }),
      [enqueueTerminalWrite]
    );

    return (
      <div
        ref={hostRef}
        className={`board-xterm is-${variant}`}
        data-input-state={connected ? 'ready' : 'unavailable'}
      />
    );
  }
);
TerminalSurface.displayName = 'TerminalSurface';

const emptyRuntime = (): BoardRuntime => ({
  serialConnected: false,
  serialOpening: false,
  serialPort: '',
  serialData: [],
  serialSessionId: '',
  serialCapturedBytes: 0,
  serialSynchronizedBytes: 0,
  serialCaptureError: '',
  serialConnectionError: '',
  serialEmitErrors: 0,
  flashLines: [],
  flashProgress: 0,
  flashState: '等待烧录',
  flashPhase: 'idle',
  flashPartition: '',
  flashPartitionProgress: null,
  flashRate: null,
  flashUsbSpeed: 'unknown',
  flashUsbTheoreticalMbps: null,
  flashCommittedBytes: null,
  flashTotalBytes: null,
  flashStorageType: '',
  flashPhysicalVerify: 'not_verified',
  flashBootVerify: 'not_verified',
  flashVerify: 'pending',
  flashPostActionKind: 'unknown',
  flashPostAction: 'pending',
  flashTaskStatus: 'idle',
});

function isFlashTask(task: WorkbenchTaskHistory) {
  const kind = task.kind.toLowerCase();
  return kind.includes('flash') || kind === 'mainline_boot';
}

function taskStateLabel(task: WorkbenchTaskHistory) {
  if (task.details?.storageType === 'spinor' && task.status === 'success') {
    return flashOutcomeLabel(
      task.status,
      'spinor',
      task.details.bootVerifyState,
      task.details.physicalVerifyState
    );
  }
  const mainline = task.kind.toLowerCase() === 'mainline_boot';
  if (task.status === 'success' || task.status === 'completed')
    return mainline ? '主线 NAND 烧录完成' : '烧录完成';
  if (task.status === 'failed') return mainline ? '主线启动失败' : '烧录失败';
  if (task.status === 'cancelled') return '已取消';
  return FLASH_PHASE_LABELS[task.phase]
    ? `正在${FLASH_PHASE_LABELS[task.phase]}`
    : mainline
      ? '正在启动主线系统'
      : '正在烧录';
}

function formatCompactUptime(seconds: number | null | undefined) {
  if (seconds == null) return '—';
  const days = Math.floor(seconds / 86_400);
  const hours = Math.floor((seconds % 86_400) / 3_600);
  const minutes = Math.floor((seconds % 3_600) / 60);
  if (days) return `${days} 天 ${hours} 小时`;
  if (hours) return `${hours} 小时 ${minutes} 分钟`;
  return `${minutes} 分钟`;
}

function boardId(board: WorkbenchDevice | undefined) {
  return String(board?.id ?? '');
}

function appendBytes(current: number[], data: number[]) {
  const maximum = TERMINAL_HISTORY_BYTES;
  if (data.length >= maximum) return data.slice(-maximum);
  const retained = Math.min(current.length, maximum - data.length);
  return current.slice(current.length - retained).concat(data);
}

function appendLine(lines: string[], line: string) {
  return [...lines, line].slice(-600);
}

function formatTaskTime(timestamp: number | null) {
  return timestamp ? new Date(timestamp * 1000).toLocaleString() : '时间未知';
}

export const BoardConsole: React.FC<BoardConsoleProps> = ({
  applicationCandidateId,
  view,
  devices,
  selectedDevice,
  isActive,
  taskHistory,
  serialDetected,
  adbDetected,
  flashDetected,
  flashBound,
  flashProtocol,
  savedSerialPort,
  aiChannel,
  onOpenSettings,
  workspace,
}) => {
  const { isDark } = useTheme();
  const terminalAppearance = isDark ? 'dark' : 'light';
  const [runtimes, setRuntimes] = useState<Record<string, BoardRuntime>>({});
  const [serialInputNotice, setSerialInputNotice] = useState('');
  const boardsRef = useRef<WorkbenchDevice[]>([]);
  const selectedDeviceRef = useRef<WorkbenchDevice | undefined>(selectedDevice);
  const flashTargetRef = useRef('');
  const flashTasksRef = useRef<Record<number, string>>({});
  const workbenchTasksRef = useRef<Record<string, string>>({});
  const latestFlashTaskRef = useRef(new Map<number, string>());
  const openedPortsRef = useRef(new Set<string>());
  const serialPortOwnersRef = useRef(new Map<string, string>());
  const serialFailureFingerprintRef = useRef(new Map<string, string>());
  const terminalReconcileRunningRef = useRef(false);
  const serialTerminalRef = useRef<TerminalHandle>(null);
  const flashLogRef = useRef<HTMLPreElement>(null);
  const serialWriteQueueRef = useRef(Promise.resolve());
  const pendingSerialWriteRef = useRef<number[]>([]);
  const serialWriteTimerRef = useRef<number | null>(null);
  const serialWritePortRef = useRef('');
  const pendingSerialDataRef = useRef(new Map<string, number[]>());
  const serialFlushFrameRef = useRef<number | null>(null);
  const serialFlushTimerRef = useRef<number | null>(null);
  const pendingSerialHistoryRef = useRef(new Map<string, number[]>());
  const serialHistoryTimerRef = useRef<number | null>(null);
  const serialCaptureCursorsRef = useRef(
    new Map<string, { port: string; sessionId: string; nextOffset: number }>()
  );
  const serialCaptureChainsRef = useRef(new Map<string, Promise<void>>());

  boardsRef.current = devices;
  selectedDeviceRef.current = selectedDevice;
  const selected = selectedDevice ?? devices[0];
  const selectedId = boardId(selected);
  const runtime = runtimes[selectedId] ?? emptyRuntime();
  const latestFlashTask = taskHistory
    .filter((task) => String(task.deviceId ?? '') === selectedId && isFlashTask(task))
    .sort((left, right) => right.startedAt - left.startedAt)[0];

  useEffect(() => {
    if (view !== 'flash' && view !== 'workspace') return;
    const output = flashLogRef.current;
    if (output) output.scrollTop = output.scrollHeight;
  }, [runtime.flashLines.length, selectedId, view]);
  const runningFlashTask = latestFlashTask?.status === 'running' ? latestFlashTask : undefined;
  const showFlashOutput = Boolean(
    runningFlashTask || latestFlashTask || (!latestFlashTask && runtime.flashLines.length)
  );
  const displayedFlashProgress = showFlashOutput ? runtime.flashProgress : 0;

  useEffect(() => {
    setSerialInputNotice('');
  }, [selectedId]);

  const updateRuntime = useCallback(
    (id: string, update: (current: BoardRuntime) => BoardRuntime) => {
      if (!id) return;
      setRuntimes((current) => {
        const previous = current[id] ?? emptyRuntime();
        const next = update(previous);
        if (sameBoardRuntime(previous, next)) return current;
        return { ...current, [id]: next };
      });
    },
    []
  );

  const flushSerialHistory = useCallback(() => {
    serialHistoryTimerRef.current = null;
    const pending = pendingSerialHistoryRef.current;
    pendingSerialHistoryRef.current = new Map();
    for (const [id, data] of pending) {
      updateRuntime(id, (current) => ({
        ...current,
        serialConnected: true,
        serialData: appendBytes(current.serialData, data),
      }));
    }
  }, [updateRuntime]);

  const flushSerialData = useCallback(() => {
    if (serialFlushFrameRef.current != null) {
      window.cancelAnimationFrame(serialFlushFrameRef.current);
    }
    if (serialFlushTimerRef.current != null) {
      window.clearTimeout(serialFlushTimerRef.current);
    }
    serialFlushFrameRef.current = null;
    serialFlushTimerRef.current = null;
    const pending = pendingSerialDataRef.current;
    pendingSerialDataRef.current = new Map();
    const visibleId = boardId(selectedDeviceRef.current ?? boardsRef.current[0]);
    for (const [id, data] of pending) {
      if (id === visibleId) {
        serialTerminalRef.current?.write(new Uint8Array(data));
      }
      const history = pendingSerialHistoryRef.current.get(id);
      if (history) history.push(...data);
      else pendingSerialHistoryRef.current.set(id, data);
    }
    if (pending.size && serialHistoryTimerRef.current == null) {
      serialHistoryTimerRef.current = window.setTimeout(flushSerialHistory, 1000);
    }
  }, [flushSerialHistory]);

  const queueSerialData = useCallback(
    (id: string, data: number[]) => {
      const queued = pendingSerialDataRef.current.get(id);
      if (queued) queued.push(...data);
      else pendingSerialDataRef.current.set(id, [...data]);
      if (serialFlushFrameRef.current == null) {
        serialFlushFrameRef.current = window.requestAnimationFrame(flushSerialData);
        // requestAnimationFrame may be throttled while the Workbench is
        // covered or the window is not focused. The timer keeps the queue
        // bounded; offset replay remains the source of truth on return.
        serialFlushTimerRef.current = window.setTimeout(flushSerialData, 16);
      }
    },
    [flushSerialData]
  );

  useEffect(
    () => () => {
      if (serialFlushFrameRef.current != null) {
        window.cancelAnimationFrame(serialFlushFrameRef.current);
      }
      if (serialFlushTimerRef.current != null) {
        window.clearTimeout(serialFlushTimerRef.current);
      }
      if (serialHistoryTimerRef.current != null) {
        window.clearTimeout(serialHistoryTimerRef.current);
      }
      pendingSerialDataRef.current.clear();
      pendingSerialHistoryRef.current.clear();
      serialCaptureCursorsRef.current.clear();
      serialCaptureChainsRef.current.clear();
    },
    []
  );

  const reconcileSerialCapture = useCallback(
    async (
      id: string,
      port: string,
      sessionId: string,
      targetOffset: number,
      eventChunk?: { startOffset: number; endOffset: number; data: number[] }
    ) => {
      let cursor = serialCaptureCursorsRef.current.get(id);
      if (!cursor || cursor.port !== port || cursor.sessionId !== sessionId) {
        cursor = { port, sessionId, nextOffset: 0 };
        serialCaptureCursorsRef.current.set(id, cursor);
        pendingSerialDataRef.current.delete(id);
        pendingSerialHistoryRef.current.delete(id);
        if (id === boardId(selectedDeviceRef.current ?? boardsRef.current[0])) {
          serialTerminalRef.current?.replace(new Uint8Array());
        }
        updateRuntime(id, (current) => ({
          ...current,
          serialData: [],
          serialSessionId: sessionId,
          serialCapturedBytes: targetOffset,
          serialSynchronizedBytes: 0,
          serialCaptureError: '',
          serialEmitErrors: 0,
        }));
      }

      const replayTo = async (endOffset: number) => {
        while (cursor.nextOffset < endOffset) {
          const chunk = await invokeCommand('serial_capture_read', {
            port,
            sessionId,
            offset: cursor.nextOffset,
            maxBytes: Math.min(256 * 1024, endOffset - cursor.nextOffset),
          });
          if (chunk.startOffset !== cursor.nextOffset || chunk.nextOffset <= cursor.nextOffset) {
            throw new Error(
              `SERIAL_CAPTURE_GAP:expected=${cursor.nextOffset}:start=${chunk.startOffset}:next=${chunk.nextOffset}`
            );
          }
          queueSerialData(id, chunk.data);
          cursor.nextOffset = chunk.nextOffset;
        }
      };

      if (eventChunk) {
        if (eventChunk.startOffset > cursor.nextOffset) {
          await replayTo(eventChunk.startOffset);
        }
        if (eventChunk.endOffset > cursor.nextOffset) {
          const skip = Math.max(0, cursor.nextOffset - eventChunk.startOffset);
          const expectedLength = eventChunk.endOffset - eventChunk.startOffset;
          if (eventChunk.data.length !== expectedLength) {
            throw new Error(
              `SERIAL_EVENT_LENGTH:expected=${expectedLength}:actual=${eventChunk.data.length}`
            );
          }
          queueSerialData(id, eventChunk.data.slice(skip));
          cursor.nextOffset = eventChunk.endOffset;
        }
      }
      if (cursor.nextOffset < targetOffset) {
        await replayTo(targetOffset);
      }
      updateRuntime(id, (current) => ({
        ...current,
        serialSessionId: sessionId,
        serialCapturedBytes: Math.max(current.serialCapturedBytes, targetOffset),
        serialSynchronizedBytes: cursor.nextOffset,
      }));
    },
    [queueSerialData, updateRuntime]
  );

  const enqueueSerialCaptureWork = useCallback(
    (id: string, work: () => Promise<void>) => {
      const previous = serialCaptureChainsRef.current.get(id) ?? Promise.resolve();
      const next = previous.then(work).catch((value) => {
        const message = formatErrorMessage(value);
        console.error('Serial capture reconciliation failed', value);
        updateRuntime(id, (current) => ({ ...current, serialCaptureError: message }));
      });
      serialCaptureChainsRef.current.set(id, next);
      return next;
    },
    [updateRuntime]
  );

  useEffect(() => {
    const mapping: Record<string, string> = {};
    for (const task of taskHistory) {
      if (task.deviceId) mapping[task.taskId] = String(task.deviceId);
    }
    workbenchTasksRef.current = mapping;
    const latestByDevice = new Map<number, WorkbenchTaskHistory>();
    for (const task of taskHistory) {
      if (task.deviceId && isFlashTask(task) && !latestByDevice.has(task.deviceId)) {
        latestByDevice.set(task.deviceId, task);
      }
    }
    for (const [deviceId, task] of latestByDevice) {
      const details = task.details ?? {};
      const outputTail = details.outputTail ?? [];
      const previousTaskId = latestFlashTaskRef.current.get(deviceId);
      const isNewTask = previousTaskId !== task.taskId;
      latestFlashTaskRef.current.set(deviceId, task.taskId);
      updateRuntime(String(deviceId), (current) => ({
        ...current,
        flashProgress: task.progressPct ?? details.progressPct ?? current.flashProgress,
        flashState: taskStateLabel(task),
        flashPhase: deriveFlashPhase(task.phase || details.phase || current.flashPhase, outputTail),
        flashPartition: details.currentPartition ?? '',
        flashPartitionProgress: details.partitionProgressPct ?? null,
        flashRate: details.bytesPerSecond ?? null,
        flashUsbSpeed: details.usbSpeed ?? 'unknown',
        flashUsbTheoreticalMbps: details.usbTheoreticalMbps ?? null,
        flashCommittedBytes: details.committedBytes ?? null,
        flashTotalBytes: details.totalBytes ?? null,
        flashStorageType: details.storageType ?? '',
        flashPhysicalVerify: details.physicalVerifyState ?? 'not_verified',
        flashBootVerify: details.bootVerifyState ?? 'not_verified',
        flashVerify: details.verifyState ?? 'pending',
        flashPostActionKind: details.postFlashAction ?? 'unknown',
        flashPostAction: details.postFlashState ?? 'pending',
        flashTaskStatus: task.status,
        flashLines:
          isNewTask && task.status === 'running'
            ? []
            : isNewTask && outputTail.length
              ? restoreFlashOutputTail(outputTail, task.error)
              : task.status === 'failed' &&
                  task.error &&
                  !current.flashLines.includes(`[SYSTEM] ${task.error}`)
                ? appendLine(current.flashLines, `[SYSTEM] ${task.error}`)
                : current.flashLines,
      }));
    }
  }, [taskHistory, updateRuntime]);

  useEffect(() => {
    const unlisteners: Array<() => void> = [];
    let disposed = false;
    const register = (subscription: Promise<() => void>) => {
      subscription
        .then((unlisten) => {
          // Tauri's listen API resolves asynchronously. React Strict Mode can
          // clean up this effect before that happens; retaining a late
          // listener makes every incoming byte (including input echo) appear
          // twice. Registering each cleanup independently also avoids leaking
          // the other listeners if one subscription fails.
          if (disposed) unlisten();
          else unlisteners.push(unlisten);
        })
        .catch((value) => {
          if (!disposed) {
            console.error('Failed to subscribe to a board console event', value);
          }
        });
    };

    [
      subscribeEvent('serial-data-received', (payload) => {
        const id =
          serialPortOwnersRef.current.get(payload.port) ||
          boardId(boardsRef.current.find((item) => item.serialPort === payload.port));
        if (!id) return;
        void enqueueSerialCaptureWork(id, async () => {
          await reconcileSerialCapture(
            id,
            payload.port,
            payload.sessionId,
            payload.endOffset,
            payload.data.length
              ? {
                  startOffset: payload.startOffset,
                  endOffset: payload.endOffset,
                  data: payload.data,
                }
              : undefined
          );
          if (!payload.data.length) {
            flushSerialData();
            updateRuntime(id, (current) => ({
              ...current,
              serialConnected: false,
              serialOpening: false,
            }));
          }
        });
      }),
      subscribeEvent('flash-progress', (payload) => {
        let id = flashTasksRef.current[payload.taskId];
        if (!id) {
          id = flashTargetRef.current || boardId(selectedDeviceRef.current);
          if (id) flashTasksRef.current[payload.taskId] = id;
        }
        if (!id) return;
        updateRuntime(id, (current) => ({
          ...current,
          flashProgress: payload.overallPercent,
          flashState: payload.stageLabel,
          flashPhase: payload.stageId || current.flashPhase,
          flashPartition: payload.currentPartition ?? current.flashPartition,
          flashPartitionProgress: payload.partitionPercent ?? current.flashPartitionProgress,
          flashTaskStatus: 'running',
        }));
      }),
      subscribeEvent('flash-log', (payload) => {
        const id =
          flashTasksRef.current[payload.taskId] ||
          flashTargetRef.current ||
          boardId(selectedDeviceRef.current);
        if (!id) return;
        flashTasksRef.current[payload.taskId] = id;
        updateRuntime(id, (current) => ({
          ...current,
          flashLines: appendLine(
            current.flashLines,
            `[${payload.level.toUpperCase()}] ${payload.message}`
          ),
        }));
      }),
      subscribeEvent('flash-state', (payload) => {
        const id =
          flashTasksRef.current[payload.taskId] ||
          flashTargetRef.current ||
          boardId(selectedDeviceRef.current);
        if (!id) return;
        flashTasksRef.current[payload.taskId] = id;
        updateRuntime(id, (current) => ({
          ...current,
          flashProgress: payload.status === 'completed' ? 100 : current.flashProgress,
          flashState:
            payload.status === 'completed'
              ? current.flashStorageType === 'spinor'
                ? flashOutcomeLabel(
                    'success',
                    'spinor',
                    current.flashBootVerify,
                    current.flashPhysicalVerify
                  )
                : '烧录完成'
              : payload.status === 'failed'
                ? '烧录失败'
                : payload.status === 'cancelled'
                  ? '已取消'
                  : '开始烧录',
          flashTaskStatus: payload.status,
          flashLines: appendLine(
            current.flashLines,
            `[SYSTEM] ${payload.message || payload.status}`
          ),
        }));
      }),
      subscribeEvent('workbench-task-event', (payload) => {
        if (payload.phase !== 'flash') return;
        // Unknown tasks are restored from scoped history on the next refresh;
        // never attribute another board's MCP output to the selected board.
        const id = workbenchTasksRef.current[payload.taskId];
        if (!id) return;
        const progress =
          payload.current != null && payload.total
            ? (payload.current / payload.total) * 100
            : payload.current;
        updateRuntime(id, (current) => ({
          ...current,
          flashProgress: progress ?? current.flashProgress,
          flashState:
            payload.status === 'success'
              ? current.flashStorageType === 'spinor'
                ? flashOutcomeLabel(
                    'success',
                    'spinor',
                    current.flashBootVerify,
                    current.flashPhysicalVerify
                  )
                : '烧录完成'
              : payload.status === 'failed'
                ? '烧录失败'
                : payload.status === 'cancelled'
                  ? '已取消'
                  : current.flashState === '等待烧录'
                    ? '正在烧录'
                    : current.flashState,
          flashTaskStatus: payload.status,
          flashLines: payload.message
            ? appendLine(current.flashLines, `[${payload.stream.toUpperCase()}] ${payload.message}`)
            : current.flashLines,
        }));
      }),
      subscribeEvent('workbench-flash-telemetry', (payload) => {
        const id = workbenchTasksRef.current[payload.taskId];
        if (!id) return;
        const details = payload.telemetry;
        updateRuntime(id, (current) => ({
          ...current,
          flashProgress: details.progressPct ?? current.flashProgress,
          flashState:
            details.phase === 'complete'
              ? current.flashStorageType === 'spinor'
                ? flashOutcomeLabel(
                    'success',
                    'spinor',
                    current.flashBootVerify,
                    current.flashPhysicalVerify
                  )
                : '烧录完成'
              : FLASH_PHASE_LABELS[details.phase ?? '']
                ? `正在${FLASH_PHASE_LABELS[details.phase ?? '']}`
                : current.flashState,
          flashPhase: details.phase ?? current.flashPhase,
          flashPartition: details.currentPartition ?? '',
          flashPartitionProgress: details.partitionProgressPct ?? null,
          flashRate: details.bytesPerSecond ?? null,
          flashUsbSpeed: details.usbSpeed ?? 'unknown',
          flashUsbTheoreticalMbps: details.usbTheoreticalMbps ?? null,
          flashCommittedBytes: details.committedBytes ?? null,
          flashTotalBytes: details.totalBytes ?? null,
          flashStorageType: details.storageType ?? '',
          flashPhysicalVerify: details.physicalVerifyState ?? 'not_verified',
          flashBootVerify: details.bootVerifyState ?? 'not_verified',
          flashVerify: details.verifyState ?? 'pending',
          flashPostActionKind: details.postFlashAction ?? current.flashPostActionKind,
          flashPostAction: details.postFlashState ?? 'pending',
        }));
      }),
    ].forEach(register);

    return () => {
      disposed = true;
      unlisteners.forEach((unlisten) => unlisten());
      // Physical serial and ADB sessions are application-owned. Page changes,
      // React remounts, and board selection must not tear down another board's
      // live terminal.
    };
  }, [enqueueSerialCaptureWork, flushSerialData, reconcileSerialCapture, updateRuntime]);

  useEffect(() => {
    if (!isActive) return;
    let disposed = false;
    const synchronize = () => {
      for (const [port, id] of serialPortOwnersRef.current) {
        void invokeCommand('serial_capture_status', { port })
          .then((status) => {
            if (disposed) return;
            updateRuntime(id, (current) => ({
              ...current,
              serialCapturedBytes: status.nextOffset,
              serialCaptureError: status.captureError ?? '',
              serialEmitErrors: status.emitErrors,
            }));
            return enqueueSerialCaptureWork(id, () =>
              reconcileSerialCapture(id, port, status.sessionId, status.nextOffset)
            );
          })
          .catch((value) => {
            if (!disposed) {
              updateRuntime(id, (current) => ({
                ...current,
                serialCaptureError: formatErrorMessage(value),
              }));
            }
          });
      }
    };
    synchronize();
    const timer = window.setInterval(synchronize, 500);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, [enqueueSerialCaptureWork, isActive, reconcileSerialCapture, updateRuntime]);

  const openSerial = useCallback(
    async (target: WorkbenchDevice, timeoutMs = 1_000, silent = false) => {
      const id = boardId(target);
      if (!target.serialPort) return false;
      const knownPort = [...serialPortOwnersRef.current].find(([, owner]) => owner === id)?.[0];
      if (knownPort) {
        const isOpen = await invokeCommand('serial_is_open', { port: knownPort }).catch(
          () => false
        );
        if (!isOpen) {
          serialPortOwnersRef.current.delete(knownPort);
          openedPortsRef.current.delete(knownPort);
          await invokeCommand('serial_close', { port: knownPort }).catch(() => undefined);
        }
      }
      updateRuntime(id, (current) => ({ ...current, serialOpening: true }));
      try {
        const resolved = await invokeCommand('serial_wait_and_open', {
          identity: {
            port: target.serialPort,
            vid: target.serialVid,
            pid: target.serialPid,
            serialNumber: target.serialNumber,
            interfaceNumber: target.serialInterfaceNumber,
          },
          baudRate: target.baudRate,
          dataBits: 8,
          stopBits: 1,
          parity: 'none',
          flowControl: 'none',
          timeoutMs,
          auditContext: {
            actorType: 'user',
            actorId: 'local-user',
            scopeType: 'linux_board',
            scopeId: `linux:${id}`,
            sessionId: `serial-linux-${id}-${Date.now()}`,
            sessionTitle: `${target.name} · ${target.serialPort || '串口'}`,
          },
        });
        const alreadyOpen = openedPortsRef.current.has(resolved.name);
        if (!alreadyOpen) {
          openedPortsRef.current.add(resolved.name);
        }
        for (const [port, owner] of serialPortOwnersRef.current) {
          if (owner === id && port !== resolved.name) {
            serialPortOwnersRef.current.delete(port);
            openedPortsRef.current.delete(port);
            await invokeCommand('serial_close', { port }).catch(() => undefined);
          }
        }
        serialPortOwnersRef.current.set(resolved.name, id);
        updateRuntime(id, (current) => ({
          ...current,
          serialConnected: true,
          serialOpening: false,
          serialPort: resolved.name,
          serialConnectionError: '',
        }));
        serialFailureFingerprintRef.current.delete(id);
        return true;
      } catch (value) {
        updateRuntime(id, (current) => ({
          ...current,
          serialConnected: false,
          serialOpening: false,
          serialConnectionError: formatErrorMessage(value),
          serialData: silent
            ? current.serialData
            : appendBytes(
                current.serialData,
                Array.from(
                  new TextEncoder().encode(`\r\n[串口连接失败] ${formatErrorMessage(value)}\r\n`)
                )
              ),
        }));
        const fingerprint = `${target.serialPort}:${target.baudRate}`;
        if (serialFailureFingerprintRef.current.get(id) !== fingerprint) {
          serialFailureFingerprintRef.current.set(id, fingerprint);
          void appendRecordEvent({
            actorType: 'service',
            actorId: 'serial-manager',
            category: 'serial',
            action: 'serial.connect_failed',
            scopeType: 'board',
            scopeId: `linux:${id}`,
            result: 'failed',
            errorCode: 'SERIAL_OPEN_FAILED',
            summary: `${target.name} 串口连接失败`,
            metadata: {
              requestedPort: target.serialPort,
              baudRate: target.baudRate,
              boardName: target.name,
            },
          }).catch((error) => console.warn('Failed to record serial failure', error));
        }
        return false;
      }
    },
    [updateRuntime]
  );

  useEffect(() => {
    if (!isActive) return;
    let cancelled = false;
    const reconcileBoardTerminals = async () => {
      if (terminalReconcileRunningRef.current) return;
      terminalReconcileRunningRef.current = true;
      try {
        const configuredBoards = boardsRef.current.filter((board) => board.serialPort);
        const configuredBoardIds = new Set(configuredBoards.map((board) => boardId(board)));
        for (const [port, owner] of [...serialPortOwnersRef.current]) {
          if (!configuredBoardIds.has(owner)) {
            serialPortOwnersRef.current.delete(port);
            openedPortsRef.current.delete(port);
            await invokeCommand('serial_close', { port }).catch(() => undefined);
            updateRuntime(owner, (current) => ({
              ...current,
              serialConnected: false,
              serialOpening: false,
            }));
          }
        }
        for (const target of configuredBoards) {
          if (cancelled) break;
          await openSerial(target, 250, true).catch(() => undefined);
        }
      } finally {
        terminalReconcileRunningRef.current = false;
      }
    };
    void reconcileBoardTerminals();
    const timer = window.setInterval(() => {
      if (!cancelled) void reconcileBoardTerminals();
    }, 3000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [isActive, openSerial, updateRuntime]);

  useEffect(() => {
    const target = selectedDeviceRef.current;
    if (!isActive || !target?.id) return;
    if (!serialDetected) {
      updateRuntime(boardId(target), (current) => ({
        ...current,
        serialConnected: false,
        serialOpening: false,
      }));
    }
  }, [isActive, selected?.id, serialDetected, updateRuntime]);

  const flushPendingSerialWrite = useCallback(() => {
    if (serialWriteTimerRef.current != null) {
      window.clearTimeout(serialWriteTimerRef.current);
      serialWriteTimerRef.current = null;
    }
    const port = serialWritePortRef.current;
    const bytes = pendingSerialWriteRef.current.splice(0);
    if (!port || !bytes.length) return;
    const chunks: number[][] = [];
    for (let offset = 0; offset < bytes.length; offset += SERIAL_WRITE_CHUNK_BYTES) {
      chunks.push(bytes.slice(offset, offset + SERIAL_WRITE_CHUNK_BYTES));
    }
    serialWriteQueueRef.current = serialWriteQueueRef.current
      .catch(() => undefined)
      .then(async () => {
        for (const data of chunks) {
          if (serialWritePortRef.current !== port) return;
          await invokeCommand('serial_write', { port, data });
        }
      })
      .catch((value) => {
        setSerialInputNotice(`串口写入失败：${formatErrorMessage(value)}`);
        serialTerminalRef.current?.write(
          new TextEncoder().encode(`\r\n[写入失败] ${formatErrorMessage(value)}\r\n`)
        );
      });
  }, []);

  useEffect(() => {
    return () => {
      if (serialWriteTimerRef.current != null) {
        window.clearTimeout(serialWriteTimerRef.current);
        serialWriteTimerRef.current = null;
      }
      pendingSerialWriteRef.current = [];
      serialWritePortRef.current = '';
    };
  }, [runtime.serialPort]);

  const sendSerial = useCallback(
    (data: string) => {
      if (!runtime.serialPort || !runtime.serialConnected) {
        setSerialInputNotice('串口当前不可输入，请等待自动接管或点击“重新接管串口”。');
        return;
      }
      setSerialInputNotice('');
      const port = runtime.serialPort;
      const bytes = Array.from(new TextEncoder().encode(data));
      if (serialWritePortRef.current && serialWritePortRef.current !== port) {
        pendingSerialWriteRef.current = [];
        if (serialWriteTimerRef.current != null) {
          window.clearTimeout(serialWriteTimerRef.current);
          serialWriteTimerRef.current = null;
        }
      }
      serialWritePortRef.current = port;
      pendingSerialWriteRef.current.push(...bytes);
      if (data.includes('\r') || data.includes('\n')) {
        flushPendingSerialWrite();
      } else if (serialWriteTimerRef.current == null) {
        serialWriteTimerRef.current = window.setTimeout(
          flushPendingSerialWrite,
          SERIAL_WRITE_BATCH_MS
        );
      }
    },
    [flushPendingSerialWrite, runtime.serialConnected, runtime.serialPort]
  );

  const reclaimSerial = useCallback(async () => {
    if (!selected?.serialPort) {
      setSerialInputNotice('当前单板尚未绑定串口。');
      return;
    }
    setSerialInputNotice('正在重新接管串口…');
    const connected = await openSerial(selected, selected.serialReconnectTimeoutMs);
    setSerialInputNotice(
      connected ? '串口已由 LYNX 接管，用户可输入。' : '重新接管失败，请检查占用、连接或权限。'
    );
  }, [openSerial, selected]);

  const pasteSerial = useCallback(async () => {
    if (!runtime.serialPort || !runtime.serialConnected) {
      setSerialInputNotice('串口当前不可输入，请等待自动接管或点击“重新接管串口”。');
      return;
    }
    try {
      const text = await navigator.clipboard.readText();
      if (!text) {
        setSerialInputNotice('剪贴板中没有可粘贴的文本。');
        return;
      }
      sendSerial(text);
      setSerialInputNotice(
        `已提交 ${new TextEncoder().encode(text).length} 字节，按顺序分块发送。`
      );
    } catch (value) {
      setSerialInputNotice(`读取剪贴板失败：${formatErrorMessage(value)}`);
    }
  }, [runtime.serialConnected, runtime.serialPort, sendSerial]);

  const exportSerialCapture = useCallback(async () => {
    if (!runtime.serialPort || !runtime.serialSessionId) return;
    const target = await save({
      defaultPath: `${selected?.name || 'board'}-${runtime.serialSessionId}.bin`,
      filters: [{ name: 'Serial capture', extensions: ['bin', 'log'] }],
    });
    if (!target) return;
    try {
      await invokeCommand('serial_capture_export', {
        port: runtime.serialPort,
        sessionId: runtime.serialSessionId,
        target,
      });
      updateRuntime(selectedId, (current) => ({ ...current, serialCaptureError: '' }));
    } catch (value) {
      updateRuntime(selectedId, (current) => ({
        ...current,
        serialCaptureError: formatErrorMessage(value),
      }));
    }
  }, [runtime.serialPort, runtime.serialSessionId, selected?.name, selectedId, updateRuntime]);

  if (!selected?.id) return null;
  const serialReady = runtime.serialConnected && !!runtime.serialPort && !!selected.serialPort;
  const adbReady = adbDetected && !!selected.adbSerial;

  if (view === 'workspace' && workspace) {
    const scopedTasks = taskHistory
      .filter((task) => String(task.deviceId ?? '') === selectedId)
      .slice(0, 40);
    const taskLog = runtime.flashLines.length
      ? runtime.flashLines.slice(-600)
      : scopedTasks.map(
          (task) =>
            `[${formatTaskTime(task.startedAt)}] ${task.kind} · ${task.status}${task.error ? ` · ${task.error}` : ''}`
        );
    return (
      <LinuxTerminalWorkspace
        device={selected}
        projectName={workspace.projectName}
        hostLabel={workspace.hostLabel}
        onRequestExit={workspace.onRequestExit}
        panes={[
          {
            id: 'serial',
            eyebrow: 'UART / SERIAL',
            title: `${selected.name} · UART`,
            detail: serialReady
              ? `${runtime.serialPort} · ${selected.baudRate}`
              : selected.serialPort || '未绑定串口',
            icon: 'terminal',
            online: serialReady,
            readOnly: false,
            actions: (
              <>
                <button type="button" onClick={() => void reclaimSerial()}>
                  重新接管串口
                </button>
                <button type="button" disabled={!serialReady} onClick={() => void pasteSerial()}>
                  粘贴
                </button>
                <button type="button" onClick={() => serialTerminalRef.current?.clear()}>
                  清屏
                </button>
              </>
            ),
            content: (
              <div className="terminal-workspace-board-channel">
                <TerminalSurface
                  key={`workspace-serial-${selected.id}-${terminalAppearance}`}
                  ref={serialTerminalRef}
                  initialData={runtime.serialData}
                  connected={serialReady}
                  placeholder="UART 在线后可直接输入；本窗格只绑定当前 Linux 设备"
                  variant="serial"
                  appearance={terminalAppearance}
                  onData={sendSerial}
                />
              </div>
            ),
          },
          {
            id: 'primary-ssh',
            eyebrow: 'PRIMARY / SSH',
            title: `${selected.name} · 主终端`,
            detail: workspace.projectName || '未绑定 SDK 项目',
            icon: 'code',
            online: workspace.primaryTerminalOnline,
            readOnly: false,
            content: workspace.primaryTerminal,
          },
          {
            id: 'device-status',
            eyebrow: 'FLASH / DEVICE STATUS',
            title: `${selected.name} · 烧录与设备状态`,
            detail: showFlashOutput
              ? runtime.flashState
              : flashDetected
                ? '烧录设备已连接 · 当前空闲'
                : flashBound
                  ? '未检测到烧录设备'
                  : '未绑定烧录接口',
            icon: 'task',
            online: serialReady || adbReady || flashDetected || workspace.primaryTerminalOnline,
            readOnly: true,
            content: (
              <div
                className="terminal-workspace-device-status"
                data-testid="linux-workspace-device-flash-status"
                aria-readonly="true"
              >
                <section className="terminal-workspace-link-status">
                  <header>
                    <span>设备连接</span>
                    <strong>{selected.boardUid || selected.name}</strong>
                  </header>
                  <div>
                    {[
                      ['UART', serialReady, runtime.serialPort || selected.serialPort || '未绑定'],
                      ['ADB', adbReady, selected.adbSerial || '未绑定'],
                      ['FLASH USB', flashDetected, flashBound ? selected.usbBinding : '未绑定'],
                      ['SSH', workspace.primaryTerminalOnline, workspace.hostLabel || '未配置'],
                    ].map(([label, online, detail]) => (
                      <span key={String(label)} className={online ? 'is-online' : ''}>
                        <i />
                        <small>{label}</small>
                        <strong>{online ? '在线' : '离线'}</strong>
                        <em title={String(detail)}>{detail}</em>
                      </span>
                    ))}
                  </div>
                  {workspace.overview ? (
                    <dl>
                      <div>
                        <dt>主机</dt>
                        <dd>{workspace.overview.hostname || '—'}</dd>
                      </div>
                      <div>
                        <dt>系统</dt>
                        <dd>{workspace.overview.osName || '—'}</dd>
                      </div>
                      <div>
                        <dt>架构</dt>
                        <dd>{workspace.overview.architecture || '—'}</dd>
                      </div>
                      <div>
                        <dt>运行时间</dt>
                        <dd>{formatCompactUptime(workspace.overview.uptimeSeconds)}</dd>
                      </div>
                    </dl>
                  ) : (
                    <p>连接 UART 或 ADB 并登录目标 Shell 后显示设备遥测。</p>
                  )}
                </section>

                <section className="terminal-workspace-flash-status">
                  <header>
                    <span>烧录状态</span>
                    <strong>{showFlashOutput ? runtime.flashState : '当前空闲'}</strong>
                  </header>
                  <div
                    className="terminal-workspace-flash-progress"
                    role="progressbar"
                    aria-valuemin={0}
                    aria-valuemax={100}
                    aria-valuenow={displayedFlashProgress}
                  >
                    <i style={{ width: `${Math.min(100, displayedFlashProgress)}%` }} />
                    <strong>{displayedFlashProgress.toFixed(0)}%</strong>
                  </div>
                  <dl>
                    <div>
                      <dt>阶段</dt>
                      <dd>{formatFlashPhase(runtime.flashPhase, runtime.flashTaskStatus)}</dd>
                    </div>
                    <div>
                      <dt>分区</dt>
                      <dd>
                        {runtime.flashPartition ||
                          (['failed', 'cancelled', 'success', 'completed'].includes(
                            runtime.flashTaskStatus
                          )
                            ? '未开始'
                            : '等待')}
                        {runtime.flashPartitionProgress != null
                          ? ` · ${runtime.flashPartitionProgress.toFixed(0)}%`
                          : ''}
                      </dd>
                    </div>
                    <div>
                      <dt>速率</dt>
                      <dd>{formatFlashRate(runtime.flashRate, runtime.flashTaskStatus)}</dd>
                    </div>
                    <div>
                      <dt>USB</dt>
                      <dd>
                        {formatUsbSpeed(runtime.flashUsbSpeed, runtime.flashUsbTheoreticalMbps)}
                      </dd>
                    </div>
                    <div>
                      <dt>校验</dt>
                      <dd>{formatVerifyState(runtime.flashVerify, runtime.flashTaskStatus)}</dd>
                    </div>
                    <div>
                      <dt>烧后动作</dt>
                      <dd>
                        {formatPostFlashState(
                          runtime.flashPostActionKind,
                          runtime.flashPostAction,
                          runtime.flashTaskStatus
                        )}
                      </dd>
                    </div>
                  </dl>
                </section>
              </div>
            ),
          },
          {
            id: 'task-log',
            eyebrow: 'DEVICE / TASK LOG',
            title: `${selected.name} · 实时日志`,
            detail: `${scopedTasks.length} 条设备任务`,
            icon: 'list',
            online: scopedTasks.some((task) => task.status === 'running'),
            readOnly: true,
            content: (
              <pre className="terminal-workspace-task-log" aria-readonly="true">
                {taskLog.length ? taskLog.join('\n') : '当前设备尚无任务日志。'}
              </pre>
            ),
          },
        ]}
      />
    );
  }

  return (
    <section className="workbench-card board-console">
      <div
        className={`board-console-grid is-${view} ${showFlashOutput ? 'is-flashing' : 'is-idle'}`}
      >
        {view === 'serial' && (
          <div className="board-console-panel board-console-serial">
            <header>
              <div className="board-terminal-heading">
                <i className={serialDetected ? 'is-online' : ''} />
                <div>
                  <small>SERIAL TERMINAL</small>
                  <strong>串口终端</strong>
                </div>
              </div>
              <div className="board-terminal-actions">
                <span className={serialReady ? 'is-online' : ''}>
                  {runtime.serialOpening
                    ? '连接中'
                    : serialReady
                      ? `${runtime.serialPort} · ${selected.baudRate}`
                      : serialDetected
                        ? `${selected.serialPort} · 已检测，终端未连接`
                        : '未检测到串口'}
                </span>
                {runtime.serialSessionId && (
                  <span className={runtime.serialCaptureError ? 'is-error' : 'is-online'}>
                    {runtime.serialCaptureError
                      ? `捕获异常 · ${runtime.serialCaptureError}`
                      : runtime.serialSynchronizedBytes >= runtime.serialCapturedBytes
                        ? `无缺口 · ${runtime.serialCapturedBytes} B`
                        : `补传中 · ${runtime.serialSynchronizedBytes}/${runtime.serialCapturedBytes} B`}
                    {runtime.serialEmitErrors ? ` · 事件失败 ${runtime.serialEmitErrors}` : ''}
                  </span>
                )}
                <button type="button" onClick={() => void reclaimSerial()}>
                  重新接管串口
                </button>
                <button type="button" disabled={!serialReady} onClick={() => void pasteSerial()}>
                  粘贴
                </button>
                <button type="button" onClick={() => serialTerminalRef.current?.clear()}>
                  清屏
                </button>
                <button
                  type="button"
                  disabled={!runtime.serialSessionId}
                  onClick={() => void exportSerialCapture()}
                >
                  导出完整日志
                </button>
              </div>
            </header>
            <div className="board-serial-identity" role="status">
              <span>
                保存节点：<code>{savedSerialPort || '未绑定'}</code>
              </span>
              <span>
                实际打开：<code>{runtime.serialConnected ? runtime.serialPort : '尚未打开'}</code>
              </span>
              <span>
                接收：{runtime.serialCapturedBytes.toLocaleString()} 字节 · {selected.baudRate} /
                8N1
              </span>
              {runtime.serialConnectionError ? (
                <strong>{runtime.serialConnectionError}</strong>
              ) : serialReady && runtime.serialCapturedBytes === 0 ? (
                <span>
                  端口已打开，尚未收到数据。请核对模块通道、RX/TX、共地和波特率；板卡处于 FEL
                  时也可能没有串口输出。
                </span>
              ) : null}
            </div>
            <div
              className={`board-terminal-input-state ${runtime.serialOpening ? 'is-reclaiming' : serialReady ? 'is-ready' : 'is-unavailable'}`}
              data-input-state={
                runtime.serialOpening ? 'reclaiming' : serialReady ? 'ready' : 'unavailable'
              }
              role="status"
            >
              <i />
              <strong>
                {runtime.serialOpening
                  ? '正在重新接管'
                  : serialReady
                    ? '用户可输入'
                    : '串口不可输入'}
              </strong>
              <span>
                {serialInputNotice ||
                  (serialReady
                    ? '点击终端输入；可使用 Ctrl+C、方向键、Tab 或“粘贴”。'
                    : '等待 LYNX 自动接管，或点击“重新接管串口”。')}
              </span>
            </div>
            <TerminalSurface
              key={`serial-v2-${selected.id}`}
              ref={serialTerminalRef}
              initialData={runtime.serialData}
              connected={serialReady}
              placeholder="点击终端后直接输入；支持方向键、Tab、Ctrl+C 与粘贴"
              variant="serial"
              onData={sendSerial}
            />
          </div>
        )}
        {view === 'flash' && (
          <FirmwareFlashPanel
            applicationCandidateId={applicationCandidateId}
            key={selected.id}
            board={selected}
            detected={flashDetected}
            protocol={flashProtocol}
            runtime={runtime}
            tasks={taskHistory}
            aiChannel={aiChannel}
            onOpenSettings={onOpenSettings}
            onTaskStarted={(taskId) => {
              flashTargetRef.current = selectedId;
              workbenchTasksRef.current[taskId] = selectedId;
            }}
          />
        )}
      </div>
    </section>
  );
};

export default BoardConsole;
