import FirmwareBuildPanel from './FirmwareBuildPanel';
import { forwardRef, useCallback, useEffect, useImperativeHandle, useRef, useState } from 'react';
import { FitAddon } from '@xterm/addon-fit';
import { Terminal } from '@xterm/xterm';
import '@xterm/xterm/css/xterm.css';
import { LynxIcon } from '../../CoreUI';
import { useTheme } from '../../Themes';
import {
  invokeCommand,
  normalizeIpcError,
  subscribeEvent,
  type WorkbenchDevice,
  type WorkbenchHost,
  type WorkbenchProject,
  type WorkbenchSshTerminalMode,
  type WorkbenchSshTerminalSnapshot,
  type WorkbenchTunnelStatus,
} from '../../Platform/IPC';
import {
  activateSshTerminalInputSurface,
  ownsSshTerminalInputSurface,
  releaseSshTerminalInputSurface,
  unseenSshTerminalOutput,
} from './sshTerminalStream';
import './SdkTerminal.css';

interface Props {
  isActive: boolean;
  board: WorkbenchDevice;
  project?: WorkbenchProject;
  host?: WorkbenchHost;
  tunnel?: WorkbenchTunnelStatus;
  onOpenSettings(): void;
  variant?: 'page' | 'workspace';
  sessionIndex?: number;
  workspaceRole?: 'primary' | 'secondary';
  onWorkspaceOnlineChange?(online: boolean): void;
  openRequestId?: number;
  onOpenRequestHandled?(requestId: number): void;
}

const SSH_SESSION_LIST_CHANGED_EVENT = 'lynx:ssh-terminal-list-changed';

interface TerminalHandle {
  write(data: Uint8Array): void;
  clear(): void;
  replace(data: Uint8Array): void;
}

const SshTerminalSurface = forwardRef<
  TerminalHandle,
  {
    snapshot: WorkbenchSshTerminalSnapshot;
    appearance?: 'dark' | 'light';
    onClaimInput(surfaceId: string): Promise<void>;
    onInput(data: string, surfaceId: string): void;
    onResize(cols: number, rows: number): void;
  }
