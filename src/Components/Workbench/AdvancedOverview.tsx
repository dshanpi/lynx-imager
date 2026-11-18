import { LynxIcon } from '../../CoreUI';
import type {
  LinuxDeviceOverview,
  WorkbenchDevice,
  WorkbenchTaskHistory,
} from '../../Platform/IPC/Commands';
import {
  formatLinuxBytes,
  formatUsbBinding,
  linuxVendorLabel,
  readAllwinnerFlashOptions,
  readK230Medium,
} from './linuxWorkbenchModel';

export interface MonitorSample {
  time: number;
  cpu: number | null;
  memory: number | null;
}

const TIME_RANGE = 5 * 60_000;
export function recordOverview(samples: MonitorSample[], sample: MonitorSample) {
  const last = samples[samples.length - 1];
  if (last && last.time >= sample.time) return samples;
  return [...samples.filter((item) => item.time >= sample.time - TIME_RANGE), sample].slice(-150);
}

function mediaLabel(device: WorkbenchDevice) {
  if (device.chipVendor === 'allwinner') {
    const route = readAllwinnerFlashOptions(device.flasherArgs).route;
    return {
      mainline_emmc: 'eMMC',
      mainline_nand: 'SPI NAND',
      mainline_spinor: 'SPI NOR',
      vendor_image: '由厂商镜像指定',
    }[route];
  }
  if (device.chipVendor === 'kendryte') return readK230Medium(device.flasherArgs);
  return '由烧录配置指定';
}

function timeLabel(time: number) {
  return new Date(time).toLocaleTimeString([], { hour12: false });
}

function usedSpace(total?: number | null, available?: number | null) {
  if (total == null || available == null || total <= 0 || available < 0 || available > total)
    return null;
  return formatLinuxBytes(total - available) + ' / ' + formatLinuxBytes(total);
}

function Monitor({ samples, offline }: { samples: MonitorSample[]; offline: boolean }) {
  const end = samples[samples.length - 1]?.time ?? 0;
  const start = end - TIME_RANGE;
  const x = (time: number) => 34 + ((time - start) / TIME_RANGE) * 522;
  const y = (v: number) => 114 - Math.max(0, Math.min(100, v));
  const series = (key: 'cpu' | 'memory') => {
    let previous = 0;
    const paths: string[] = [];
    const points = samples
      .filter((s) => s.time >= start)
      .flatMap((s) => {
        const value = s[key];
        if (value == null || !Number.isFinite(value)) {
          previous = 0;
          return [];
        }
        const xx = x(s.time),
          yy = y(value);
        paths.push(
          `${previous && s.time - previous <= 25_000 ? 'L' : 'M'} ${xx.toFixed(2)} ${yy.toFixed(2)}`
        );
        previous = s.time;
        return [{ time: s.time, x: xx, y: yy, value }];
      });
    return (
      <g className={key}>
        <path d={paths.join(' ')} />
        {points.map((p) => (
          <circle key={p.time} cx={p.x} cy={p.y} r="1.6">
            <title>
              {timeLabel(p.time)} · {p.value.toFixed(1)}%
            </title>
          </circle>
        ))}
      </g>
    );
  };
  return (
    <section className="adv-monitor">
      <header>
        <h3>运行监控</h3>
        <span className="adv-period">本次会话 · 近 5 分钟</span>
      </header>
      <div className="adv-key">
        <span>CPU 使用率</span>
        <span>内存使用率</span>
        <small>单位 %</small>
      </div>
      {samples.some((sample) => sample.cpu != null || sample.memory != null) ? (
        <>
          <svg
            role="img"
            aria-label={`CPU 与内存使用率，最近采样 ${timeLabel(end)}`}
            viewBox="0 0 564 143"
            preserveAspectRatio="none"
          >
            {[0, 25, 50, 75, 100].map((v) => (
              <g key={v}>
                <line className="grid" x1="34" y1={y(v)} x2="556" y2={y(v)} />
                <text x="1" y={y(v) + 4}>
                  {v}
                </text>
              </g>
            ))}
            {series('cpu')}
            {series('memory')}
            <text x="34" y="137">
              {timeLabel(start)}
            </text>
            <text x="556" y="137" textAnchor="end">
              {timeLabel(end)}
            </text>
          </svg>
          {offline && <small>采样暂停，显示上次已读取的数据。</small>}
        </>
      ) : (
        <p className="adv-graph-empty">等待当前设备的 CPU 或内存采样。</p>
      )}
    </section>
  );
}

interface Props {
  device: WorkbenchDevice;
  snapshot: LinuxDeviceOverview | null;
  samples: MonitorSample[];
  history: WorkbenchTaskHistory[];
  error: string;
  loading: boolean;
  serial: string;
  sdk: string;
  tunnel: string;
  usb: boolean;
  adb: boolean;
  serialOnline: boolean;
  onRefresh: () => void;
  onOpenRecords?: () => void;
}

