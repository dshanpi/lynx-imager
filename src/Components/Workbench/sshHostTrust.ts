import type { WorkbenchHost } from '../../Platform/IPC/Commands';

export function displaySshFingerprint(value: string): string {
  const text = value.trim();
  try {
    const bytes = /^([0-9a-f]{2}:){31}[0-9a-f]{2}$/i.test(text)
      ? String.fromCharCode(...text.split(':').map((part) => parseInt(part, 16)))
      : text.startsWith('SHA256:')
        ? atob(text.slice(7))
        : '';
    return bytes.length === 32 ? `SHA256:${btoa(bytes).replace(/=+$/, '')}` : '';
  } catch {
    return '';
  }
}

export function hostKeyChallenge(message: string): string | null {
  const match =
    message.match(/HOST_KEY_CHANGED:expected=[^,\s]*,actual=([^\s]+)/) ??
    message.match(/HOST_KEY_CONFIRM_REQUIRED:([^\s]+)/);
  return match ? displaySshFingerprint(match[1]) || null : null;
}

export function hostTrustPrompt(
  host: WorkbenchHost,
  observed: string,
  sharedChannels: number
): string {
  return [
    `请核验 SSH 服务器身份：${host.username}@${host.address}:${host.port}`,
    `已保存指纹：${displaySshFingerprint(host.hostFingerprint) || host.hostFingerprint || '尚未信任'}`,
    `本次指纹：${observed}`,
    `此主机配置关联 ${sharedChannels} 条 SDK 通道，更新后这些通道将使用同一指纹。`,
    '请通过服务器控制台或管理员提供的可信信息核对本次指纹。仅当确认它属于目标服务器时继续。',
    '继续后会重新验证该指纹并测试登录；校验或登录失败不会更新配置。',
  ].join('\n\n');
}

/** No authentication bypass: the second handshake must match the exact approved key. */
export async function testAndConfirmSshHost<R extends { fingerprint: string; trusted: boolean }>(
  host: WorkbenchHost,
  actions: {
    test: (host: WorkbenchHost) => Promise<R>;
    confirm: (message: string) => Promise<boolean>;
    describeError: (error: unknown) => string;
    isCurrent: () => boolean;
    sharedChannels: number;
  }
): Promise<{ host: WorkbenchHost; result: R } | null> {
  const requireCurrent = () => {
    if (!actions.isCurrent()) throw new Error('SSH_TARGET_CHANGED:主机或通道已切换，请重新核验');
  };
  let candidate = { ...host };
  let result: R;
  try {
    result = await actions.test(candidate);
  } catch (error) {
    const observed = hostKeyChallenge(actions.describeError(error));
    if (!observed) throw error;
    requireCurrent();
    if (!(await actions.confirm(hostTrustPrompt(candidate, observed, actions.sharedChannels))))
      return null;
    requireCurrent();
    candidate = { ...candidate, hostFingerprint: observed };
    // A new mismatch or authentication error propagates: no loop or automatic acceptance.
    result = await actions.test(candidate);
  }
  requireCurrent();
  if (
    !result.trusted ||
    !displaySshFingerprint(result.fingerprint) ||
    displaySshFingerprint(result.fingerprint) !== displaySshFingerprint(candidate.hostFingerprint)
  ) {
    throw new Error('SSH_TRUST_RESULT_INVALID:连接结果未匹配已确认指纹');
  }
  return { host: { ...candidate, hostFingerprint: result.fingerprint }, result };
}
