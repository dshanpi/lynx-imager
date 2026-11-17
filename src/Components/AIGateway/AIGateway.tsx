import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invokeCommand, subscribeEvent } from '../../Platform/IPC';
import type {
  GatewayInteraction,
  McuBoardConfig,
  McuMcpGatewayStatus,
  McuMcpInteraction,
  McuMcpTunnelStatus,
  WorkbenchDevice,
  WorkbenchProject,
  WorkbenchTunnelStatus,
} from '../../Platform/IPC/Commands';
import { formatErrorMessage } from '../../Utils';
import './AIGateway.css';

type LinuxAiDevice = {
  kind: 'linux';
  board: WorkbenchDevice;
  project: WorkbenchProject;
};

type McuAiDevice = {
  kind: 'mcu';
  board: McuBoardConfig;
  gateway: McuMcpGatewayStatus;
  tunnel?: McuMcpTunnelStatus;
};

type AiDevice = LinuxAiDevice | McuAiDevice;

function deviceId(device: AiDevice | undefined) {
  if (!device) return '';
  return device.kind === 'mcu' ? `mcu:${device.board.id}` : `linux:${device.project.id ?? ''}`;
}

function matchesDeviceKey(board: WorkbenchDevice, key: string) {
  return (
    key === `device:${board.id}` ||
    (!!board.serialPort && key === `serial:${board.serialPort}`) ||
    (!!board.usbBinding && key === `usb:${board.usbBinding}`) ||
    (!!board.adbSerial && key === `adb:${board.adbSerial}`)
  );
}