export default function AdvancedOverview({
  device,
  snapshot,
  samples,
  history,
  error,
  loading,
  serial,
  sdk,
  tunnel,
  usb,
  adb,
  serialOnline,
  onRefresh,
  onOpenRecords,
}: Props) {
  const taskEvents = history
    .filter((t) => t.deviceId === device.id)
    .map((t) => ({
      id: t.taskId,
      time: (t.finishedAt ?? t.startedAt) * 1000,
      source: '设备任务',
      detail: [t.kind, t.status, t.error].filter(Boolean).join(' · '),
    }));
  if (snapshot)
    taskEvents.push({
      id: 'snapshot',
      time: snapshot.collectedAtMs,
      source: '运行监控',
      detail: `已通过 ${snapshot.transport === 'adb' ? 'ADB' : '串口'} 读取设备快照`,
    });
  const events = taskEvents.sort((a, b) => b.time - a.time).slice(0, 8);
  const metrics = [
    ['CPU 使用率', snapshot?.cpuUsagePct == null ? null : snapshot.cpuUsagePct.toFixed(1) + ' %'],
    ['已用内存', usedSpace(snapshot?.memoryTotalBytes, snapshot?.memoryAvailableBytes)],
    ['根分区', usedSpace(snapshot?.storageTotalBytes, snapshot?.storageAvailableBytes)],
    [
      'SoC 温度',
      snapshot?.temperatureCelsius == null ? null : snapshot.temperatureCelsius.toFixed(1) + ' °C',
    ],
  ];
  return (
    <div className="adv-overview">
      <div className="adv-page-heading">
        <h2>设备概览</h2>
        <span>{snapshot ? '采样于 ' + timeLabel(snapshot.collectedAtMs) : '尚未采样'}</span>
        <button disabled={loading} onClick={onRefresh}>
          <LynxIcon name="refresh" size={16} />
          {loading ? '读取中' : '刷新'}
        </button>
      </div>
      {error && (
        <p role="status" className="adv-warning">
          {error}
        </p>
      )}
      <dl className="adv-metrics">
        {metrics.map(([label, value]) => (
          <div key={label}>
            <dt>{label}</dt>
            <dd>{value ?? '未采集'}</dd>
          </div>
        ))}
      </dl>
      <div className="adv-columns">
        <Monitor samples={samples} offline={Boolean(error) || (!adb && !serialOnline)} />
        <section className="adv-props">
          <h3>设备属性</h3>
          <dl>
            {[
              ['芯片', snapshot?.cpuModel || linuxVendorLabel(device.chipVendor)],
              ['系统', snapshot?.osName || '未采集'],
              ['内核', snapshot?.kernel || '未采集'],
              ['存储介质', mediaLabel(device)],
              ['调试串口', serial || '未绑定'],
            ].map(([title, value]) => (
              <div key={title}>
                <dt>{title}</dt>
                <dd>{value}</dd>
              </div>
            ))}
          </dl>
        </section>
      </div>
      <section className="adv-events">
        <header>
          <h3>设备事件</h3>
          {onOpenRecords && (
            <button onClick={onOpenRecords}>
              <LynxIcon name="history" size={16} />
              任务记录
            </button>
          )}
        </header>
        <table>
          <thead>
            <tr>
              <th>时间</th>
              <th>来源</th>
              <th>事件</th>
            </tr>
          </thead>
          <tbody>
            {events.map((e) => (
              <tr key={e.id}>
                <td>{timeLabel(e.time)}</td>
                <td>{e.source}</td>
                <td>{e.detail}</td>
              </tr>
            ))}
          </tbody>
        </table>
        {!events.length && <p className="adv-empty-message">当前设备还没有已记录的事件。</p>}
      </section>
      <details className="adv-connections">
        <summary>接口与 SDK 配置</summary>
        <dl>
          {[
            ['串口', serialOnline ? serial : device.serialPort || '未绑定'],
            [
              '烧录接口',
              [formatUsbBinding(device.usbBinding), usb ? '已检测' : '未检测'].join(' · '),
            ],
            [
              'ADB',
              device.adbSerial ? device.adbSerial + (adb ? ' · 已检测' : ' · 未检测') : '未绑定',
            ],
            ['SDK 主机', sdk || '未配置'],
            ['远端 SDK 隧道', tunnel],
            ['IP 地址', snapshot?.ipAddresses.join(' · ') || '未采集'],
            ['架构', snapshot?.architecture || '未采集'],
          ].map(([key, value]) => (
            <div key={key}>
              <dt>{key}</dt>
              <dd>{value}</dd>
            </div>
          ))}
        </dl>
      </details>
    </div>
  );
}
