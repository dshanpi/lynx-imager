export type McpClientId = 'codex' | 'zcode' | 'claude_code' | 'deepseek_harness' | 'custom';
export type McpClientShell = 'posix' | 'powershell';

export const MCP_CLIENTS: { id: McpClientId; label: string }[] = [
  { id: 'codex', label: 'Codex' },
  { id: 'zcode', label: 'ZCode' },
  { id: 'claude_code', label: 'Claude Code' },
  { id: 'deepseek_harness', label: 'DeepSeek' },
  { id: 'custom', label: '自定义 MCP' },
];

export function buildMcpClientConfig(
  client: McpClientId,
  endpoint: string,
  name: string,
  shell: McpClientShell
) {
  const kind = client === 'zcode' || client === 'custom' ? 'config' : 'command';
  const match = endpoint.match(/^http:\/\/127\.0\.0\.1:(\d+)\/mcp$/);
  if (!match || Number(match[1]) < 1 || Number(match[1]) > 65535 || !/^[a-zA-Z0-9_-]+$/.test(name))
    return { kind, value: '' };
  const config = { mcpServers: { [name]: { type: 'http', url: endpoint } } };
  if (kind === 'config') return { kind, value: JSON.stringify(config, null, 2) };
  if (client === 'claude_code') {
    return { kind, value: `claude --strict-mcp-config --mcp-config '${JSON.stringify(config)}'` };
  }
  if (client === 'deepseek_harness') {
    const patch = `- id: mcp-${name}\n  name: '@deepseek-ai/dsh-mcp-client'\n  config:\n    serverName: ${name}\n    transport: streamable-http\n    url: ${endpoint}\n`;
    // The YAML contains its own quoted package name; escape for each host shell.
    const quoted =
      shell === 'powershell' ? patch.replace(/'/g, "''") : patch.replace(/'/g, "'\"'\"'");
    return {
      kind,
      value:
        shell === 'powershell'
          ? `$mcp_tmp = [IO.Path]::GetTempFileName(); try { [IO.File]::WriteAllText($mcp_tmp, '${quoted}', [Text.UTF8Encoding]::new($false)); dsh web --patch $mcp_tmp } finally { Remove-Item -LiteralPath $mcp_tmp -ErrorAction SilentlyContinue }`
          : `(mcp_tmp=$(mktemp) && trap 'rm -f "$mcp_tmp"' EXIT HUP INT TERM && chmod 600 "$mcp_tmp" && printf '%s' '${quoted}' >"$mcp_tmp" && dsh web --patch "$mcp_tmp")`,
    };
  }
  return { kind, value: `codex -c 'mcp_servers={"${name}"={url="${endpoint}"}}'` };
}
