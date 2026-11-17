import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { LynxIcon, type LynxIconName } from '../../CoreUI';
import { normalizeIpcError, type WorkbenchDevice } from '../../Platform/IPC';
import {
  DEFAULT_TERMINAL_WORKSPACE_ORDER,
  selectTerminalWorkspaceProfile,
} from './terminalWorkspaceLayout';
import './LinuxTerminalWorkspace.css';

export type LinuxTerminalWorkspacePaneId = (typeof DEFAULT_TERMINAL_WORKSPACE_ORDER)[number];

export interface LinuxTerminalWorkspacePane {
  id: LinuxTerminalWorkspacePaneId;
  eyebrow: string;
  title: string;
  detail: string;
  icon: LynxIconName;
  online: boolean;
  readOnly: boolean;
  actions?: React.ReactNode;
  content: React.ReactNode;
}

interface LinuxTerminalWorkspaceProps {
  device: WorkbenchDevice;
  panes: LinuxTerminalWorkspacePane[];
  projectName: string;
  hostLabel: string;
  onRequestExit(): void;
}

function validStoredOrder(value: unknown): value is LinuxTerminalWorkspacePaneId[] {
  return (
    Array.isArray(value) &&
    value.length === DEFAULT_TERMINAL_WORKSPACE_ORDER.length &&
    DEFAULT_TERMINAL_WORKSPACE_ORDER.every((id) => value.includes(id))
  );
}

