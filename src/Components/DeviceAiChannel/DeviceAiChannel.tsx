import React, { useEffect, useMemo, useState } from 'react';
import { LynxIcon } from '../../CoreUI';
import { invokeCommand, subscribeEvent } from '../../Platform/IPC';
import type {
  GatewayInteraction,
  McuBoardConfig,
  McuMcpInteraction,
  McuMcpTunnelStatus,
  WorkbenchDevice,
  WorkbenchHost,
  WorkbenchProject,
  WorkbenchTunnelStatus,
} from '../../Platform/IPC/Commands';
import { formatErrorMessage } from '../../Utils';
import './DeviceAiChannel.css';

type LinuxProps = {
  kind: 'linux';
  boardId: number;
  board: WorkbenchDevice;
  project?: WorkbenchProject;
  host?: WorkbenchHost;
  tunnel?: WorkbenchTunnelStatus;
  onOpenSettings: () => void;
};

type McuProps = {
  kind: 'mcu';
  boardId: string;
  board: McuBoardConfig;
  tunnel?: McuMcpTunnelStatus | null;
  onOpenSettings: () => void;
};

type DeviceAiChannelProps = LinuxProps | McuProps;

function time(value: number) {
  return new Intl.DateTimeFormat('zh-CN', {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  }).format(new Date(value));
}

function matchesLinux(interaction: GatewayInteraction, props: LinuxProps) {
  return (
    interaction.projectId === props.project?.id ||
    interaction.deviceId === props.boardId ||
    interaction.deviceKey === `device:${props.boardId}` ||
    (!!props.board.serialPort && interaction.deviceKey === `serial:${props.board.serialPort}`) ||
    (!!props.board.usbBinding && interaction.deviceKey === `usb:${props.board.usbBinding}`) ||
    (!!props.board.adbSerial && interaction.deviceKey === `adb:${props.board.adbSerial}`)
  );
}

