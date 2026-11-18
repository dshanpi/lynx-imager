import { useEffect, useRef, useState } from 'react';
import { confirm } from '@tauri-apps/plugin-dialog';
import { invokeCommand } from '../../Platform/IPC';
import type {
  GatewayInteraction,
  LocalMcpStatus,
  WorkbenchDevice,
  WorkbenchHost,
  WorkbenchProject,
  WorkbenchTunnelStatus,
} from '../../Platform/IPC/Commands';
import { localMcpConnection, remoteMcpConnection } from '../Workbench/mcpConnections';
import {
  buildMcpClientConfig,
  MCP_CLIENTS,
  type McpClientId,
} from '../Workbench/mcpClientProfiles';
import { testAndConfirmSshHost } from '../Workbench/sshHostTrust';
import { EMPTY_WORKBENCH_HOST, EMPTY_WORKBENCH_PROJECT } from '../Workbench/linuxWorkbenchModel';
import {
  AIFacts,
  AIMessage,
  AITable,
  copyVerifiedText,
  safeDetail,
  timeLabel,
  useAIAction,
} from './imagerAIControls';

interface Props {
  section: 'local' | 'remote';
  device: WorkbenchDevice | null;
  project: WorkbenchProject | null;
  projects: WorkbenchProject[];
  hosts: WorkbenchHost[];
  onSaved: () => void;
  onOpenRemote: () => void;
}

