import { check } from '@tauri-apps/plugin-updater';
import { confirm, message } from '@tauri-apps/plugin-dialog';
import { relaunch } from '@tauri-apps/plugin-process';
import { invokeCommand } from '../Platform/IPC';
import type { DeltaUpdateFailure } from '../Platform/IPC/Commands';
import { formatErrorMessage } from '../Utils';

const UPDATE_CHECK_DELAY_MS = 5_000;
const SKIPPED_UPDATE_VERSION_KEY = 'lynx-skipped-update-version';
const FORCE_FULL_UPDATE_HEADER = 'X-Update-Mode';

export type UpdateStrategy = 'delta' | 'full';

export function skipUpdateVersion(version: string) {
  localStorage.setItem(SKIPPED_UPDATE_VERSION_KEY, version);
}

export function isUpdateVersionSkipped(version: string) {
  return localStorage.getItem(SKIPPED_UPDATE_VERSION_KEY) === version;
}

function updateDescription(
  version: string,
  body: string | undefined,
  strategy: UpdateStrategy,
  assetSize?: number
) {
  const notes = body?.trim() || '此版本未提供额外更新说明。';
  const size = assetSize ? `（${(assetSize / 1024 / 1024).toFixed(1)} MiB）` : '';
  const method = strategy === 'delta' ? `小版本增量包${size}` : `完整安装包${size}`;
  return `发现芯桥新版本 ${version}\n\n更新方式：${method}\n\n更新内容：\n${notes}\n\n是否立即下载并安装？安装完成后应用将自动重启。`;
}

async function getSystemProxy(): Promise<string | undefined> {
  try {
    return await invokeCommand('get_system_proxy');
  } catch (error) {
    console.warn('Unable to read the system proxy; checking for updates without it.', error);
    return undefined;
  }
}

/**
 * Check through the configured system proxy, then retry directly if that proxy
 * is stale or unavailable. The final error is preserved so callers never
 * confuse a network failure with "already up to date".
 */
export function getUpdateStrategy(update: { rawJson?: Record<string, unknown> }): UpdateStrategy {
  return update.rawJson?.strategy === 'delta' ? 'delta' : 'full';
}

export function getUpdateAssetSize(update: {
  rawJson?: Record<string, unknown>;
}): number | undefined {
  const value = update.rawJson?.asset_size;
  return typeof value === 'number' && Number.isSafeInteger(value) && value > 0 ? value : undefined;
}

export async function checkForUpdate(forceFull = false) {
  // The configured GitHub-only feed always serves signed full packages.
  // Static hosting cannot select a different artifact from X-Update-Mode.
  const proxy = await getSystemProxy();
  const headers = forceFull ? { [FORCE_FULL_UPDATE_HEADER]: 'full' } : undefined;
  if (!proxy) return check({ headers, timeout: 30_000 });

  try {
    return await check({ headers, proxy, timeout: 30_000 });
  } catch (proxyError) {
    console.warn('Update check through the system proxy failed; retrying directly.', proxyError);
    return check({ headers, timeout: 30_000 });
  }
}

async function offerFullFallback(failure: DeltaUpdateFailure): Promise<void> {
  const accepted = await confirm(
    `小版本更新未能完成：${failure.message}。\n\nLYNX 已恢复原版本，是否改用完整安装包？完整包仍会进行签名校验。`,
    {
      title: 'LYNX 更新恢复',
      kind: 'warning',
      okLabel: '下载完整包',
      cancelLabel: '暂不更新',
    }
  );
  if (!accepted) return;

  const update = await checkForUpdate(true);
  if (!update) {
    await message('服务器当前没有可用的完整更新包，请稍后重试。', {
      title: 'LYNX 更新恢复',
      kind: 'warning',
    });
    return;
  }
  if (getUpdateStrategy(update) !== 'full') {
    throw new Error('更新服务未返回强制完整包，已停止安装。');
  }
  await update.downloadAndInstall();
  await relaunch();
}

/**
 * Checks the public LYNX release feed once after startup.
 *
 * The signed update is downloaded and installed only after the user accepts the
 * version prompt. Tauri verifies the updater signature before installation.
 */
export function scheduleAutomaticUpdateCheck(): () => void {
  let cancelled = false;

  const timer = window.setTimeout(async () => {
    try {
      let recoveredFailure: DeltaUpdateFailure | null = null;
      try {
        recoveredFailure = await invokeCommand('consume_update_delta_failure');
      } catch (error) {
        console.error('Unable to consume the delta update recovery marker:', error);
      }
      if (cancelled) return;
      if (recoveredFailure) {
        await offerFullFallback(recoveredFailure);
        return;
      }

      const update = await checkForUpdate();
      if (cancelled || !update || isUpdateVersionSkipped(update.version)) return;

      const accepted = await confirm(
        updateDescription(
          update.version,
          update.body,
          getUpdateStrategy(update),
          getUpdateAssetSize(update)
        ),
        {
          title: 'LYNX 软件更新',
          kind: 'info',
          okLabel: '下载并安装',
          cancelLabel: '稍后',
        }
      );
      if (!accepted || cancelled) {
        if (cancelled) return;
        const skip = await confirm(
          `是否跳过 ${update.version}？\n\n选择“跳过此版本”后不再自动提示该版本；发布更高版本后仍会正常提示。`,
          {
            title: 'LYNX 软件更新',
            kind: 'info',
            okLabel: '跳过此版本',
            cancelLabel: '仅本次稍后',
          }
        );
        if (skip) skipUpdateVersion(update.version);
        return;
      }

      try {
        await update.downloadAndInstall();
      } catch (error) {
        if (getUpdateStrategy(update) !== 'delta') throw error;
        const fallback = await confirm(
          `小版本更新下载或签名校验失败：${formatErrorMessage(error)}\n\n是否改用签名完整安装包？`,
          {
            title: 'LYNX 软件更新',
            kind: 'warning',
            okLabel: '下载完整包',
            cancelLabel: '暂不更新',
          }
        );
        if (!fallback) return;
        const fullUpdate = await checkForUpdate(true);
        if (!fullUpdate || getUpdateStrategy(fullUpdate) !== 'full') {
          throw new Error('更新服务当前没有可用的完整安装包。');
        }
        await fullUpdate.downloadAndInstall();
      }
      if (!cancelled) {
        await relaunch();
      }
    } catch (error) {
      // Startup checks are deliberately non-blocking. Manual update checking in
      // Settings remains available and shows the result to the user.
      console.error('Automatic update check failed:', error);
    }
  }, UPDATE_CHECK_DELAY_MS);

  return () => {
    cancelled = true;
    window.clearTimeout(timer);
  };
}
