import { useEffect, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { invokeCommand } from '../../Platform/IPC';
import { formatErrorMessage } from '../../Utils';

export function DriverSetup({
  vendor,
  mode,
  installing,
  onInstall,
}: {
  vendor: string;
  mode: string;
  installing: boolean;
  onInstall: (infPath?: string) => void;
}) {
  const [info, setInfo] = useState<{ platform: string; ids: string[] }>();
  const [error, setError] = useState('');
  useEffect(() => {
    let current = true;
    setInfo(undefined);
    setError('');
    invokeCommand('imager_driver_info', { vendor })
      .then((value) => {
        if (current) setInfo(value);
      })
      .catch((e) => {
        if (current) setError(formatErrorMessage(e));
      });
    return () => {
      current = false;
    };
  }, [vendor]);
  if (!info)
    return (
      <p className="vi-help" role="status">
        {error || '正在读取驱动信息…'}
      </p>
    );
  const windows = info.platform === 'windows';
  const rockusb = windows && vendor === 'rockchip';
  async function selectDriver() {
    if (!rockusb) return onInstall();
    try {
      setError('');
      const path = await open({ title: '选择瑞芯微 Rockusb 驱动', filters: [{ name: 'Rockusb 驱动', extensions: ['inf'] }] });
      if (typeof path === 'string') onInstall(path);
    } catch (e) {
      setError(formatErrorMessage(e));
    }
  }
  if (!windows && info.platform !== 'linux')
    return <p className="vi-help">当前系统暂未提供驱动安装。</p>;
  return (
    <section className="vi-help" aria-label="下载口驱动安装">
      <strong>{rockusb ? '瑞芯微 Rockusb 驱动' : windows ? 'Windows 下载口驱动' : 'Linux USB 访问规则'}</strong>
      <p>
        {rockusb
          ? '解压瑞芯微官方 DriverAssistant 驱动包，选择与 Windows 架构匹配的 rockusb.inf。系统将验证驱动包并为匹配的下载口安装。'
          : windows
          ? '首次使用将从官方源下载 Zadig。打开后按 USB ID 选择开发板的下载口，在目标驱动中选择 WinUSB 并安装。'
          : '授权后，当前桌面用户可访问下列 USB 下载口。安装完成后请重新插拔开发板。'}
      </p>
      {windows && vendor === 'nxp' && (
        <p>
          i.MX6ULL 的 SDP 阶段使用系统 HID 驱动。仅在后续 Fastboot 接口缺少驱动时为该接口安装
          WinUSB。
        </p>
      )}
      <details open={info.ids.length <= 2}>
        <summary>{windows && vendor === 'nxp' ? 'Fastboot' : mode} · USB ID</summary>
        <code>{info.ids.join(' · ')}</code>
      </details>
      <button type="button" disabled={installing} onClick={() => void selectDriver()}>
        {installing ? '等待系统安装…' : rockusb ? '选择 Rockusb 驱动…' : windows ? '打开 WinUSB 安装器' : '安装 USB 访问规则'}
      </button>
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