>(({ snapshot, appearance = 'dark', onClaimInput, onInput, onResize }, ref) => {
  const hostRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const inputSurfaceToken = useRef<object>({}).current;
  const inputSurfaceId = useRef(`xterm:${globalThis.crypto.randomUUID()}`).current;
  const claimRef = useRef(onClaimInput);
  const inputRef = useRef(onInput);
  const resizeRef = useRef(onResize);
  inputRef.current = onInput;
  claimRef.current = onClaimInput;
  resizeRef.current = onResize;

  useImperativeHandle(ref, () => ({
    write(data) {
      terminalRef.current?.write(data);
    },
    clear() {
      terminalRef.current?.clear();
      terminalRef.current?.write('\x1b[2J\x1b[H');
    },
    replace(data) {
      terminalRef.current?.reset();
      if (data.length) terminalRef.current?.write(data);
    },
  }));

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    // xterm disposes its listeners, but its generated DOM can remain in the
    // host during React StrictMode's development remount. Starting from an
    // empty host guarantees one visible terminal and one input textarea.
    host.replaceChildren();
    const terminal = new Terminal({
      convertEol: false,
      cursorBlink: true,
      cursorStyle: 'bar',
      disableStdin: snapshot.state !== 'connected',
      fontFamily: '"JetBrains Mono", "Cascadia Code", Consolas, monospace',
      fontSize: 13,
      lineHeight: 1.25,
      scrollback: 15000,
      theme: {
        background: appearance === 'light' ? '#f8fbff' : '#080d17',
        foreground: appearance === 'light' ? '#24364d' : '#d6e2f2',
        cursor: appearance === 'light' ? '#087ff5' : '#66e3c4',
        selectionBackground: appearance === 'light' ? '#cfe6ff' : '#264c5f',
        black: appearance === 'light' ? '#1f2937' : '#111827',
        red: appearance === 'light' ? '#c62828' : '#fb7185',
        green: appearance === 'light' ? '#16813f' : '#65e6b4',
        yellow: appearance === 'light' ? '#9a6100' : '#f7c873',
        blue: appearance === 'light' ? '#006fde' : '#7cb7ff',
        magenta: appearance === 'light' ? '#7c3aed' : '#c4a7ff',
        cyan: appearance === 'light' ? '#087c8f' : '#64d8e6',
        white: appearance === 'light' ? '#52657c' : '#d8e3f0',
        brightBlack: appearance === 'light' ? '#7b8da3' : '#617086',
        brightRed: appearance === 'light' ? '#dc2626' : '#fda4af',
        brightGreen: appearance === 'light' ? '#0f9f4a' : '#9af2cf',
        brightYellow: appearance === 'light' ? '#b66b00' : '#fde2a7',
        brightBlue: appearance === 'light' ? '#087ff5' : '#a8ceff',
        brightMagenta: appearance === 'light' ? '#9333ea' : '#dbc9ff',
        brightCyan: appearance === 'light' ? '#0891b2' : '#a0eef5',
        brightWhite: appearance === 'light' ? '#0d1830' : '#ffffff',
      },
    });
    const fit = new FitAddon();
    terminal.loadAddon(fit);
    terminal.open(host);
    if (snapshot.data.length) terminal.write(new Uint8Array(snapshot.data));
    activateSshTerminalInputSurface(snapshot.terminalId, inputSurfaceToken);
    let disposed = false;
    let claimed = false;
    let claiming = false;
    const pendingInput: string[] = [];
    const claimInput = () => {
      if (disposed || claimed || claiming) return;
      claiming = true;
      void claimRef
        .current(inputSurfaceId)
        .then(() => {
          if (disposed) return;
          claimed = true;
          while (pendingInput.length) inputRef.current(pendingInput.shift()!, inputSurfaceId);
        })
        .catch(() => {
          pendingInput.length = 0;
        })
        .finally(() => {
          claiming = false;
        });
    };
    claimInput();
    const focusInput = () => {
      claimed = false;
      claimInput();
    };
    host.addEventListener('focusin', focusInput);
    const input = terminal.onData((data) => {
      if (ownsSshTerminalInputSurface(snapshot.terminalId, inputSurfaceToken)) {
        if (claimed) inputRef.current(data, inputSurfaceId);
        else {
          pendingInput.push(data);
          claimInput();
        }
      }
    });
    let frame = 0;
    let lastSize = '';
    const scheduleFit = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        if (!hostRef.current?.clientWidth || !hostRef.current?.clientHeight) return;
        fit.fit();
        const size = `${terminal.cols}x${terminal.rows}`;
        if (size !== lastSize) {
          lastSize = size;
          resizeRef.current(terminal.cols, terminal.rows);
        }
      });
    };
    const observer = new ResizeObserver(scheduleFit);
    observer.observe(host);
    window.addEventListener('resize', scheduleFit);
    terminalRef.current = terminal;
    scheduleFit();
    return () => {
      disposed = true;
      cancelAnimationFrame(frame);
      observer.disconnect();
      window.removeEventListener('resize', scheduleFit);
      input.dispose();
      host.removeEventListener('focusin', focusInput);
      releaseSshTerminalInputSurface(snapshot.terminalId, inputSurfaceToken);
      terminal.dispose();
      host.replaceChildren();
      terminalRef.current = null;
    };
  }, [appearance, inputSurfaceId, inputSurfaceToken, snapshot.terminalId]);

  useEffect(() => {
    if (terminalRef.current) {
      terminalRef.current.options.disableStdin = snapshot.state !== 'connected';
    }
  }, [snapshot.state]);

  return <div className="sdk-terminal-xterm" ref={hostRef} />;
});
SshTerminalSurface.displayName = 'SshTerminalSurface';

function errorText(value: unknown) {
  const error = normalizeIpcError(value);
  return `${error.machineCode || error.name}：${error.message}`;
}