function formatTime(timestamp: number) {
  return new Intl.DateTimeFormat('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  }).format(new Date(timestamp));
}

export const AIGateway: React.FC = () => {
  const [devices, setDevices] = useState<WorkbenchDevice[]>([]);
  const [projects, setProjects] = useState<WorkbenchProject[]>([]);
  const [tunnels, setTunnels] = useState<WorkbenchTunnelStatus[]>([]);
  const [mcuBoards, setMcuBoards] = useState<McuBoardConfig[]>([]);
  const [mcuGateways, setMcuGateways] = useState<Record<string, McuMcpGatewayStatus>>({});
  const [mcuTunnels, setMcuTunnels] = useState<Record<string, McuMcpTunnelStatus>>({});
  const [interactions, setInteractions] = useState<Record<string, GatewayInteraction[]>>({});
  const [selectedId, setSelectedId] = useState('');
  const [message, setMessage] = useState('');
  const aiDevicesRef = useRef<AiDevice[]>([]);
  const mcuBoardsRef = useRef<McuBoardConfig[]>([]);

  mcuBoardsRef.current = mcuBoards;

  const realAiDevices = useMemo<AiDevice[]>(() => {
    const linuxDevices: LinuxAiDevice[] = projects.flatMap((project) => {
      if (
        !project.id ||
        !project.deviceId ||
        !tunnels.some((item) => item.projectId === project.id && item.state === 'connected')
      ) {
        return [];
      }
      const board = devices.find((item) => item.id === project.deviceId);
      if (!board) return [];
      return [{ kind: 'linux' as const, board, project }];
    });
    const mcuDevices: McuAiDevice[] = mcuBoards.flatMap((board) => {
      const tunnel = mcuTunnels[board.id];
      const gateway = mcuGateways[board.id];
      return gateway?.running ? [{ kind: 'mcu' as const, board, gateway, tunnel }] : [];
    });
    return [...linuxDevices, ...mcuDevices];
  }, [devices, mcuBoards, mcuGateways, mcuTunnels, projects, tunnels]);

  aiDevicesRef.current = realAiDevices;
  const selected = realAiDevices.find((item) => deviceId(item) === selectedId) ?? realAiDevices[0];
  const selectedDeviceId = deviceId(selected);
  const selectedInteractions = useMemo(
    () => interactions[selectedDeviceId] ?? [],
    [interactions, selectedDeviceId]
  );

  useEffect(() => {
    if (selected && deviceId(selected) !== selectedId) setSelectedId(deviceId(selected));
  }, [selected, selectedId]);

  const routeInteraction = useCallback((interaction: GatewayInteraction) => {
    const candidates = aiDevicesRef.current.filter(
      (item): item is LinuxAiDevice => item.kind === 'linux'
    );
    const target =
      candidates.find((item) => item.project.id === interaction.projectId) ??
      candidates.find(
        (item) =>
          item.board.id === interaction.deviceId &&
          matchesDeviceKey(item.board, interaction.deviceKey)
      ) ??
      candidates.find((item) => matchesDeviceKey(item.board, interaction.deviceKey)) ??
      (candidates.length === 1 ? candidates[0] : undefined);
    const id = deviceId(target);
    if (!id) return;
    setInteractions((current) => {
      const existing = current[id] ?? [];
      if (existing.some((item) => item.id === interaction.id)) return current;
      return {
        ...current,
        [id]: [...existing, interaction].slice(-300),
      };
    });
  }, []);

  const routeMcuInteraction = useCallback((interaction: McuMcpInteraction) => {
    const target = aiDevicesRef.current.find(
      (item): item is McuAiDevice => item.kind === 'mcu' && item.board.id === interaction.boardId
    );
    const id = deviceId(target);
    if (!id) return;
    const normalized: GatewayInteraction = {
      id: interaction.id,
      timestampMs: interaction.timestampMs,
      tool: interaction.tool,
      category: interaction.category,
      summary: interaction.summary,
      deviceKey: `mcu:${interaction.boardId}`,
      projectId: null,
      deviceId: null,
    };
    setInteractions((current) => {
      const existing = current[id] ?? [];
      if (existing.some((item) => item.id === normalized.id)) return current;
      return { ...current, [id]: [...existing, normalized].slice(-300) };
    });
  }, []);

  useEffect(() => {
    let cancelled = false;
    const loadLinux = async () => {
      const [nextDevices, nextProjects] = await Promise.all([
        invokeCommand('workbench_list_devices'),
        invokeCommand('workbench_list_projects'),
      ]);
      if (cancelled) return;
      setDevices(nextDevices);
      setProjects(nextProjects);
    };
    const loadMcu = async () => {
      const nextMcuBoards = await invokeCommand('mcu_list_boards');
      if (cancelled) return;
      setMcuBoards(nextMcuBoards);
      const nextMcuGateways: Array<readonly [string, McuMcpGatewayStatus]> = [];
      const gatewayErrors: string[] = [];
      for (const board of nextMcuBoards) {
        try {
          const status = await invokeCommand('mcu_mcp_gateway_start', {
            request: { boardId: board.id, port: board.mcpLocalPort },
          });
          nextMcuGateways.push([board.id, status]);
        } catch (value) {
          gatewayErrors.push(`${board.name}: ${formatErrorMessage(value)}`);
        }
      }
      if (!cancelled) {
        setMcuGateways(Object.fromEntries(nextMcuGateways));
        if (gatewayErrors.length) {
          setMessage(`MCU Gateway 启动失败：${gatewayErrors.join('；')}`);
        }
      }
    };
    const refreshLinux = async () => {
      const next = await invokeCommand('workbench_list_tunnel_statuses');
      if (!cancelled) setTunnels(next);
    };
    const refreshMcu = async () => {
      const [nextMcuResults, nextMcuGatewayResults] = await Promise.all([
        Promise.allSettled(
          mcuBoardsRef.current.map(
            async (board) =>
              [
                board.id,
                await invokeCommand('mcu_mcp_tunnel_status', { boardId: board.id }),
              ] as const
          )
        ),
        Promise.allSettled(
          mcuBoardsRef.current.map(async (board) => {
            const status = await invokeCommand('mcu_mcp_gateway_status', {
              port: board.mcpLocalPort,
            });
            return [board.id, status.boardId === board.id ? status : null] as const;
          })
        ),
      ]);
      if (!cancelled) {
        const nextMcu = nextMcuResults.flatMap((result) =>
          result.status === 'fulfilled' ? [result.value] : []
        );
        const nextMcuGateways = nextMcuGatewayResults.flatMap((result) =>
          result.status === 'fulfilled' ? [result.value] : []
        );
        setMcuTunnels(
          Object.fromEntries(nextMcu.filter((item) => item[1])) as Record<
            string,
            McuMcpTunnelStatus
          >
        );
        setMcuGateways(
          Object.fromEntries(nextMcuGateways.filter((item) => item[1])) as Record<
            string,
            McuMcpGatewayStatus
          >
        );
      }
    };
    const load = async () => {
      try {
        await loadLinux();
        await refreshLinux();
      } catch (value) {
        if (!cancelled) setMessage(`Linux/SDK 通道加载失败：${formatErrorMessage(value)}`);
      }
      try {
        await loadMcu();
        await refreshMcu();
      } catch (value) {
        if (!cancelled) setMessage(`MCU 通道加载失败：${formatErrorMessage(value)}`);
      }
    };
    void load();
    const timer = window.setInterval(() => {
      void refreshLinux().catch(() => undefined);
      void refreshMcu().catch(() => undefined);
    }, 1500);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);

  useEffect(() => {
    const unlisteners: Array<() => void> = [];
    subscribeEvent('gateway-interaction', routeInteraction).then((unlisten) =>
      unlisteners.push(unlisten)
    );
    subscribeEvent('mcu-mcp-interaction', routeMcuInteraction).then((unlisten) =>
      unlisteners.push(unlisten)
    );
    return () => unlisteners.forEach((unlisten) => unlisten());
  }, [routeInteraction, routeMcuInteraction]);

  useEffect(() => {
    if (!realAiDevices.length) return;
    invokeCommand('gateway_list_interactions', { limit: 200 })
      .then((items) => items.reverse().forEach(routeInteraction))
      .catch((value) => setMessage(`Linux/SDK 交互记录加载失败：${formatErrorMessage(value)}`));
    invokeCommand('mcu_mcp_list_interactions', { limit: 200 })
      .then((items) => items.reverse().forEach(routeMcuInteraction))
      .catch((value) => setMessage(`MCU 交互记录加载失败：${formatErrorMessage(value)}`));
  }, [realAiDevices.length, routeInteraction, routeMcuInteraction]);

  const clearInteractions = async () => {
    if (!selected) return;
    if (selected.kind === 'mcu') {
      await invokeCommand('mcu_mcp_clear_interactions', { boardId: selected.board.id });
      setInteractions((current) => ({ ...current, [deviceId(selected)]: [] }));
    } else {
      await invokeCommand('gateway_clear_interactions');
      setInteractions((current) =>
        Object.fromEntries(Object.entries(current).filter(([key]) => !key.startsWith('linux:')))
      );
    }
    setMessage(`${selected.kind === 'mcu' ? 'MCU' : 'Linux/SDK'} AI 交互记录已清空。`);
  };

  const categoryCounts = useMemo(() => {
    const counts: Record<string, number> = {};
    for (const item of selectedInteractions) {
      counts[item.category] = (counts[item.category] ?? 0) + 1;
    }
    return counts;
  }, [selectedInteractions]);

  return (
    <div className="gateway-console">
      <aside className="gateway-channel-list">
        <div className="gateway-title gateway-device-title">
          <div>
            <small>AI CHANNELS</small>
            <h3>AI 开发设备</h3>
          </div>
          <span>{realAiDevices.length} 台在线</span>
        </div>

        {realAiDevices.map((item) => {
          const count = interactions[deviceId(item)]?.length ?? 0;
          return (
            <button
              key={deviceId(item)}
              className={deviceId(item) === deviceId(selected) ? 'is-active' : ''}
              onClick={() => setSelectedId(deviceId(item))}
            >
              <span className="gateway-device-name">
                <i />
                <strong>{item.board.name}</strong>
              </span>
              <span>
                {item.kind === 'mcu'
                  ? 'MCU · 专用 MCP'
                  : item.project.aiProvider === 'claude_code'
                    ? 'Claude Code'
                    : 'Codex'}
              </span>
              <span>{item.kind === 'mcu' ? item.board.mcuModel : item.project.name}</span>
              <b>{count} 条 AI 交互</b>
            </button>
          );
        })}

        {!realAiDevices.length && (
          <div className="gateway-waiting">
            <i />
            <strong>等待 AI 通道连接</strong>
            <span>开发工作台保存单板和 SDK 通道并连接成功后，会自动出现在这里。</span>
          </div>
        )}
      </aside>

      <main className="gateway-main">
        <section className="gateway-hero">
          <div>
            <small>AI INTERACTIONS</small>
            <h3>{selected?.board.name || 'AI 通道'}</h3>
            <p>
              {selected
                ? selected.kind === 'mcu'
                  ? `MCU 专用通道 · ${selected.board.mcuModel} · 与 Linux 通道独立`
                  : `${selected.project.aiProvider === 'claude_code' ? 'Claude Code' : 'Codex'} · ${selected.project.name}`
                : '这里只展示 AI 的连接和工具调用数据；硬件操作请进入开发工作台。'}
            </p>
          </div>
          {selected && (
            <div className="gateway-ai-meta">
              <span>
                SDK {selected.kind === 'mcu' ? '远端 AI 当前目录' : selected.project.sdkDir}
              </span>
              <span>
                MCP{' '}
                {selected.kind === 'mcu'
                  ? selected.board.mcpServerName
                  : selected.project.mcpServerName}
              </span>
              <span>
                状态{' '}
                {selected.kind === 'mcu' && selected.tunnel?.state !== 'connected'
                  ? '本机 Gateway 已就绪'
                  : '已连接'}
              </span>
            </div>
          )}
        </section>

        {selected ? (
          <>
            <section className="gateway-ai-summary">
              <div>
                <small>全部交互</small>
                <strong>{selectedInteractions.length}</strong>
              </div>
              {Object.entries(categoryCounts)
                .slice(0, 5)
                .map(([category, count]) => (
                  <div key={category}>
                    <small>{category}</small>
                    <strong>{count}</strong>
                  </div>
                ))}
              <button type="button" onClick={clearInteractions}>
                清空记录
              </button>
            </section>

            <section className="gateway-ai-feed">
              <header>
                <div>
                  <small>LIVE ACTIVITY</small>
                  <strong>AI 实时交互数据</strong>
                </div>
                <span>按当前单板隔离显示</span>
              </header>
              <div className="gateway-ai-events">
                {[...selectedInteractions].reverse().map((interaction) => (
                  <article key={interaction.id}>
                    <i className={`is-${interaction.category.toLowerCase()}`} />
                    <div>
                      <span>
                        <b>{interaction.category}</b>
                        <time>{formatTime(interaction.timestampMs)}</time>
                      </span>
                      <strong>{interaction.summary}</strong>
                      <code>{interaction.tool}</code>
                    </div>
                  </article>
                ))}
                {!selectedInteractions.length && (
                  <div className="gateway-ai-empty">
                    <strong>正在等待 AI 操作</strong>
                    <p>远端 Codex 或 Claude Code 调用该单板的 LYNX 工具后，会实时显示在这里。</p>
                  </div>
                )}
              </div>
            </section>
          </>
        ) : (
          <div className="gateway-empty">
            <strong>暂无已连接的 AI 开发设备</strong>
            <p>先到开发工作台保存单板、远端 SDK 主机和 AI 通道。</p>
          </div>
        )}

        {message && <div className="gateway-message">{message}</div>}
      </main>
    </div>
  );
};

export default AIGateway;
