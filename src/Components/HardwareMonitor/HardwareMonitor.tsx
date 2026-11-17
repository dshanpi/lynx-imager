import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  invokeCommand,
  subscribeEvent,
  type HardwareMonitorChannel,
  type HardwareMonitorSnapshot,
} from '../../Platform/IPC';
import { formatErrorMessage } from '../../Utils';
import './HardwareMonitor.css';

interface Ch347Device {
  index: number;
  name: string;
  serialNumber: string;
}

const EMPTY: HardwareMonitorSnapshot = {
  state: 'stopped',
  deviceIndex: null,
  serialNumber: '',
  refreshMs: 1000,
  sequence: 0,
  updatedAtMs: 0,
  reconnectCount: 0,
  relays: 'board-controlled',
  channels: [0, 1, 2, 3].map((index) => ({
    channel: index + 1,
    address: 0x40 + index,
    relayGpio: 4 + index,
    status: 'waiting',
    busVoltageV: null,
    shuntVoltageV: null,
    currentA: null,
    powerW: null,
    errorCode: null,
    error: null,
  })),
  ai: {
    status: 'unconfigured',
    providerCount: 0,
    enabledProviderCount: 0,
    readyProviderCount: 0,
    activeRunCount: 0,
    waitingHumanCount: 0,
    lastTestStatus: 'untested',
  },
  errorCode: null,
  error: null,
};

const runningStates = new Set([
  'starting',
  'running',
  'paused',
  'waiting',
  'reconnecting',
  'stopping',
]);

function value(value: number | null, suffix: string, signed = false): string {
  if (value === null || !Number.isFinite(value)) return `-- ${suffix}`;
  return `${signed && value >= 0 ? '+' : ''}${value.toFixed(3)} ${suffix}`;
}

const ChannelCard: React.FC<{ channel: HardwareMonitorChannel }> = ({ channel }) => {
  const { t } = useTranslation();
  return (
    <article
      className={`hm-channel hm-channel-${channel.channel}`}
      data-testid={`hardware-channel-${channel.channel}`}
    >
      <div className="hm-channel-heading">
        <strong>CH{channel.channel}</strong>
        <span>
          GPIO{channel.relayGpio} · INA{channel.address.toString(16).toUpperCase()}
        </span>
      </div>
      {channel.status === 'ok' ? (
        <div className="hm-values">
          <div>
            <span>VOLTAGE</span>
            <b>{value(channel.busVoltageV, 'V')}</b>
          </div>
          <div>
            <span>CURRENT</span>
            <b>{value(channel.currentA, 'A', true)}</b>
          </div>
          <div>
            <span>POWER</span>
            <b>{value(channel.powerW, 'W', true)}</b>
          </div>
        </div>
      ) : (
        <div className="hm-channel-error">
          <b>{channel.errorCode ?? t('tools.hardwareMonitor.waiting', 'WAITING')}</b>
          <span>
            {channel.error ??
              t('tools.hardwareMonitor.waitingFirstSample', 'Waiting for first sample')}
          </span>
        </div>
      )}
    </article>
  );
};