function durationText(startedAtMs: number) {
  const seconds = Math.max(0, Math.floor((Date.now() - startedAtMs) / 1000));
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const rest = seconds % 60;
  return [hours, minutes, rest].map((value) => String(value).padStart(2, '0')).join(':');
}

function byteText(value: number) {
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KiB`;
  return `${(value / 1024 / 1024).toFixed(1)} MiB`;
}

function stateText(session?: WorkbenchSshTerminalSnapshot) {
  if (!session) return '未启动';
  if (session.state === 'connecting') return '正在连接';
  if (session.state === 'connected') return session.mode === 'shell' ? '终端在线' : 'AI 正在运行';
  if (session.state === 'failed') return '会话失败';
  return session.exitCode == null ? '会话已结束' : `已退出 (${session.exitCode})`;
}

export default function SdkTerminal({
  isActive,
  board,
  project,
  host,
  tunnel,
  onOpenSettings,
  variant = 'page',
  sessionIndex = 0,
  workspaceRole = 'primary',
  onWorkspaceOnlineChange,
  openRequestId = 0,
  onOpenRequestHandled,
}: Props) {
  const { isDark } = useTheme();
  const terminalAppearance = isDark ? 'dark' : 'light';
  const [sessions, setSessions] = useState<WorkbenchSshTerminalSnapshot[]>([]);
  const [activeId, setActiveId] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [, setClock] = useState(0);
  const surfaceRef = useRef<TerminalHandle>(null);
  const outputCursorRef = useRef(new Map<string, { outputEpoch: number; receivedBytes: number }>());
  const activeIdRef = useRef(activeId);
  const [firmwareBuildView, setFirmwareBuildView] = useState(false);
  const handledOpenRequestRef = useRef(0);
  activeIdRef.current = activeId;
  const active =
    variant === 'workspace'
      ? sessions[sessionIndex]
      : (sessions.find((session) => session.terminalId === activeId) ?? sessions[0]);
  const scoped = !!project?.id && !!board.id && project.deviceId === board.id;

  const loadSessions = useCallback(() => {
    if (!isActive || !project?.id || !board.id) return;
    let cancelled = false;
    void invokeCommand('workbench_ssh_terminal_list', {
      projectId: project.id,
      deviceId: board.id,
    })
      .then((items) => {
        if (cancelled) return;
        for (const item of items) {
          const cursor = outputCursorRef.current.get(item.terminalId);
          if (
            !cursor ||
            item.outputEpoch > cursor.outputEpoch ||
            (item.outputEpoch === cursor.outputEpoch && item.receivedBytes > cursor.receivedBytes)
          ) {
            outputCursorRef.current.set(item.terminalId, {
              outputEpoch: item.outputEpoch,
              receivedBytes: item.receivedBytes,
            });
          }
        }
        setSessions(items);
        setActiveId((current) =>
          variant === 'workspace'
            ? (items[sessionIndex]?.terminalId ?? '')
            : items.some((item) => item.terminalId === current)
              ? current
              : (items[items.length - 1]?.terminalId ?? '')
        );
      })
      .catch((value) => !cancelled && setError(errorText(value)));
    return () => {
      cancelled = true;
    };
  }, [board.id, isActive, project?.id, sessionIndex, variant]);

  useEffect(() => loadSessions(), [loadSessions]);

  useEffect(() => {
    const refresh = () => loadSessions();
    window.addEventListener(SSH_SESSION_LIST_CHANGED_EVENT, refresh);
    return () => window.removeEventListener(SSH_SESSION_LIST_CHANGED_EVENT, refresh);
  }, [loadSessions]);

  useEffect(() => {
    activeIdRef.current = active?.terminalId ?? '';
  }, [active?.terminalId]);

  useEffect(() => {
    if (variant !== 'workspace' || !onWorkspaceOnlineChange) return;
    onWorkspaceOnlineChange(active?.state === 'connected');
    return () => onWorkspaceOnlineChange(false);
  }, [active?.state, onWorkspaceOnlineChange, variant]);

  useEffect(() => {
    if (!project?.id || !board.id) return;
    let disposed = false;
    const unsubscribers: Array<() => void> = [];
    void subscribeEvent('workbench-ssh-terminal-data', (event) => {
      if (event.projectId !== project.id || event.deviceId !== board.id || disposed) return;
      const cursor = outputCursorRef.current.get(event.terminalId);
      if (cursor && event.outputEpoch < cursor.outputEpoch) return;
      const reset = !cursor || event.outputEpoch > cursor.outputEpoch;
      const unseen = reset
        ? event.data
        : unseenSshTerminalOutput(cursor.receivedBytes, event.receivedBytes, event.data);
      outputCursorRef.current.set(event.terminalId, {
        outputEpoch: event.outputEpoch,
        receivedBytes: Math.max(cursor?.receivedBytes ?? 0, event.receivedBytes),
      });
      if (event.terminalId === activeIdRef.current) {
        if (cursor && event.outputEpoch > cursor.outputEpoch) {
          surfaceRef.current?.replace(new Uint8Array(unseen));
        } else if (unseen.length) {
          surfaceRef.current?.write(new Uint8Array(unseen));
        }
      }
      setSessions((current) =>
        current.map((item) =>
          item.terminalId !== event.terminalId || event.outputEpoch < item.outputEpoch
            ? item
            : (() => {
                const resetsItem = event.outputEpoch > item.outputEpoch;
                const unseenForItem = resetsItem
                  ? event.data
                  : unseenSshTerminalOutput(item.receivedBytes, event.receivedBytes, event.data);
                return {
                  ...item,
                  outputEpoch: event.outputEpoch,
                  receivedBytes: Math.max(item.receivedBytes, event.receivedBytes),
                  data: (resetsItem ? unseenForItem : [...item.data, ...unseenForItem]).slice(
                    -1024 * 1024
                  ),
                };
              })()
        )
      );
    }).then((unlisten) => (disposed ? unlisten() : unsubscribers.push(unlisten)));
    void subscribeEvent('workbench-ssh-terminal-status', (event) => {
      if (event.projectId !== project.id || event.deviceId !== board.id || disposed) return;
      setSessions((current) =>
        current.map((item) =>
          item.terminalId === event.terminalId
            ? {
                ...item,
                state: event.state,
                message: event.message,
                endedAtMs: event.endedAtMs,
                exitCode: event.exitCode,
                sentBytes: event.sentBytes,
                receivedBytes: event.receivedBytes,
              }
            : item
        )
      );
    }).then((unlisten) => (disposed ? unlisten() : unsubscribers.push(unlisten)));
    return () => {
      disposed = true;
      unsubscribers.forEach((unlisten) => unlisten());
    };
  }, [project?.id, board.id]);

  useEffect(() => {
    if (!active) return;
    const timer = window.setInterval(() => setClock((value) => value + 1), 1000);
    return () => window.clearInterval(timer);
  }, [active?.terminalId]);

  const openSession = useCallback(
    async (mode: WorkbenchSshTerminalMode) => {
      if (!project?.id || !board.id || !scoped) {
        setError('请先在设备设置中为当前单板绑定 SDK 项目和 SSH 主机。');
        return;
      }
      setBusy(true);
      setError('');
      try {
        const snapshot = await invokeCommand('workbench_ssh_terminal_open', {
          projectId: project.id,
          deviceId: board.id,
          mode,
          cols: 120,
          rows: 32,
        });
        outputCursorRef.current.set(snapshot.terminalId, {
          outputEpoch: snapshot.outputEpoch,
          receivedBytes: snapshot.receivedBytes,
        });
        setSessions((current) => [...current, snapshot]);
        setActiveId(snapshot.terminalId);
        window.dispatchEvent(new Event(SSH_SESSION_LIST_CHANGED_EVENT));
      } catch (value) {
        setError(errorText(value));
      } finally {
        setBusy(false);
      }
    },
    [board.id, project?.id, scoped]
  );

  useEffect(() => {
    if (!openRequestId) {
      handledOpenRequestRef.current = 0;
      return;
    }
    if (variant === 'workspace' || !isActive || openRequestId <= handledOpenRequestRef.current) {
      return;
    }
    handledOpenRequestRef.current = openRequestId;
    onOpenRequestHandled?.(openRequestId);
    void openSession('shell');
  }, [isActive, onOpenRequestHandled, openRequestId, openSession, variant]);

  async function closeSession(session = active) {
    if (!session || !project?.id || !board.id) return;
    setBusy(true);
    try {
      await invokeCommand('workbench_ssh_terminal_close', {
        terminalId: session.terminalId,
        projectId: project.id,
        deviceId: board.id,
      });
      const next = sessions.filter((item) => item.terminalId !== session.terminalId);
      outputCursorRef.current.delete(session.terminalId);
      setSessions(next);
      setActiveId(next[next.length - 1]?.terminalId ?? '');
      window.dispatchEvent(new Event(SSH_SESSION_LIST_CHANGED_EVENT));
    } catch (value) {
      setError(errorText(value));
    } finally {
      setBusy(false);
    }
  }

  async function reconnect() {
    if (!active) return;
    const mode = active.mode;
    if (!project?.id || !board.id) return;
    setBusy(true);
    setError('');
    try {
      await invokeCommand('workbench_ssh_terminal_close', {
        terminalId: active.terminalId,
        projectId: project.id,
        deviceId: board.id,
      });
      setSessions((current) => current.filter((item) => item.terminalId !== active.terminalId));
      const snapshot = await invokeCommand('workbench_ssh_terminal_open', {
        projectId: project.id,
        deviceId: board.id,
        mode,
        cols: 120,
        rows: 32,
      });
      outputCursorRef.current.delete(active.terminalId);
      outputCursorRef.current.set(snapshot.terminalId, {
        outputEpoch: snapshot.outputEpoch,
        receivedBytes: snapshot.receivedBytes,
      });
      setSessions((current) => [...current, snapshot]);
      setActiveId(snapshot.terminalId);
      window.dispatchEvent(new Event(SSH_SESSION_LIST_CHANGED_EVENT));
    } catch (value) {
      setError(errorText(value));
    } finally {
      setBusy(false);
    }
  }

  async function clearTerminal() {
    if (!active || !project?.id || !board.id) return;
    try {
      const cleared = await invokeCommand('workbench_ssh_terminal_clear', {
        terminalId: active.terminalId,
        projectId: project.id,
        deviceId: board.id,
      });
      const retained =
        active.outputEpoch === cleared.outputEpoch && active.receivedBytes > cleared.receivedBytes
          ? active.data
          : [];
      outputCursorRef.current.set(active.terminalId, {
        outputEpoch: cleared.outputEpoch,
        receivedBytes: Math.max(active.receivedBytes, cleared.receivedBytes),
      });
      surfaceRef.current?.replace(new Uint8Array(retained));
      setSessions((current) =>
        current.map((item) =>
          item.terminalId !== active.terminalId || item.outputEpoch > cleared.outputEpoch
            ? item
            : (() => {
                const retainedForItem =
                  item.outputEpoch === cleared.outputEpoch &&
                  item.receivedBytes > cleared.receivedBytes
                    ? item.data
                    : [];
                return { ...item, outputEpoch: cleared.outputEpoch, data: retainedForItem };
              })()
        )
      );
    } catch (value) {
      setError(errorText(value));
    }
  }

  async function copyTerminal() {
    if (!active) return;
    try {
      const text = new TextDecoder().decode(new Uint8Array(active.data));
      await navigator.clipboard.writeText(text);
    } catch (value) {
      setError(`复制终端输出失败：${normalizeIpcError(value).message}`);
    }
  }

  async function copySkillsPrompt() {
    try {
      await navigator.clipboard.writeText(
        '先调用 lynx_skills_search 检索当前板卡技能，再调用 lynx_skills_get 获取元数据，最后调用 lynx_skills_read 读取 SKILL.md；Skills 只提供指导，实际动作必须使用当前 MCP 通道已有工具并遵守硬件门禁。'
      );
    } catch (value) {
      setError(`复制 Skills 调用提示失败：${normalizeIpcError(value).message}`);
    }
  }

  async function syncNativeSkills() {
    if (!project?.id) return;
    if (
      !window.confirm(
        '将 LYNX 管理的只读 Skills 同步到当前 SDK 的 .agents/skills 和 .claude/skills。不会覆盖用户文件，是否继续？'
      )
    ) {
      return;
    }
    setBusy(true);
    setError('');
    try {
      const results = await invokeCommand('workbench_sync_native_skills', {
        projectId: project.id,
        providers: ['codex', 'claude'],
      });
      const conflicts = results.flatMap((result) => result.conflicts);
      setError(
        conflicts.length
          ? `Skills 已同步，以下用户文件未覆盖：${conflicts.join('、')}`
          : 'Skills 已同步到 Codex 与 Claude 项目目录。'
      );
    } catch (value) {
      setError(`同步 Skills 失败：${normalizeIpcError(value).message}`);
    } finally {
      setBusy(false);
    }
  }

  async function claimInput(surfaceId: string) {
    if (!active || !project?.id || !board.id) throw new Error('SSH_TERMINAL_NOT_READY');
    try {
      await invokeCommand('workbench_ssh_terminal_claim_input', {
        terminalId: active.terminalId,
        projectId: project.id,
        deviceId: board.id,
        surfaceId,
      });
    } catch (value) {
      setError(errorText(value));
      throw value;
    }
  }

  function writeInput(data: string, surfaceId: string) {
    if (!active || !project?.id || !board.id || active.state !== 'connected') return;
    void invokeCommand('workbench_ssh_terminal_write', {
      terminalId: active.terminalId,
      projectId: project.id,
      deviceId: board.id,
      surfaceId,
      data: Array.from(new TextEncoder().encode(data)),
    }).catch((value) => setError(errorText(value)));
  }

  function resize(cols: number, rows: number) {
    if (!active || !project?.id || !board.id) return;
    void invokeCommand('workbench_ssh_terminal_resize', {
      terminalId: active.terminalId,
      projectId: project.id,
      deviceId: board.id,
      cols,
      rows,
    }).catch(() => undefined);
  }

  if (variant === 'workspace') {
    return (
      <section
        className={`sdk-terminal-page is-workspace is-${workspaceRole}`}
        data-testid={`linux-sdk-terminal-workspace-${sessionIndex}`}
      >
        <div className="sdk-terminal-workspace-actions">
          <button disabled={busy || !scoped} onClick={() => void openSession('shell')}>
            <LynxIcon name="add" size={16} /> 新建 Shell
          </button>
          <button disabled={!active} onClick={() => void clearTerminal()}>
            <LynxIcon name="broom" size={16} /> 清屏
          </button>
          <button disabled={busy || !active} onClick={() => void closeSession()}>
            <LynxIcon name="delete" size={16} /> 关闭会话
          </button>
          {workspaceRole === 'primary' && (
            <>
              <button disabled={busy || !scoped} onClick={() => void openSession('codex')}>
                <LynxIcon name="code" size={16} /> Codex
              </button>
              <button disabled={busy || !scoped} onClick={() => void openSession('claude_code')}>
                <LynxIcon name="agent" size={16} /> Claude
              </button>
            </>
          )}
          <span>
            {active
              ? `${active.title} · ${stateText(active)}`
              : `终端槽位 ${sessionIndex + 1} 未启动`}
          </span>
        </div>
        <div className="sdk-terminal-workspace-stage">
          {active ? (
            <SshTerminalSurface
              key={`${active.terminalId}:exclusive-input-v1`}
              ref={surfaceRef}
              snapshot={active}
              appearance={terminalAppearance}
              onClaimInput={claimInput}
              onInput={writeInput}
              onResize={resize}
            />
          ) : (
            <div className="sdk-terminal-empty">
              <LynxIcon name="terminal" size={40} />
              <strong>{scoped ? '新建当前设备的 SSH 会话' : '尚未绑定 SDK 项目与 SSH 主机'}</strong>
              <span>{scoped ? '会话只属于当前 Linux 设备。' : '请退出全屏并打开设备设置。'}</span>
            </div>
          )}
        </div>
        {error && <p className="sdk-terminal-error">{error}</p>}
      </section>
    );
  }

  return (
    <>
      <nav className="fw-page-tabs">
        <button aria-pressed={!firmwareBuildView} onClick={() => setFirmwareBuildView(false)}>
          SSH 终端
        </button>
        <button aria-pressed={firmwareBuildView} onClick={() => setFirmwareBuildView(true)}>
          构建任务
        </button>
      </nav>
      {firmwareBuildView ? (
        <FirmwareBuildPanel key={board.boardUid} board={board} project={project} host={host} />
      ) : (
        <section className="sdk-terminal-page" data-testid="linux-sdk-terminal">
          <header className="sdk-terminal-hero">
            <div className="sdk-terminal-identity">
              <span className="sdk-terminal-hero-icon">
                <LynxIcon name="terminal" filled size={20} />
              </span>
              <div>
                <small>REMOTE DEVELOPMENT</small>
                <h3>SSH 终端</h3>
                <p>项目范围内的交互式远程开发会话</p>
              </div>
            </div>
            <div className="sdk-terminal-target">
              <dl>
                <div>
                  <dt>项目</dt>
                  <dd>{project?.name || '未绑定'}</dd>
                </div>
                <div>
                  <dt>主机</dt>
                  <dd>{host ? `${host.username}@${host.address}` : '未配置'}</dd>
                </div>
                <div>
                  <dt>SDK 目录</dt>
                  <dd title={active?.workingDirectory || project?.sdkDir}>
                    {active?.workingDirectory || project?.sdkDir || '未配置'}
                  </dd>
                </div>
              </dl>
              <div className="sdk-terminal-signals">
                <span className={active?.state === 'connected' ? 'is-online' : ''}>
                  <i /> SSH {stateText(active)}
                </span>
                <span className={tunnel?.state === 'connected' ? 'is-online' : ''}>
                  <i /> MCP {tunnel?.state === 'connected' ? '已连接' : '未连接'}
                </span>
              </div>
            </div>
            <button className="sdk-terminal-settings" onClick={onOpenSettings} title="设备设置">
              <LynxIcon name="settings" size={20} />
            </button>
          </header>

          <div className="sdk-terminal-actions">
            <div className="sdk-terminal-action-group" aria-label="终端操作">
              <button disabled={busy || !scoped} onClick={() => void openSession('shell')}>
                <LynxIcon name="add" size={16} /> 新建终端
              </button>
              <button disabled={busy || !active} onClick={() => void reconnect()}>
                <LynxIcon name="refresh" size={16} /> 重连
              </button>
              <button disabled={busy || !active} onClick={() => void closeSession()}>
                <LynxIcon name="delete" size={16} /> 关闭
              </button>
              <button disabled={!active} onClick={() => void clearTerminal()}>
                <LynxIcon name="broom" size={16} /> 清屏
              </button>
              <button disabled={!active?.data.length} onClick={() => void copyTerminal()}>
                <LynxIcon name="copy" size={16} /> 复制输出
              </button>
            </div>
            <div className="sdk-terminal-action-group is-agent-group" aria-label="AI 会话">
              <button
                className="is-agent"
                title="启动 Codex"
                disabled={busy || !scoped}
                onClick={() => void openSession('codex')}
              >
                <LynxIcon name="code" size={16} /> Codex
              </button>
              <button
                className="is-agent"
                title="启动 Claude"
                disabled={busy || !scoped}
                onClick={() => void openSession('claude_code')}
              >
                <LynxIcon name="agent" size={16} /> Claude
              </button>
              <button
                className="is-agent"
                title="启动 DeepSeek Harness"
                disabled={busy || !scoped}
                onClick={() => void openSession('deepseek_harness')}
              >
                <LynxIcon name="agent" size={16} /> DeepSeek
              </button>
            </div>
          </div>

          <div className="sdk-terminal-visibility" data-testid="sdk-terminal-visibility-boundary">
            <LynxIcon name="info" size={20} />
            <div>
              <strong>Lynx 托管会话</strong>
              <span>
                从本页启动的 Shell、Codex、Claude 和 DeepSeek Harness 均使用同一套可见 SSH
                PTY，输入与输出会实时显示并保留；在其他终端单独启动的进程不会被镜像到这里。
              </span>
            </div>
            <em className={active?.state === 'connected' ? 'is-live' : ''}>
              {active?.state === 'connected' ? 'LIVE' : 'IDLE'}
            </em>
          </div>

          <div className="sdk-terminal-mcp-scope" data-testid="sdk-terminal-mcp-scope">
            <LynxIcon name="link" size={20} />
            <div>
              <strong>MCP 自动选择：{project?.mcpServerName || '尚未配置'}</strong>
              <span>
                当前终端固定到 {project?.name || '当前单板 SDK'} · {board.name}；AI 使用 Skills
                时依次调用
                <code>lynx_skills_search → lynx_skills_get → lynx_skills_read</code>。 Skills
                只提供指导，实际动作仍经过当前 Linux MCP 的作用域和硬件门禁。
              </span>
            </div>
            <button type="button" onClick={() => void copySkillsPrompt()}>
              复制调用提示
            </button>
            <button
              type="button"
              disabled={busy || !project?.id}
              onClick={() => void syncNativeSkills()}
            >
              同步到 AI 工具
            </button>
          </div>

          <div className="sdk-terminal-shell">
            <div className="sdk-terminal-tabs">
              {sessions.map((session) => (
                <button
                  key={session.terminalId}
                  className={session.terminalId === active?.terminalId ? 'is-active' : ''}
                  onClick={() => setActiveId(session.terminalId)}
                >
                  <i className={session.state === 'connected' ? 'is-online' : ''} />
                  {session.title}
                  <small>
                    {session.mode === 'shell'
                      ? 'SSH'
                      : session.mode === 'codex'
                        ? 'CODEX'
                        : 'CLAUDE'}
                  </small>
                </button>
              ))}
              {!sessions.length && <span>尚未创建远程终端</span>}
            </div>
            <div className="sdk-terminal-stage">
              {active ? (
                <SshTerminalSurface
                  key={`${active.terminalId}:exclusive-input-v1`}
                  ref={surfaceRef}
                  snapshot={active}
                  appearance={terminalAppearance}
                  onClaimInput={claimInput}
                  onInput={writeInput}
                  onResize={resize}
                />
              ) : (
                <div className="sdk-terminal-empty">
                  <LynxIcon name="terminal" size={40} />
                  <strong>打开项目终端</strong>
                  <span>点击“新建终端”，会话将在已绑定 SDK 目录中启动。</span>
                </div>
              )}
            </div>
          </div>

          {error && <p className="sdk-terminal-error">{error}</p>}
          {active && active.state !== 'connected' && (
            <p className={`sdk-terminal-session-state is-${active.state}`}>
              {active.message}
              {active.exitCode != null ? ` · 退出码 ${active.exitCode}` : ''}
            </p>
          )}
          <footer className="sdk-terminal-footer">
            <span>
              <small>SESSION</small>
              <strong>{active ? durationText(active.startedAtMs) : '00:00:00'}</strong>
            </span>
            <span>
              <small>SESSION I/O</small>
              <strong className={active?.state === 'connected' ? 'is-online' : ''}>
                {active
                  ? `↑ ${byteText(active.sentBytes)}  ↓ ${byteText(active.receivedBytes)}`
                  : '—'}
              </strong>
            </span>
            <p>
              {active
                ? `${active.title} · ${stateText(active)} · ${active.message}`
                : `MCP ${project?.mcpServerName || '—'} · 尚未启动托管终端`}
            </p>
          </footer>
        </section>
      )}
    </>
  );
}
