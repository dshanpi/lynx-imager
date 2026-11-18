import { useCallback, useEffect, useState } from 'react';
import { Alert20Regular } from '@fluentui/react-icons';
import { LynxIcon, type LynxIconName } from '../../CoreUI';
import { invokeCommand } from '../../Platform/IPC';
import type { WorkbenchDevice, WorkbenchHost, WorkbenchProject } from '../../Platform/IPC/Commands';
import { AIMessage, safeDetail } from './imagerAIControls';
import { ImagerAIChannels } from './ImagerAIChannels';
import { ImagerAIConfiguration } from './ImagerAIConfiguration';
import { ImagerAIApproval } from './ImagerAIApproval';
import './ImagerAIWorkspace.css';

export type ImagerAISection =
  'local' | 'remote' | 'services' | 'policy' | 'approval' | 'notifications';

const pages: { id: ImagerAISection; label: string; title: string; icon: LynxIconName }[] = [
  { id: 'local', label: '本机 MCP', title: '本机 MCP 通道', icon: 'plug' },
  { id: 'remote', label: '远端 SDK', title: '远端 SDK 通道', icon: 'link' },
  { id: 'services', label: 'AI 服务', title: 'AI 服务', icon: 'server' },
  { id: 'policy', label: '策略与安全', title: '策略与安全', icon: 'shield' },
  { id: 'approval', label: '请求确认', title: '请求确认', icon: 'check' },
  { id: 'notifications', label: '通知', title: '通知', icon: 'info' },
];

interface Props {
  isActive: boolean;
  initialSection?: ImagerAISection;
  onOpenRecords?: () => void;
  onOpenSettings?: () => void;
  onOpenFlash?: (deviceId: number) => void;
  onStatusChange?: (status: { target: string; task: string }) => void;
}

export function ImagerAIWorkspace({
  isActive,
  initialSection = 'local',
  onOpenRecords,
  onOpenSettings,
  onOpenFlash,
  onStatusChange,
}: Props) {
  const [section, setSection] = useState<ImagerAISection>(initialSection);
  const [data, setData] = useState<{
    devices: WorkbenchDevice[];
    projects: WorkbenchProject[];
    hosts: WorkbenchHost[];
  }>({ devices: [], projects: [], hosts: [] });
  const [deviceId, setDeviceId] = useState<number | null>(null);
  const [projectId, setProjectId] = useState<number | null>(null);
  const [issue, setIssue] = useState('');
  const [refresh, setRefresh] = useState(0);
  const [pending, setPending] = useState(false);
  const reload = useCallback(() => setRefresh((n) => n + 1), []);
  useEffect(() => {
    if (!isActive) return;
    let current = true;
    setIssue('');
    Promise.all([
      invokeCommand('workbench_list_devices'),
      invokeCommand('workbench_list_projects'),
      invokeCommand('workbench_list_hosts'),
    ])
      .then(([devices, projects, hosts]) => {
        if (!current) return;
        setData({ devices, projects, hosts });
        setDeviceId((id) => {
          const preferred = Number(localStorage.getItem('lynx.workbench.selectedDeviceId')) || id;
          return devices.find((d) => d.id === preferred)?.id ?? devices[0]?.id ?? null;
        });
      })
      .catch((error) => {
        if (current) {
          setData({ devices: [], projects: [], hosts: [] });
          setIssue(safeDetail(error));
        }
      });
    return () => {
      current = false;
    };
  }, [isActive, refresh]);

  const device = data.devices.find((d) => d.id === deviceId) ?? null;
  const channels = data.projects.filter((p) => device && p.deviceId === device.id);
  const project = channels.find((p) => p.id === projectId) ?? channels[0] ?? null;
  const page = pages.find((p) => p.id === section)!;
  useEffect(() => {
    if (isActive)
      onStatusChange?.({
        target: device?.name || '未选择设备',
        task: section === 'approval' && pending ? '等待操作员确认' : '连接 AI',
      });
  }, [device?.name, section, pending, isActive, onStatusChange]);

  return (
    <div className="im-ai-workspace">
      <aside className="im-ai-rail" aria-label="连接 AI 导航">
        <h2>连接 AI</h2>
        <nav>
          {[
            { title: 'MCP 通道', items: pages.slice(0, 2) },
            { title: '客户端与权限', items: pages.slice(2) },
          ].map(({ title, items }) => (
            <div key={title}>
              <div className="im-ai-group">
                <LynxIcon name="chevron-down" size={16} />
                {title}
              </div>
              {items.map((item) => (
                <button
                  key={item.id}
                  type="button"
                  aria-current={section === item.id ? 'page' : undefined}
                  onClick={() => setSection(item.id)}
                >
                  {item.id === 'notifications' ? <Alert20Regular aria-hidden="true" fontSize={16} /> : <LynxIcon name={item.icon} size={16} />}
                  {item.label}
                </button>
              ))}
            </div>
          ))}
        </nav>
        <div className="im-ai-rail-foot">
          {onOpenRecords && (
            <button onClick={onOpenRecords}>
              <LynxIcon name="history" size={16} />
              任务记录
            </button>
          )}
          {onOpenSettings && (
            <button onClick={onOpenSettings}>
              <LynxIcon name="settings" size={16} />
              应用设置
            </button>
          )}
        </div>
      </aside>
      <div className="im-ai-main">
        <div className="im-ai-context">
          <LynxIcon name="link" size={16} />
          <strong>连接 AI</strong>
          <span>/</span>
          <select
            aria-label="目标设备"
            value={device?.id ?? ''}
            onChange={(e) => {
              localStorage.setItem('lynx.workbench.selectedDeviceId', e.target.value);
              setDeviceId(Number(e.target.value));
              setProjectId(null);
            }}
          >
            {!data.devices.length && <option value="">未配置设备</option>}
            {data.devices.map((d) => (
              <option key={d.id} value={d.id!}>
                {d.name}
              </option>
            ))}
          </select>
          {channels.length > 1 && (
            <select
              aria-label="SDK 通道"
              value={project?.id ?? ''}
              onChange={(e) => setProjectId(Number(e.target.value))}
            >
              {channels.map((p) => (
                <option key={p.id} value={p.id!}>
                  {p.name}
                </option>
              ))}
            </select>
          )}
        </div>
        <section className="im-ai-body" aria-label={page.title} key={section}>
          <h1>{page.title}</h1>
          {issue && (
            <AIMessage>
              {issue} <button onClick={reload}>重新读取设备</button>
            </AIMessage>
          )}
          {isActive &&
            (section === 'local' || section === 'remote' ? (
              <ImagerAIChannels
                key={String(device?.id) + '-' + String(project?.id)}
                section={section}
                device={device}
                project={project}
                projects={data.projects}
                hosts={data.hosts}
                onSaved={reload}
                onOpenRemote={() => setSection('remote')}
              />
            ) : section === 'approval' ? (
              <ImagerAIApproval
                reloadDevice={reload}
                key={device?.id}
                device={device}
                onPending={setPending}
                onOpenFlash={onOpenFlash}
              />
            ) : (
              <ImagerAIConfiguration section={section} devices={data.devices} device={device} />
            ))}
        </section>
      </div>
    </div>
  );
}