export const HardwareMonitor: React.FC<{ isActive: boolean }> = ({ isActive }) => {
  const { t } = useTranslation();
  const [snapshot, setSnapshot] = useState<HardwareMonitorSnapshot>(EMPTY);
  const [devices, setDevices] = useState<Ch347Device[]>([]);
  const [selectedIndex, setSelectedIndex] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [pageError, setPageError] = useState('');

  const active = runningStates.has(snapshot.state);
  const selected = useMemo(
    () => devices.find((device) => device.index === selectedIndex) ?? null,
    [devices, selectedIndex]
  );

  const refreshDevices = useCallback(async () => {
    if (runningStates.has(snapshot.state)) return;
    try {
      const found = await invokeCommand('ch347_list_devices');
      setDevices(found);
      setSelectedIndex((current) =>
        found.some((device) => device.index === current) ? current : (found[0]?.index ?? null)
      );
      setPageError('');
    } catch (error) {
      setPageError(formatErrorMessage(error));
    }
  }, [snapshot.state]);

  useEffect(() => {
    let disposed = false;
    let unlistenUpdate: (() => void) | undefined;
    let unlistenDevice: (() => void) | undefined;
    void invokeCommand('hardware_monitor_status').then((current) => {
      if (!disposed) setSnapshot(current);
    });
    void subscribeEvent('hardware-monitor-update', (current) => {
      if (!disposed) setSnapshot(current);
    }).then((unlisten) => {
      unlistenUpdate = unlisten;
    });
    void subscribeEvent('ch347-device-changed', () => {
      if (!disposed) void refreshDevices();
    }).then((unlisten) => {
      unlistenDevice = unlisten;
    });
    return () => {
      disposed = true;
      unlistenUpdate?.();
      unlistenDevice?.();
    };
  }, [refreshDevices]);

  useEffect(() => {
    if (isActive && !active) void refreshDevices();
  }, [active, isActive, refreshDevices]);

  const start = async () => {
    if (!selected || busy) return;
    setBusy(true);
    setPageError('');
    try {
      setSnapshot(
        await invokeCommand('hardware_monitor_start', {
          request: { deviceIndex: selected.index, serialNumber: selected.serialNumber },
        })
      );
    } catch (error) {
      setPageError(formatErrorMessage(error));
    } finally {
      setBusy(false);
    }
  };

  const stop = async () => {
    if (busy) return;
    setBusy(true);
    try {
      setSnapshot(await invokeCommand('hardware_monitor_stop'));
    } catch (error) {
      setPageError(formatErrorMessage(error));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="hm-page" data-testid="hardware-monitor-page">
      <header className="hm-header">
        <div>
          <p className="hm-eyebrow">CH347F · ST7789VW · INA226</p>
          <h1>{t('tools.hardwareMonitor.name', '硬件监视')}</h1>
          <p>
            {t(
              'tools.hardwareMonitor.description',
              '四路电源、继电器、SPI 屏幕与 AI 通道原生实时监视'
            )}
          </p>
        </div>
        <div className={`hm-state hm-state-${snapshot.state}`}>
          <i />
          <span>{t(`tools.hardwareMonitor.states.${snapshot.state}`, snapshot.state)}</span>
          <small>
            #{snapshot.sequence} · {snapshot.refreshMs / 1000}s
          </small>
        </div>
      </header>

      <section className="hm-controls">
        <label>
          <span>{t('tools.hardwareMonitor.device', 'CH347 设备')}</span>
          <select
            data-testid="hardware-monitor-device"
            value={selectedIndex ?? ''}
            onChange={(event) => setSelectedIndex(Number(event.target.value))}
            disabled={active || busy}
          >
            {devices.length === 0 && (
              <option value="">{t('tools.hardwareMonitor.noDevices', 'No device found')}</option>
            )}
            {devices.map((device) => (
              <option key={device.index} value={device.index}>
                {device.name}
              </option>
            ))}
          </select>
        </label>
        {!active ? (
          <button data-testid="hardware-monitor-start" onClick={start} disabled={!selected || busy}>
            {busy
              ? t('tools.hardwareMonitor.startingAction', 'Starting…')
              : t('tools.hardwareMonitor.start', 'Start monitoring')}
          </button>
        ) : (
          <button
            data-testid="hardware-monitor-stop"
            className="danger"
            onClick={stop}
            disabled={busy || snapshot.state === 'stopping'}
          >
            {snapshot.state === 'stopping'
              ? t('tools.hardwareMonitor.stoppingAction', 'Stopping…')
              : t('tools.hardwareMonitor.stop', 'Stop monitoring')}
          </button>
        )}
        {!active && (
          <button className="secondary" onClick={() => void refreshDevices()} disabled={busy}>
            {t('tools.hardwareMonitor.refreshDevices', 'Refresh devices')}
          </button>
        )}
        <div className={`hm-relays hm-relays-${snapshot.relays}`}>
          {t('tools.hardwareMonitor.relayLabel', 'GPIO4–7: ')}
          {t(`tools.hardwareMonitor.relays.${snapshot.relays}`, snapshot.relays)}
        </div>
      </section>

      {(pageError || snapshot.error) && (
        <div className="hm-alert" role="alert">
          <b>
            {snapshot.errorCode ??
              t('tools.hardwareMonitor.genericError', 'HARDWARE_MONITOR_ERROR')}
          </b>
          <span>{pageError || snapshot.error}</span>
          {snapshot.state === 'reconnecting' && (
            <small>
              {t('tools.hardwareMonitor.reconnectAttempt', {
                count: snapshot.reconnectCount,
                defaultValue: 'Reconnect attempt {{count}}',
              })}
            </small>
          )}
        </div>
      )}

      <section className="hm-ai">
        <div className={`hm-ai-dot hm-ai-${snapshot.ai.status}`} />
        <div>
          <span>AI CHANNEL</span>
          <strong>{snapshot.ai.status.toUpperCase()}</strong>
        </div>
        <dl>
          <div>
            <dt>READY</dt>
            <dd>
              {snapshot.ai.readyProviderCount}/{snapshot.ai.enabledProviderCount}
            </dd>
          </div>
          <div>
            <dt>RUN</dt>
            <dd>{snapshot.ai.activeRunCount}</dd>
          </div>
          <div>
            <dt>WAIT</dt>
            <dd>{snapshot.ai.waitingHumanCount}</dd>
          </div>
          <div>
            <dt>TEST</dt>
            <dd>{snapshot.ai.lastTestStatus.toUpperCase()}</dd>
          </div>
        </dl>
      </section>

      <section className="hm-grid">
        {snapshot.channels.map((channel) => (
          <ChannelCard key={channel.channel} channel={channel} />
        ))}
      </section>

      <footer className="hm-footnote">
        15mΩ shunt · 100kHz I²C · RGB565 big-endian · 1Hz native worker
      </footer>
    </div>
  );
};