export function LinuxTerminalWorkspace({
  device,
  panes,
  projectName,
  hostLabel,
  onRequestExit,
}: LinuxTerminalWorkspaceProps) {
  const shellRef = useRef<HTMLDivElement>(null);
  const draggedPaneRef = useRef<LinuxTerminalWorkspacePaneId | null>(null);
  const [viewport, setViewport] = useState(() => ({
    width: window.innerWidth,
    height: window.innerHeight,
  }));
  const [activePaneId, setActivePaneId] = useState<LinuxTerminalWorkspacePaneId>('primary-ssh');
  const [maximizedPaneId, setMaximizedPaneId] = useState<LinuxTerminalWorkspacePaneId | null>(null);
  const [layoutLocked, setLayoutLocked] = useState(false);
  const [syncInputEnabled, setSyncInputEnabled] = useState(false);
  const [fullscreenError, setFullscreenError] = useState('');
  const [paneOrder, setPaneOrder] = useState<LinuxTerminalWorkspacePaneId[]>(() => {
    try {
      const stored = JSON.parse(
        window.localStorage.getItem(`lynx.terminalWorkspace.${device.id}.order`) ?? 'null'
      );
      if (validStoredOrder(stored)) return stored;
    } catch {
      // Invalid local presentation data falls back to the deterministic profile.
    }
    return [...DEFAULT_TERMINAL_WORKSPACE_ORDER];
  });

  const profile = useMemo(
    () => selectTerminalWorkspaceProfile(viewport.width, viewport.height, panes.length),
    [panes.length, viewport.height, viewport.width]
  );
  const paneById = useMemo(() => new Map(panes.map((pane) => [pane.id, pane])), [panes]);
  const orderedPanes = paneOrder
    .map((id) => paneById.get(id))
    .filter((pane): pane is LinuxTerminalWorkspacePane => Boolean(pane));

  const exitWorkspace = useCallback(() => {
    setSyncInputEnabled(false);
    onRequestExit();
  }, [onRequestExit]);

  useEffect(() => {
    const appWindow = getCurrentWindow();
    let disposed = false;
    void appWindow.setFullscreen(true).catch((error) => {
      if (!disposed) {
        const detail = normalizeIpcError(error);
        setFullscreenError(
          `无法进入系统全屏：${detail.machineCode || detail.name}：${detail.message}`
        );
      }
    });
    return () => {
      disposed = true;
      void appWindow.setFullscreen(false).catch(() => undefined);
    };
  }, []);

  useEffect(() => {
    const shell = shellRef.current;
    if (!shell) return;
    let frame = 0;
    const update = () => {
      window.cancelAnimationFrame(frame);
      frame = window.requestAnimationFrame(() => {
        const bounds = shell.getBoundingClientRect();
        setViewport((current) => {
          const next = { width: Math.round(bounds.width), height: Math.round(bounds.height) };
          return current.width === next.width && current.height === next.height ? current : next;
        });
      });
    };
    const observer = new ResizeObserver(update);
    observer.observe(shell);
    update();
    return () => {
      window.cancelAnimationFrame(frame);
      observer.disconnect();
    };
  }, []);

  useEffect(() => {
    window.localStorage.setItem(
      `lynx.terminalWorkspace.${device.id}.order`,
      JSON.stringify(paneOrder)
    );
  }, [device.id, paneOrder]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        const target = event.target instanceof Element ? event.target : null;
        if (target?.closest('.xterm')) return;
        event.preventDefault();
        if (maximizedPaneId) setMaximizedPaneId(null);
        else exitWorkspace();
        return;
      }
      if (!event.altKey || !['h', 'j', 'k', 'l'].includes(event.key.toLowerCase())) return;
      event.preventDefault();
      event.stopPropagation();
      const visible = orderedPanes.slice(0, profile.visiblePaneCount);
      const currentIndex = Math.max(
        0,
        visible.findIndex((pane) => pane.id === activePaneId)
      );
      const backwards = ['h', 'k'].includes(event.key.toLowerCase());
      const nextIndex = (currentIndex + (backwards ? -1 : 1) + visible.length) % visible.length;
      setActivePaneId(visible[nextIndex].id);
    };
    window.addEventListener('keydown', onKeyDown, true);
    return () => window.removeEventListener('keydown', onKeyDown, true);
  }, [activePaneId, exitWorkspace, maximizedPaneId, orderedPanes, profile.visiblePaneCount]);

  const resetLayout = () => {
    setPaneOrder([...DEFAULT_TERMINAL_WORKSPACE_ORDER]);
    setMaximizedPaneId(null);
    setActivePaneId('primary-ssh');
  };

  const swapPanes = (
    source: LinuxTerminalWorkspacePaneId,
    target: LinuxTerminalWorkspacePaneId
  ) => {
    if (layoutLocked || source === target) return;
    setPaneOrder((current) => {
      const sourceIndex = current.indexOf(source);
      const targetIndex = current.indexOf(target);
      if (sourceIndex < 0 || targetIndex < 0) return current;
      const next = [...current];
      next[sourceIndex] = target;
      next[targetIndex] = source;
      return next;
    });
  };

  const gridStyle = {
    '--terminal-workspace-areas': profile.areas.map((area) => `"${area}"`).join(' '),
    '--terminal-workspace-columns': profile.columns,
    '--terminal-workspace-rows': profile.rows,
  } as React.CSSProperties;

  return (
    <div
      ref={shellRef}
      className="linux-terminal-workspace"
      data-testid="linux-terminal-workspace"
      data-device-id={device.id}
      data-layout-profile={profile.id}
    >
      <header className="terminal-workspace-toolbar">
        <div className="terminal-workspace-brand">
          <LynxIcon name="terminal" filled size={20} />
          <span>
            <strong>LYNX 多终端工作区</strong>
            <small>{device.name} · 单 Linux 设备</small>
          </span>
        </div>
        <div className="terminal-workspace-shortcuts" aria-label="工作区快捷键">
          <span>Alt H/J/K/L 切换</span>
          <span>双击标题最大化</span>
          <span>空白处 Esc 退出</span>
        </div>
        <div className="terminal-workspace-actions">
          <button
            type="button"
            className={layoutLocked ? 'is-active' : ''}
            aria-pressed={layoutLocked}
            onClick={() => setLayoutLocked((current) => !current)}
          >
            <LynxIcon name={layoutLocked ? 'lock' : 'flow'} size={16} />
            {layoutLocked ? '布局已锁定' : '自动编排'}
          </button>
          <button type="button" onClick={resetLayout}>
            <LynxIcon name="refresh" size={16} /> 恢复布局
          </button>
          <button
            type="button"
            disabled
            aria-pressed={syncInputEnabled}
            title="同步输入将在共享终端控制器完成后启用；当前保持关闭"
          >
            <LynxIcon name="link" size={16} /> 同步输入：关闭
          </button>
          <button
            type="button"
            className="is-exit"
            data-testid="linux-exit-terminal-workspace"
            onClick={exitWorkspace}
          >
            <LynxIcon name="fullscreen" size={16} /> 退出全屏
          </button>
        </div>
      </header>

      <aside className="terminal-workspace-rail" aria-label="终端窗格">
        {orderedPanes.map((pane, index) => (
          <button
            key={pane.id}
            type="button"
            className={`${pane.id === activePaneId ? 'is-active' : ''} ${index >= profile.visiblePaneCount ? 'is-waiting' : ''}`}
            title={`${pane.title}${index >= profile.visiblePaneCount ? '（等待区）' : ''}`}
            onClick={() => {
              if (index >= profile.visiblePaneCount) {
                swapPanes(pane.id, orderedPanes[profile.visiblePaneCount - 1].id);
              }
              setActivePaneId(pane.id);
            }}
          >
            <LynxIcon name={pane.icon} filled={pane.id === activePaneId} size={20} />
            <i className={pane.online ? 'is-online' : ''} />
          </button>
        ))}
      </aside>

      <main
        className={`terminal-workspace-grid ${maximizedPaneId ? 'is-maximized' : ''}`}
        style={gridStyle}
      >
        {orderedPanes.slice(0, profile.visiblePaneCount).map((pane, index) => {
          const maximized = maximizedPaneId === pane.id;
          return (
            <article
              key={pane.id}
              className={`terminal-workspace-pane ${activePaneId === pane.id ? 'is-active' : ''} ${maximized ? 'is-pane-maximized' : ''}`}
              style={{ gridArea: profile.slots[index] }}
              data-workspace-slot={profile.slots[index]}
              onDragOver={(event) => event.preventDefault()}
              onDrop={() => {
                if (draggedPaneRef.current) swapPanes(draggedPaneRef.current, pane.id);
                draggedPaneRef.current = null;
              }}
              onMouseDown={() => setActivePaneId(pane.id)}
            >
              <header
                className="terminal-workspace-pane-header"
                draggable={!layoutLocked}
                onDragStart={() => {
                  draggedPaneRef.current = pane.id;
                }}
                onDoubleClick={() => setMaximizedPaneId(maximized ? null : pane.id)}
              >
                <div>
                  <i className={pane.online ? 'is-online' : ''} />
                  <span>
                    <small>{pane.eyebrow}</small>
                    <strong>{pane.title}</strong>
                  </span>
                </div>
                <span className="terminal-workspace-pane-detail">{pane.detail}</span>
                <span className="terminal-workspace-pane-actions">{pane.actions}</span>
                <span className="terminal-workspace-pane-mode">
                  {pane.readOnly ? 'READ ONLY' : pane.online ? 'LIVE' : 'IDLE'}
                </span>
                <button
                  type="button"
                  title={maximized ? '恢复多窗布局' : '最大化窗格'}
                  onClick={(event) => {
                    event.stopPropagation();
                    setMaximizedPaneId(maximized ? null : pane.id);
                  }}
                >
                  <LynxIcon name={maximized ? 'panel-contract' : 'fullscreen'} size={16} />
                </button>
              </header>
              <div className="terminal-workspace-pane-content">{pane.content}</div>
            </article>
          );
        })}
      </main>

      <footer className="terminal-workspace-statusbar">
        <span>
          <i className={panes.some((pane) => pane.online) ? 'is-online' : ''} />
          {panes.filter((pane) => pane.online).length}/{panes.length} 通道在线
        </span>
        <span>设备：{device.name}</span>
        <span>项目：{projectName || '未绑定'}</span>
        <span>主机：{hostLabel || '未配置'}</span>
        <span>
          布局：{profile.visiblePaneCount} 窗 · {layoutLocked ? '锁定' : '自动'}
        </span>
        <strong>{syncInputEnabled ? 'SYNC ON' : 'INPUT ISOLATED'}</strong>
      </footer>

      {fullscreenError && <div className="terminal-workspace-error">{fullscreenError}</div>}
      <span className="terminal-workspace-readonly-contract" aria-readonly="true" hidden>
        日志窗格只读
      </span>
    </div>
  );
}

export default LinuxTerminalWorkspace;
