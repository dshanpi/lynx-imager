import type { WorkbenchTunnelStatus, LocalMcpStatus } from '../../Platform/IPC/Commands';

const validPort = (port: number | null) =>
  port !== null && Number.isInteger(port) && port > 0 && port <= 65535;

export function localMcpConnection(
  status: LocalMcpStatus | null,
  projectId: number | null,
  deviceId: number | null,
  name: string
) {
  const ready =
    !!projectId &&
    !!deviceId &&
    status?.projectId === projectId &&
    status.deviceId === deviceId &&
    status.running &&
    status.verified &&
    validPort(status.port) &&
    status.endpoint === `http://127.0.0.1:${status.port}/mcp` &&
    /^[a-zA-Z0-9_-]+$/.test(name);
  const endpoint = ready ? status.endpoint! : '';
  return {
    endpoint,
    command: endpoint ? `codex -c 'mcp_servers={"${name}"={url="${endpoint}"}}'` : '',
  };
}

export function remoteMcpConnection(
  status: WorkbenchTunnelStatus | null | undefined,
  projectId: number | null
) {
  const ready =
    !!projectId &&
    status?.projectId === projectId &&
    status.state === 'connected' &&
    validPort(status.remotePort);
  return { endpoint: ready ? `http://127.0.0.1:${status.remotePort}/mcp` : '' };
}