export function ImagerAIChannels({
  section,
  device,
  project,
  hosts,
  projects,
  onSaved,
  onOpenRemote,
}: Props) {
  const savedHost = hosts.find((h) => h.id === project?.hostId);
  const [host, setHost] = useState<WorkbenchHost>(() => ({
    ...(savedHost || EMPTY_WORKBENCH_HOST),
  }));
  const [draft, setDraft] = useState<WorkbenchProject>(
    () => project || { ...EMPTY_WORKBENCH_PROJECT, name: device ? device.name + ' SDK' : '' }
  );
  const [password, setPassword] = useState('');
  const [client, setClient] = useState<McpClientId>('custom');
  const [local, setLocal] = useState<LocalMcpStatus | null>(null);
  const [tunnel, setTunnel] = useState<WorkbenchTunnelStatus | null>(null);
  const [history, setHistory] = useState<GatewayInteraction[]>([]);
  const [failures, setFailures] = useState<string[]>([]);
  const [refresh, setRefresh] = useState(0);
  const current = useRef(true);
  const { run, busy, notice } = useAIAction([password]);
  const did = device?.id ?? null;
  const pid = project?.id ?? null;
  const name = project?.mcpServerName || '';
  const localEndpoint = localMcpConnection(local, pid, did, name).endpoint;
  const remoteEndpoint = remoteMcpConnection(tunnel, pid).endpoint;
  const shell = /Windows/i.test(navigator.userAgent) ? 'powershell' : 'posix';
  const config = buildMcpClientConfig(client, localEndpoint, name, shell);
  const remoteConfig = buildMcpClientConfig(client, remoteEndpoint, name, 'posix');

  useEffect(() => {
    current.current = true;
    return () => {
      current.current = false;
    };
  }, []);
  useEffect(() => {
    if (!pid || !did) return;
    let stopped = false;
    let timer: number | undefined;
    async function read() {
      const [gateway, tunnels, calls] = await Promise.allSettled([
        invokeCommand('gateway_channel_status', { projectId: pid!, deviceId: did! }),
        invokeCommand('workbench_list_tunnel_statuses'),
        invokeCommand('gateway_list_interactions', { limit: 100 }),
      ]);
      if (stopped) return;
      setLocal(gateway.status === 'fulfilled' ? gateway.value : null);
      setTunnel(
        tunnels.status === 'fulfilled'
          ? tunnels.value.find((t) => t.projectId === pid) || null
          : null
      );
      setHistory(
        calls.status === 'fulfilled'
          ? calls.value.filter((c) => c.projectId === pid && c.deviceId === did).slice(0, 10)
          : []
      );
      setFailures(
        [gateway, tunnels, calls].flatMap((r, i) =>
          r.status === 'rejected'
            ? [['本机服务', '远端隧道', '调用记录'][i] + '：' + safeDetail(r.reason)]
            : []
        )
      );
      timer = window.setTimeout(read, 5000);
    }
    void read();
    return () => {
      stopped = true;
      window.clearTimeout(timer);
    };
  }, [pid, did, refresh]);

  async function copy(scope: 'local' | 'remote', withConfig = false) {
    if (!pid || !did) return;
    await copyVerifiedText(
      (async () => {
        const endpoint =
          scope === 'local'
            ? localMcpConnection(
                await invokeCommand('gateway_channel_status', { projectId: pid, deviceId: did }),
                pid,
                did,
                name
              ).endpoint
            : remoteMcpConnection(
                (await invokeCommand('workbench_list_tunnel_statuses')).find(
                  (t) => t.projectId === pid
                ),
                pid
              ).endpoint;
        if (!current.current) throw new Error('通道已切换。');
        if (!endpoint || endpoint !== (scope === 'local' ? localEndpoint : remoteEndpoint)) {
          setRefresh((n) => n + 1);
          throw new Error('连接状态或端口已变化，请核对刷新后的地址。');
        }
        const value = withConfig
          ? buildMcpClientConfig(client, endpoint, name, scope === 'local' ? shell : 'posix').value
          : endpoint;
        if (!value) throw new Error('当前通道缺少可用的客户端配置。');
        return value;
      })()
    );
    return withConfig ? '已复制当前客户端配置。' : '已复制已验证地址。';
  }

  function updateHost(patch: Partial<WorkbenchHost>) {
    setPassword('');
    setHost((h) => ({ ...h, ...patch, hostFingerprint: '' }));
  }
  async function verifyHost() {
    if (
      !host.address.trim() ||
      !host.username.trim() ||
      !Number.isInteger(host.port) ||
      host.port < 1 ||
      host.port > 65535
    )
      throw new Error('请填写 SSH 主机、用户以及 1 到 65535 之间的端口。');
    const existing = hosts.find(
      (h) =>
        h.address === host.address.trim() &&
        h.username === host.username.trim() &&
        h.port === host.port
    );
    const candidate = {
      ...host,
      id: existing?.id ?? null,
      address: host.address.trim(),
      username: host.username.trim(),
      name: existing?.name || host.address.trim(),
      hostFingerprint: existing?.hostFingerprint || host.hostFingerprint,
    };
    const credentials =
      host.authType === 'password'
        ? { password: password || undefined }
        : { keyPassphrase: password || undefined };
    const result = await testAndConfirmSshHost(candidate, {
      test: (target) =>
        invokeCommand('workbench_test_ssh', { host: target, credentials, allowUntrusted: false }),
      confirm: (text) =>
        confirm(text, {
          title: '核验 SSH 主机指纹',
          kind: 'warning',
          okLabel: '已核实指纹',
          cancelLabel: '取消',
        }),
      describeError: safeDetail,
      isCurrent: () => current.current,
      sharedChannels: existing?.id ? projects.filter((p) => p.hostId === existing.id).length : 0,
    });
    if (!result || !current.current) return null;
    const hostId = await invokeCommand('workbench_save_host', {
      host: result.host,
      connectionTestSessionId: result.result.auditSessionId,
    });
    if (password && current.current)
      await invokeCommand('workbench_save_credentials', { hostId, credentials });
    if (!current.current) return null;
    setPassword('');
    setHost({ ...result.host, id: hostId });
    onSaved();
    return hostId;
  }
  async function saveChannel() {
    if (!did || !draft.name.trim() || !draft.sdkDir.trim().startsWith('/'))
      throw new Error('请选择设备，并填写通道名称和远端 SDK 的绝对路径。');
    const hostId = await verifyHost();
    if (!hostId || !current.current) return;
    const other = projects.filter((p) => p.id !== pid);
    function available(preferred: number, used: number[]) {
      let port = preferred;
      while (used.includes(port) && port < 65535) port++;
      if (used.includes(port)) throw new Error('通道端口已用完。');
      return port;
    }
    const value: WorkbenchProject = {
      ...draft,
      hostId,
      deviceId: did,
      name: draft.name.trim(),
      sdkDir: draft.sdkDir.trim(),
      mcpServerName: project?.mcpServerName || 'lynx-board-' + did + '-sdk',
      localGatewayPort: available(
        draft.localGatewayPort,
        other.map((p) => p.localGatewayPort)
      ),
      remoteForwardPort: available(
        draft.remoteForwardPort,
        other.filter((p) => p.hostId === hostId).map((p) => p.remoteForwardPort)
      ),
    };
    const id = await invokeCommand('workbench_save_project', { project: value });
    if (!current.current) return;
    try {
      await invokeCommand('gateway_start', {
        port: value.localGatewayPort,
        deviceId: did,
        projectId: id,
      });
      if (current.current) await invokeCommand('workbench_start_tunnel', { projectId: id });
    } finally {
      onSaved();
    }
    setRefresh((n) => n + 1);
    return '通道已保存，连接结果见链路验证。';
  }

  const remoteState =
    tunnel?.state === 'connected' && remoteEndpoint
      ? '远端 HTTP 通过'
      : {
          connecting: '正在连接',
          verifying: '正在验证',
          reconnecting: '正在重连',
          stopped: '已停止',
          connected: '待验证',
          host_key_required: '待核验指纹',
        }[tunnel?.state || 'stopped'];
  const callRows = history.map((item) => (
    <tr key={item.id}>
      <td>{timeLabel(item.timestampMs)}</td>
      <td>
        <code>{item.tool}</code>
      </td>
      <td>{device?.name}</td>
      <td>{item.category}</td>
    </tr>
  ));
  return (
    <>
      {section === 'local' ? (
        <>
          <div className="im-ai-columns">
            <section>
              <h2>本机服务</h2>
              <AIFacts
                rows={[
                  ['服务名称', name || '尚未配置 SDK 通道'],
                  [
                    '本机 MCP 地址',
                    <code key="endpoint">{localEndpoint || '尚无已验证地址'}</code>,
                  ],
                  ['协议', 'Streamable HTTP'],
                  ['设备作用域', device?.name || '未选择设备'],
                  [
                    '服务验证',
                    localEndpoint ? '本机 HTTP 通过' : safeDetail(local?.message || '尚未验证'),
                  ],
                ]}
              />
              <div className="im-ai-actions">
                <button disabled={busy || !pid} onClick={() => setRefresh((n) => n + 1)}>
                  检查服务
                </button>
                <button
                  disabled={busy || !localEndpoint}
                  onClick={() => void run(() => copy('local'))}
                >
                  复制地址
                </button>
                {pid && did && local && !local.running && (
                  <button
                    disabled={busy}
                    onClick={() =>
                      void run(async () => {
                        await invokeCommand('gateway_start', {
                          port: project!.localGatewayPort,
                          projectId: pid,
                          deviceId: did,
                        });
                        setRefresh((n) => n + 1);
                        return '已请求启动本机服务。';
                      })
                    }
                  >
                    启动本机服务
                  </button>
                )}
                {!pid && device && <button onClick={onOpenRemote}>配置 SDK 通道</button>}
              </div>
            </section>
            <section>
              <h2>客户端配置</h2>
              <label>
                客户端
                <select value={client} onChange={(e) => setClient(e.target.value as McpClientId)}>
                  {MCP_CLIENTS.map((c) => (
                    <option key={c.id} value={c.id}>
                      {c.label}
                    </option>
                  ))}
                </select>
              </label>
              <pre className="im-ai-code">
                {config.value || '本机服务通过验证后，此处显示客户端配置。'}
              </pre>
              <button
                className="im-ai-primary"
                disabled={busy || !config.value}
                onClick={() => void run(() => copy('local', true))}
              >
                {config.kind === 'command' ? '复制启动命令' : '复制配置'}
              </button>
            </section>
          </div>
          <section>
            <h2>最近工具调用</h2>
            <AITable
              columns={['时间', '工具', '设备', '类别']}
              empty={history.length ? '' : '当前通道尚无调用记录。'}
            >
              {callRows}
            </AITable>
          </section>
        </>
      ) : (
        <>
          <div className="im-ai-columns">
            <section>
              <h2>SSH 与 SDK</h2>
              <fieldset className="im-ai-fields" disabled={busy}>
                <label>
                  通道名称
                  <input
                    value={draft.name}
                    onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                  />
                </label>
                <label>
                  远端终端工具
                  <select
                    value={draft.aiProvider}
                    onChange={(e) =>
                      setDraft({
                        ...draft,
                        aiProvider: e.target.value as WorkbenchProject['aiProvider'],
                      })
                    }
                  >
                    <option value="codex">Codex</option>
                    <option value="claude_code">Claude Code</option>
                    <option value="deepseek_harness">DeepSeek</option>
                  </select>
                </label>
                <label>
                  SSH 主机
                  <input
                    value={host.address}
                    onChange={(e) => updateHost({ address: e.target.value })}
                  />
                </label>
                <label>
                  SSH 用户
                  <input
                    value={host.username}
                    onChange={(e) => updateHost({ username: e.target.value })}
                  />
                </label>
                <label>
                  SSH 端口
                  <input
                    type="number"
                    min="1"
                    max="65535"
                    value={host.port || ''}
                    onChange={(e) => updateHost({ port: Number(e.target.value) })}
                  />
                </label>
                <label>
                  远端 SDK 目录
                  <input
                    value={draft.sdkDir}
                    onChange={(e) => setDraft({ ...draft, sdkDir: e.target.value })}
                  />
                </label>
              </fieldset>
              <details>
                <summary>SSH 认证与指纹</summary>
                <fieldset className="im-ai-fields" disabled={busy}>
                  <label>
                    认证方式
                    <select
                      value={host.authType}
                      onChange={(e) => {
                        setPassword('');
                        setHost({ ...host, authType: e.target.value as WorkbenchHost['authType'] });
                      }}
                    >
                      <option value="private_key">私钥</option>
                      <option value="password">密码</option>
                    </select>
                  </label>
                  {host.authType === 'private_key' && (
                    <label>
                      私钥文件路径
                      <input
                        value={host.privateKeyPath}
                        onChange={(e) => setHost({ ...host, privateKeyPath: e.target.value })}
                      />
                    </label>
                  )}
                  <label>
                    {host.authType === 'password' ? 'SSH 密码' : '私钥口令'}
                    <input
                      type="password"
                      autoComplete="off"
                      placeholder="留空沿用已保存凭据"
                      value={password}
                      onChange={(e) => setPassword(e.target.value)}
                    />
                  </label>
                </fieldset>
                <p className="im-ai-muted">
                  已保存指纹 <code>{host.hostFingerprint || '尚未核验'}</code>
                </p>
              </details>
            </section>
            <section>
              <h2>连接链路</h2>
              <AITable columns={['节点', '资源', '验证结果']}>
                <tr>
                  <td>本机网关</td>
                  <td>{name || '尚未配置'}</td>
                  <td>{localEndpoint ? '本机 HTTP 通过' : '未验证'}</td>
                </tr>
                <tr>
                  <td>SSH 隧道</td>
                  <td>{savedHost ? savedHost.address + ':' + savedHost.port : '尚未配置'}</td>
                  <td>{remoteState}</td>
                </tr>
                <tr>
                  <td>远端回环端口</td>
                  <td>{remoteEndpoint ? tunnel?.remotePort : '等待分配'}</td>
                  <td>{remoteEndpoint ? '远端 HTTP 通过' : '未验证'}</td>
                </tr>
              </AITable>
              <p className="im-ai-muted">
                {tunnel?.message && !remoteEndpoint
                  ? safeDetail(tunnel.message)
                  : '远端端口在隧道建立后确认，重新连接时可能改变。'}
              </p>
            </section>
          </div>
          <section>
            <h2>两端 MCP 地址</h2>
            <AITable columns={['执行位置', '已验证地址', '操作']}>
              {(['local', 'remote'] as const).map((side) => (
                <tr key={side}>
                  <td>{side === 'local' ? '运行 LYNX 的电脑' : '远端 SDK 主机'}</td>
                  <td>
                    <code>
                      {(side === 'local' ? localEndpoint : remoteEndpoint) || '尚无已验证地址'}
                    </code>
                  </td>
                  <td>
                    <button
                      disabled={busy || !(side === 'local' ? localEndpoint : remoteEndpoint)}
                      onClick={() => void run(() => copy(side))}
                    >
                      {side === 'local' ? '复制本机地址' : '复制远端地址'}
                    </button>
                  </td>
                </tr>
              ))}
            </AITable>
            <details>
              <summary>远端客户端配置</summary>
              <label>
                客户端
                <select value={client} onChange={(e) => setClient(e.target.value as McpClientId)}>
                  {MCP_CLIENTS.map((c) => (
                    <option key={c.id} value={c.id}>
                      {c.label}
                    </option>
                  ))}
                </select>
              </label>
              <pre className="im-ai-code">{remoteConfig.value || '远端隧道验证后可生成配置。'}</pre>
              <button
                disabled={busy || !remoteConfig.value}
                onClick={() => void run(() => copy('remote', true))}
              >
                复制远端配置
              </button>
            </details>
            <div className="im-ai-actions">
              <button
                disabled={busy || !host.address || !host.username}
                onClick={() =>
                  void run(async () =>
                    (await verifyHost()) ? 'SSH 主机指纹已核验。' : '已取消核验。'
                  )
                }
              >
                核验 SSH 指纹
              </button>
              <button
                className="im-ai-primary"
                disabled={busy || !did || !host.address || !host.username || !draft.sdkDir}
                onClick={() => void run(saveChannel)}
              >
                保存通道并连接
              </button>
            </div>
          </section>
        </>
      )}
      <AIMessage>{notice || failures.join('；')}</AIMessage>
    </>
  );
}