export const DeviceAiChannel: React.FC<DeviceAiChannelProps> = (props) => {
  const [interactions, setInteractions] = useState<GatewayInteraction[]>([]);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [error, setError] = useState('');
  const [gatewayReady, setGatewayReady] = useState(false);
  const connected =
    props.kind === 'linux'
      ? props.tunnel?.state === 'connected'
      : props.tunnel?.state === 'connected';
  const scopeKey = `${props.kind}:${props.boardId}`;
  const subscriptionKey =
    props.kind === 'linux'
      ? `${scopeKey}:${props.project?.id ?? ''}:${props.board.serialPort}:${props.board.usbBinding}:${props.board.adbSerial}`
      : scopeKey;
  const gatewayPort =
    props.kind === 'linux' ? props.project?.localGatewayPort : props.board.mcpLocalPort;
  const linuxProjectConfigured = props.kind === 'linux' && Boolean(props.project);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    setInteractions([]);
    setSelectedId(null);
    setError('');
    if (props.kind === 'linux') {
      invokeCommand('gateway_list_interactions', { limit: 200 })
        .then((items) => {
          if (cancelled) return;
          const scoped = items.filter((item) => matchesLinux(item, props));
          setInteractions(scoped);
          setSelectedId(scoped[scoped.length - 1]?.id ?? null);
        })
        .catch((value) => !cancelled && setError(`调用记录加载失败：${formatErrorMessage(value)}`));
      void subscribeEvent('gateway-interaction', (item) => {
        if (!matchesLinux(item, props)) return;
        setInteractions((current) =>
          [...current.filter((entry) => entry.id !== item.id), item].slice(-300)
        );
        setSelectedId(item.id);
      }).then((value) => {
        unlisten = value;
      });
    } else {
      invokeCommand('mcu_mcp_list_interactions', { limit: 200 })
        .then((items) => {
          if (cancelled) return;
          const scoped = items
            .filter((item) => item.boardId === props.boardId)
            .map(normalizeMcuInteraction);
          setInteractions(scoped);
          setSelectedId(scoped[scoped.length - 1]?.id ?? null);
        })
        .catch(
          (value) => !cancelled && setError(`MCU 调用记录加载失败：${formatErrorMessage(value)}`)
        );
      void subscribeEvent('mcu-mcp-interaction', (item) => {
        if (item.boardId !== props.boardId) return;
        const normalized = normalizeMcuInteraction(item);
        setInteractions((current) =>
          [...current.filter((entry) => entry.id !== normalized.id), normalized].slice(-300)
        );
        setSelectedId(normalized.id);
      }).then((value) => {
        unlisten = value;
      });
    }
    return () => {
      cancelled = true;
      unlisten?.();
    };
    // The saved board bindings are the subscription boundary. Connection
    // status refreshes do not reload the complete call feed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [subscriptionKey]);

  useEffect(() => {
    let cancelled = false;
    setGatewayReady(false);
    if (props.kind === 'linux') {
      invokeCommand('gateway_status')
        .then((status) => {
          if (!cancelled)
            setGatewayReady(
              status.running && (!linuxProjectConfigured || status.port === gatewayPort)
            );
        })
        .catch(() => !cancelled && setGatewayReady(false));
    } else {
      invokeCommand('mcu_mcp_gateway_status', { port: gatewayPort })
        .then((status) => {
          if (!cancelled) setGatewayReady(status.running && status.boardId === props.boardId);
        })
        .catch(() => !cancelled && setGatewayReady(false));
    }
    return () => {
      cancelled = true;
    };
  }, [gatewayPort, linuxProjectConfigured, props.boardId, props.kind]);

  const selected =
    interactions.find((item) => item.id === selectedId) ?? interactions[interactions.length - 1];
  const categories = useMemo(
    () => new Set(interactions.map((item) => item.category)).size,
    [interactions]
  );
  const channel =
    props.kind === 'linux'
      ? {
          gateway: props.project ? `127.0.0.1:${props.project.localGatewayPort}` : '未配置',
          host: props.host ? `${props.host.address}:${props.host.port}` : '未配置远端主机',
          ai: props.project?.aiProvider === 'claude_code' ? 'Claude Code' : 'Codex',
          mcp: props.project?.mcpServerName || '未配置 MCP',
          endpoint: props.project ? `远端端口 ${props.project.remoteForwardPort}` : '未配置',
          sdk: props.project?.sdkDir || '未配置 SDK 目录',
        }
      : {
          gateway: `127.0.0.1:${props.board.mcpLocalPort}`,
          host: props.board.mcpHostAddress
            ? `${props.board.mcpHostAddress}:${props.board.mcpHostPort}`
            : '未配置远端主机',
          ai: 'Codex / Claude Code',
          mcp: props.board.mcpServerName || '未配置 MCU MCP',
          endpoint: `远端端口 ${props.tunnel?.remotePort || props.board.mcpRemotePort}`,
          sdk: props.board.mcuSdkDir || '未配置 MCU SDK 目录',
        };

  return (
    <div className="device-ai-channel">
      <section className="device-ai-channel-intro">
        <div>
          <small>AI DEVICE CONTROL PLANE</small>
          <h3>设备级 AI 通道</h3>
          <p>远端 AI 通过受控 MCP 链路访问当前设备，所有工具调用均保留设备作用域与审计证据。</p>
        </div>
        <span className={connected && gatewayReady ? 'is-ready' : ''}>
          <i />
          {connected && gatewayReady ? '链路可用' : '等待链路'}
        </span>
      </section>

      <section className="device-ai-channel-summary">
        {[
          ['Gateway', channel.gateway, gatewayReady],
          ['SSH 隧道', channel.host, connected],
          ['远端 AI', channel.ai, connected],
          ['MCP 服务', channel.mcp, connected],
        ].map(([label, value, ready]) => (
          <div key={String(label)} className={ready ? 'is-ready' : ''}>
            <span>
              {ready ? <LynxIcon name="check" size={16} /> : <LynxIcon name="error" size={16} />}
            </span>
            <p>
              <strong>{label}</strong>
              <small>{value}</small>
            </p>
          </div>
        ))}
      </section>

      <section className="device-ai-channel-chain">
        <header>
          <div>
            <LynxIcon name="link" size={20} />
            <strong>连接链路</strong>
          </div>
          <span>{connected ? '全部正常' : '存在未连接节点'}</span>
        </header>
        <div>
          <article>
            <LynxIcon name="server" size={20} />
            <strong>LYNX 本机</strong>
            <small>{channel.gateway}</small>
          </article>
          <i>
            <LynxIcon name="branch" size={16} />
          </i>
          <article>
            <LynxIcon name="link" size={20} />
            <strong>SSH</strong>
            <small>{channel.host}</small>
          </article>
          <i>
            <LynxIcon name="branch" size={16} />
          </i>
          <article>
            <LynxIcon name="brain" size={20} />
            <strong>{channel.ai}</strong>
            <small>{connected ? '远端可用' : '等待连接'}</small>
          </article>
          <i>
            <LynxIcon name="branch" size={16} />
          </i>
          <article>
            <LynxIcon name="wrench" size={20} />
            <strong>{channel.mcp}</strong>
            <small>{channel.endpoint}</small>
          </article>
        </div>
      </section>

      <section className="device-ai-channel-config">
        <div>
          <span>设备作用域</span>
          <strong>
            {props.kind}:{props.boardId}
          </strong>
        </div>
        <div>
          <span>SDK 目录</span>
          <strong>{channel.sdk}</strong>
        </div>
        <div>
          <span>传输状态</span>
          <strong>{connected ? '已连接并验证' : '尚未连接'}</strong>
        </div>
        <div className="device-ai-key">
          <LynxIcon name="key" size={16} />
          <span>API Key 仅存系统密钥环</span>
        </div>
        <button onClick={props.onOpenSettings}>打开设备设置</button>
      </section>

      {error && <div className="device-ai-channel-error">{error}</div>}

      <div className="device-ai-channel-data">
        <section className="device-ai-call-list">
          <header>
            <div>
              <strong>实时工具调用</strong>
              <span>
                {interactions.length} 条 · {categories} 类
              </span>
            </div>
            <LynxIcon name="refresh" size={16} />
          </header>
          <div className="device-ai-call-table">
            {interactions
              .slice()
              .reverse()
              .map((item) => (
                <button
                  key={item.id}
                  className={item.id === selected?.id ? 'is-selected' : ''}
                  onClick={() => setSelectedId(item.id)}
                >
                  <time>{time(item.timestampMs)}</time>
                  <span>{item.category}</span>
                  <strong>{item.tool}</strong>
                  <em>成功</em>
                </button>
              ))}
            {!interactions.length && (
              <div className="device-ai-empty">
                <strong>等待当前设备的 AI 工具调用</strong>
                <span>
                  只有匹配 {props.kind}:{props.boardId} 的真实调用会显示在这里。
                </span>
              </div>
            )}
          </div>
        </section>
        <section className="device-ai-call-detail">
          <header>
            <strong>调用详情</strong>
            <span>{selected ? 'MCP 工具调用' : '未选择'}</span>
          </header>
          {selected ? (
            <dl>
              <div>
                <dt>时间</dt>
                <dd>{time(selected.timestampMs)}</dd>
              </div>
              <div>
                <dt>工具</dt>
                <dd>{selected.tool}</dd>
              </div>
              <div>
                <dt>分类</dt>
                <dd>{selected.category}</dd>
              </div>
              <div>
                <dt>设备作用域</dt>
                <dd>
                  {props.kind}:{props.boardId}
                </dd>
              </div>
              <div>
                <dt>摘要</dt>
                <dd>{selected.summary}</dd>
              </div>
              <div>
                <dt>来源</dt>
                <dd>{props.kind === 'linux' ? channel.ai : 'MCU 专用 MCP'}</dd>
              </div>
            </dl>
          ) : (
            <div className="device-ai-empty">
              <span>选择一条调用查看详情。</span>
            </div>
          )}
        </section>
      </div>
    </div>
  );
};

function normalizeMcuInteraction(item: McuMcpInteraction): GatewayInteraction {
  return {
    id: item.id,
    timestampMs: item.timestampMs,
    tool: item.tool,
    category: item.category,
    summary: item.summary,
    deviceKey: `mcu:${item.boardId}`,
    projectId: null,
    deviceId: null,
  };
}

export default DeviceAiChannel;
