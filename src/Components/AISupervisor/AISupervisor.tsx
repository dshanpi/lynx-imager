import { FirmwareRuntimePanel } from '../Workbench/FirmwareBuildPanel';
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { confirm } from '@tauri-apps/plugin-dialog';
import { LynxIcon, type LynxIconName } from '../../CoreUI';
import { invokeCommand, subscribeEvent } from '../../Platform/IPC';
import { formatErrorMessage } from '../../Utils';
import type {
  AiProviderConfig,
  McuBoardConfig,
  SaveAiProviderRequest,
  SupervisorAuditRecord,
  SupervisorAutomaticTask,
  SupervisorHumanRequest,
  SupervisorPolicy,
  SupervisorRound,
  SupervisorRun,
  WorkbenchDevice,
  WorkbenchWorkflowStep,
} from '../../Platform/IPC/Commands';
import {
  PRESETS,
  ACTION_LABELS,
  defaultProvider,
  defaultPolicy,
  providerToDraft,
} from './supervisorConfiguration';
import { DeviceFlowEditor } from './DeviceFlowEditor';
import './AISupervisor.css';

export type AISupervisorSection = 'overview' | 'config' | 'automatic' | 'runtime';

const SECTION_META: Record<
  AISupervisorSection,
  { title: string; subtitle: string; icon: LynxIconName }
> = {
  overview: {
    title: 'AI 监督总览',
    subtitle: '设备任务、AI 恢复与人工介入的统一态势。',
    icon: 'brain',
  },
  config: {
    title: '配置中心',
    subtitle: '连接、策略、预算与动作权限一次配置。',
    icon: 'shield',
  },
  automatic: {
    title: '创建任务',
    subtitle: '为固定设备创建可审计、可验收的自动任务。',
    icon: 'clock',
  },
  runtime: {
    title: '运行中心',
    subtitle: '在一个任务上下文中处理运行、人工介入与审计证据。',
    icon: 'runtime',
  },
};

const formatTime = (timestamp: number | null) =>
  timestamp ? new Date(timestamp * 1000).toLocaleString() : '—';

const formatCost = (cost: number) => `$${cost.toFixed(cost < 0.01 ? 4 : 2)}`;

const statusLabel = (status: string) =>
  ({
    available: '可用',
    running: '运行中',
    paused: '已暂停',
    waiting_approval: '等待审批',
    waiting_human: '需要人工',
    success: '成功',
    failed: '失败',
    terminated: '已终止',
    cancelled: '已取消',
    manual: '手动模式',
    allowed: '允许',
    approval_required: '需要审批',
    blocked: '已阻止',
    pending: '等待验收',
    not_executed: '未执行',
  })[status] ??
  status ??
  '未知';

const statusTone = (status: string): StatusTone => {
  if (['available', 'success', 'allowed', 'verified'].includes(status)) return 'success';
  if (['running'].includes(status)) return 'info';
  if (['paused', 'waiting_approval', 'waiting_human', 'pending'].includes(status)) return 'warning';
  if (['failed', 'terminated', 'cancelled', 'blocked', 'denied'].includes(status)) return 'danger';
  return 'neutral';
};

type StatusTone = 'neutral' | 'info' | 'success' | 'warning' | 'danger';

const StatusBadge = ({ label, tone = 'neutral' }: { label: string; tone?: StatusTone }) => (
  <span className={`ais-status ais-status--${tone}`}>{label}</span>
);

const defaultAutomaticTask = (): SupervisorAutomaticTask => ({
  id: null,
  name: '新自动任务',
  enabled: false,
  triggerType: 'manual_api',
  trigger: {},
  boardId: '',
  taskTemplate: { kind: 'device_flow', version: 1, nodes: [], edges: [] },
  policyId: null,
  successConditions: [{ kind: 'status', equals: 'success' }],
  nextRunAt: null,
  lastRunAt: null,
});

const readableError = (error: unknown): string => {
  if (typeof error === 'string') return error;
  if (error instanceof Error) return error.message || error.name;
  if (error && typeof error === 'object') {
    const value = error as Record<string, unknown>;
    const message = typeof value.message === 'string' ? value.message : '';
    const cause = value.cause && value.cause !== error ? readableError(value.cause) : '';
    if (message && message !== '[object Object]')
      return cause && cause !== message ? `${message}: ${cause}` : message;
    if (cause) return cause;
    try {
      return JSON.stringify(error);
    } catch {
      return message || '未知错误';
    }
  }
  return formatErrorMessage(error);
};

const automaticTaskErrorMessage = (error: unknown) => {
  const message = readableError(error);
  if (
    message.includes('SUPERVISOR_AUTOMATIC_TASK_NAME_AND_BOARD_REQUIRED') ||
    message.includes('SUPERVISOR_DEVICE_FLOW_BOARD_REQUIRED')
  ) {
    return '任务尚未关联设备。请在右侧“任务设置 → 固定设备”选择已保存的单板；系统会自动同步到全部设备节点。';
  }
  if (message.includes('SUPERVISOR_DEVICE_FLOW_PRIMARY_BOARD_SCOPE_MISMATCH')) {
    return '任务固定设备与流程节点不一致。请重新选择一次“固定设备”，系统将统一节点作用域。';
  }
  if (message.includes('SUPERVISOR_AUTOMATIC_POLICY_REQUIRED')) {
    return '任务尚未选择监督策略。请在右侧任务设置中选择策略后再保存。';
  }
  if (message.includes('SUPERVISOR_AUTOMATIC_POLICY_SCOPE_MISMATCH')) {
    return '当前监督策略没有包含这块固定设备。请到“配置中心 → 策略与安全 → 板卡作用域”勾选该单板并保存策略。';
  }
  if (message.includes('SUPERVISOR_DEVICE_FLOW_NODE_CONFIG_OR_SECRET_INVALID')) {
    return '节点配置包含不允许保存的敏感字段或格式错误。密码只能写入系统密钥环，流程中只保存凭据引用。';
  }
  return message;
};

export const AISupervisor: React.FC<{ section: AISupervisorSection; isActive: boolean }> = ({
  section,
  isActive,
}) => {
  const [providers, setProviders] = useState<AiProviderConfig[]>([]);
  const [policies, setPolicies] = useState<SupervisorPolicy[]>([]);
  const [automaticTasks, setAutomaticTasks] = useState<SupervisorAutomaticTask[]>([]);
  const [firmwareRuntime, setFirmwareRuntime] = useState(false);
  const [workbenchDevices, setWorkbenchDevices] = useState<WorkbenchDevice[]>([]);
  const [mcuBoards, setMcuBoards] = useState<McuBoardConfig[]>([]);
  const [automaticDraft, setAutomaticDraft] =
    useState<SupervisorAutomaticTask>(defaultAutomaticTask);
  const [automaticEditorResetKey, setAutomaticEditorResetKey] = useState(0);
  const [webhookSecret, setWebhookSecret] = useState('');
  const [audit, setAudit] = useState<SupervisorAuditRecord[]>([]);
  const [runs, setRuns] = useState<SupervisorRun[]>([]);
  const [rounds, setRounds] = useState<SupervisorRound[]>([]);
  const [workflowSteps, setWorkflowSteps] = useState<WorkbenchWorkflowStep[]>([]);
  const [selectedRunId, setSelectedRunId] = useState('');
  const [humanRequests, setHumanRequests] = useState<SupervisorHumanRequest[]>([]);
  const [overview, setOverview] = useState({
    activeRunCount: 0,
    waitingHumanCount: 0,
    todayCallCount: 0,
    todayEstimatedCost: 0,
  });
  const [providerDraft, setProviderDraft] = useState<SaveAiProviderRequest>(defaultProvider);
  const [policyDraft, setPolicyDraft] = useState<SupervisorPolicy>(defaultPolicy);
  const [message, setMessage] = useState('');
  const [refreshError, setRefreshError] = useState('');
  const [loading, setLoading] = useState(true);
  const [lastRefreshAt, setLastRefreshAt] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [configTab, setConfigTab] = useState<'providers' | 'policy' | 'notifications'>('providers');

  useEffect(() => {
    setProviderDraft((current) => {
      const isLegacyUnsavedDefault =
        !current.id &&
        current.preset === 'openai' &&
        current.name === PRESETS.openai.name &&
        current.baseUrl === PRESETS.openai.baseUrl &&
        current.model === PRESETS.openai.model &&
        !current.apiKey;
      return isLegacyUnsavedDefault ? defaultProvider() : current;
    });
  }, []);

  const refresh = useCallback(async () => {
    setLoading(true);
    const [
      providerResult,
      policyResult,
      taskResult,
      auditResult,
      overviewResult,
      runResult,
      humanResult,
    ] = await Promise.allSettled([
      invokeCommand('supervisor_list_providers'),
      invokeCommand('supervisor_list_policies'),
      invokeCommand('supervisor_list_automatic_tasks'),
      invokeCommand('supervisor_list_audit', { limit: 100 }),
      invokeCommand('supervisor_overview'),
      invokeCommand('supervisor_list_runs', { limit: 100 }),
      invokeCommand('supervisor_list_human_requests', { waitingOnly: false }),
    ]);
    const failures = [
      providerResult,
      policyResult,
      taskResult,
      auditResult,
      overviewResult,
      runResult,
      humanResult,
    ].filter((result) => result.status === 'rejected');
    setRefreshError(
      failures.length
        ? `${failures.length} 项监督数据加载失败；已保留其余可用数据。${readableError((failures[0] as PromiseRejectedResult).reason)}`
        : ''
    );
    if (providerResult.status === 'fulfilled') setProviders(providerResult.value);
    if (policyResult.status === 'fulfilled') {
      setPolicies(policyResult.value);
      setPolicyDraft((current) => {
        if (current.id) return policyResult.value.find((item) => item.id === current.id) ?? current;
        return policyResult.value[0] ?? current;
      });
    }
    if (taskResult.status === 'fulfilled') setAutomaticTasks(taskResult.value);
    if (auditResult.status === 'fulfilled') setAudit(auditResult.value);
    if (overviewResult.status === 'fulfilled') setOverview(overviewResult.value);
    if (runResult.status === 'fulfilled') {
      setRuns(runResult.value);
      setSelectedRunId((current) =>
        current && runResult.value.some((run) => run.id === current)
          ? current
          : (runResult.value[0]?.id ?? '')
      );
    }
    if (humanResult.status === 'fulfilled') setHumanRequests(humanResult.value);
    setLastRefreshAt(Date.now());
    setLoading(false);
  }, []);

  useEffect(() => {
    if (!isActive) return;
    refresh().catch((error) => setMessage(readableError(error)));
  }, [isActive, refresh]);

  useEffect(() => {
    if (!isActive) return;
    let timer: number | undefined;
    const scheduleRefresh = () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(() => void refresh(), 120);
    };
    const subscriptions = [
      subscribeEvent('supervisor-automatic-trigger', scheduleRefresh),
      subscribeEvent('supervisor-automatic-result', scheduleRefresh),
      subscribeEvent('supervisor-run-update', scheduleRefresh),
    ];
    return () => {
      window.clearTimeout(timer);
      subscriptions.forEach((subscription) => {
        void subscription.then((unlisten) => unlisten());
      });
    };
  }, [isActive, refresh]);

  useEffect(() => {
    if (!isActive || section !== 'runtime' || !selectedRunId) {
      if (!selectedRunId) setRounds([]);
      return;
    }
    invokeCommand('supervisor_list_rounds', { runId: selectedRunId, limit: 100 })
      .then(setRounds)
      .catch((error) => {
        setRounds([]);
        setRefreshError(`监督回合加载失败：${readableError(error)}`);
      });
  }, [isActive, section, selectedRunId]);

  useEffect(() => {
    if (!isActive || section !== 'runtime' || !selectedRunId) {
      setWorkflowSteps([]);
      return;
    }
    const selectedRun = runs.find((run) => run.id === selectedRunId);
    if (!selectedRun) {
      setWorkflowSteps([]);
      return;
    }
    let disposed = false;
    const loadSteps = () =>
      invokeCommand('workbench_list_workflow_steps', { taskId: selectedRun.taskId })
        .then((steps) => {
          if (!disposed) setWorkflowSteps(steps);
        })
        .catch((error) => {
          if (!disposed) setRefreshError(`任务步骤加载失败：${readableError(error)}`);
        });
    void loadSteps();
    const timer = window.setInterval(loadSteps, 500);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, [isActive, runs, section, selectedRunId]);

  useEffect(() => {
    if (!isActive || (section !== 'automatic' && !(section === 'config' && configTab === 'policy')))
      return;
    Promise.allSettled([
      invokeCommand('workbench_list_devices'),
      invokeCommand('mcu_list_boards'),
    ]).then(([linuxResult, mcuResult]) => {
      if (linuxResult.status === 'fulfilled') setWorkbenchDevices(linuxResult.value);
      if (mcuResult.status === 'fulfilled') setMcuBoards(mcuResult.value);
      const failure = [linuxResult, mcuResult].find((result) => result.status === 'rejected');
      if (failure?.status === 'rejected')
        setRefreshError(`部分设备资源加载失败：${readableError(failure.reason)}`);
    });
  }, [configTab, isActive, section]);

  const saveProvider = async () => {
    setBusy(true);
    setMessage('');
    try {
      const saved = await invokeCommand('supervisor_save_provider', { request: providerDraft });
      setProviderDraft(providerToDraft(saved));
      await refresh();
      setMessage('连接配置已保存；API Key 已写入系统密钥环。');
    } catch (error) {
      setMessage(readableError(error));
    } finally {
      setBusy(false);
    }
  };

  const testProvider = async () => {
    if (!providerDraft.id) return;
    setBusy(true);
    try {
      const result = await invokeCommand('supervisor_test_provider', {
        providerId: providerDraft.id,
      });
      setMessage(`${result.message}（${result.latencyMs} ms）`);
      await refresh();
    } catch (error) {
      setMessage(readableError(error));
    } finally {
      setBusy(false);
    }
  };

  const savePolicy = async (draft = policyDraft) => {
    setBusy(true);
    try {
      const saved = await invokeCommand('supervisor_save_policy', { policy: draft });
      setPolicyDraft(saved);
      await refresh();
      setMessage('监督策略已保存。');
    } catch (error) {
      setMessage(readableError(error));
    } finally {
      setBusy(false);
    }
  };

  const defaultConnection = providers.find((provider) => provider.isDefault) ?? providers[0];
  const connectionReady = Boolean(
    defaultConnection?.enabled &&
    defaultConnection.hasApiKey &&
    defaultConnection.lastTestStatus === 'available'
  );
  const statusCounts = useMemo(
    () => ({
      ready: providers.filter((provider) => provider.enabled && provider.hasApiKey).length,
      policies: policies.filter((policy) => policy.enabled).length,
    }),
    [policies, providers]
  );

  return (
    <div className="ais-root">
      <header className="ais-header">
        <div>
          <h2>
            <LynxIcon name={SECTION_META[section].icon} size={16} /> {SECTION_META[section].title}
          </h2>
          <p>{SECTION_META[section].subtitle}</p>
        </div>
        <div className="ais-header-actions">
          <span className={`ais-health ${connectionReady ? 'ready' : ''}`}>
            <LynxIcon name={connectionReady ? 'check-circle' : 'error'} size={16} />
            {connectionReady ? '监督服务可用' : '连接尚未就绪'}
          </span>
          <button className="ais-refresh" disabled={loading} onClick={() => void refresh()}>
            <LynxIcon name={'refresh'} size={16} /> {loading ? '加载中' : '刷新'}
          </button>
        </div>
      </header>

      {message && <div className="ais-message">{message}</div>}
      {refreshError && (
        <div className="ais-message ais-message--error">
          <LynxIcon name={'error'} size={16} /> {refreshError}
        </div>
      )}

      {loading && lastRefreshAt === null ? (
        <EmptyState
          icon={'refresh'}
          title="正在加载监督数据"
          detail="LYNX 正在读取本机配置、运行记录和脱敏审计，不会探测或操作硬件。"
        />
      ) : (
        <>
          {section === 'overview' && (
            <Overview
              providers={providers}
              policies={policies}
              ready={statusCounts.ready}
              overview={overview}
              runs={runs}
              audit={audit}
              automaticTasks={automaticTasks}
              lastRefreshAt={lastRefreshAt}
            />
          )}
          {section === 'config' && (
            <ConfigurationCenter
              activeTab={configTab}
              onTabChange={setConfigTab}
              providers={providers}
              providerDraft={providerDraft}
              setProviderDraft={setProviderDraft}
              policies={policies}
              policyDraft={policyDraft}
              setPolicyDraft={setPolicyDraft}
              devices={workbenchDevices}
              mcuBoards={mcuBoards}
              busy={busy}
              onSaveProvider={saveProvider}
              onTestProvider={testProvider}
              onNewProvider={() => setProviderDraft(defaultProvider())}
              onSelectProvider={(provider) => setProviderDraft(providerToDraft(provider))}
              onDeleteProvider={async (id) => {
                await invokeCommand('supervisor_delete_provider', { providerId: id });
                setProviderDraft(defaultProvider());
                await refresh();
              }}
              onSavePolicy={() => savePolicy()}
              onNewPolicy={() => setPolicyDraft(defaultPolicy())}
            />
          )}
          {section === 'automatic' && (
            <DeviceFlowEditor
              tasks={automaticTasks}
              policies={policies}
              devices={workbenchDevices}
              mcuBoards={mcuBoards}
              resetKey={automaticEditorResetKey}
              draft={automaticDraft}
              setDraft={setAutomaticDraft}
              webhookSecret={webhookSecret}
              setWebhookSecret={setWebhookSecret}
              busy={busy}
              onNew={() => {
                setAutomaticDraft(defaultAutomaticTask());
                setWebhookSecret('');
                setAutomaticEditorResetKey((current) => current + 1);
              }}
              onSave={async (task) => {
                setBusy(true);
                try {
                  const saved = await invokeCommand('supervisor_save_automatic_task', {
                    task,
                    webhookSecret: webhookSecret || null,
                  });
                  setAutomaticDraft(saved);
                  setWebhookSecret('');
                  await refresh();
                  setMessage('自动任务配置已保存。');
                  return saved;
                } catch (error) {
                  const message = automaticTaskErrorMessage(error);
                  setMessage(message);
                  throw new Error(message);
                } finally {
                  setBusy(false);
                }
              }}
              onDelete={async (id) => {
                await invokeCommand('supervisor_delete_automatic_task', { taskId: id });
                setAutomaticDraft(defaultAutomaticTask());
                setWebhookSecret('');
                await refresh();
              }}
              onTrigger={async (id) => {
                try {
                  const nodes = Array.isArray(automaticDraft.taskTemplate.nodes)
                    ? automaticDraft.taskTemplate.nodes
                    : [];
                  const highRiskTypes = new Set([
                    'serial.send',
                    'serial.command',
                    'adb.command',
                    'power.action',
                    'flash.workbench',
                    'ssh.build',
                    'mcu.operation',
                  ]);
                  const isDeviceFlow = automaticDraft.taskTemplate.kind === 'device_flow';
                  const requiresApproval =
                    !isDeviceFlow ||
                    nodes.some(
                      (node) =>
                        typeof node === 'object' &&
                        node !== null &&
                        highRiskTypes.has(String((node as { type?: unknown }).type ?? ''))
                    );
                  if (
                    requiresApproval &&
                    !((import.meta as ImportMeta & { env?: Record<string, string> }).env
                      ?.VITE_LYNX_AI_RUNTIME_GATE === '1'
                      ? window.confirm(
                          '此任务包含烧录、命令、电源或其他高风险设备动作。确认在当前固定板卡上立即执行？'
                        )
                      : await confirm(
                          '此任务包含烧录、命令、电源或其他高风险设备动作。确认在当前固定板卡上立即执行？',
                          { title: '确认执行高风险任务', kind: 'warning' }
                        ))
                  ) {
                    return;
                  }
                  const taskId = await invokeCommand('supervisor_trigger_automatic_task', {
                    taskId: id,
                    operatorApproved: requiresApproval,
                  });
                  setMessage(`自动任务已启动：${taskId}`);
                  await refresh();
                } catch (error) {
                  setMessage(readableError(error));
                }
              }}
            />
          )}
          {section === 'runtime' && (
            <>
              <nav className="fw-page-tabs">
                <button onClick={() => setFirmwareRuntime(false)}>监督任务</button>
                <button onClick={() => setFirmwareRuntime(true)}>固件任务与发布</button>
              </nav>
              {firmwareRuntime ? (
                <FirmwareRuntimePanel boards={workbenchDevices} />
              ) : (
                <RunCenter
                  runs={runs}
                  rounds={rounds}
                  workflowSteps={workflowSteps}
                  audit={audit}
                  automaticTasks={automaticTasks}
                  providers={providers}
                  humanRequests={humanRequests}
                  selectedRunId={selectedRunId}
                  onSelectRun={setSelectedRunId}
                  onApprove={async (run) => {
                    try {
                      const result = await invokeCommand('supervisor_execute_action', {
                        runId: run.id,
                        roundIndex: run.attemptCount,
                        approved: true,
                      });
                      setMessage(`已批准并执行：${result.action}`);
                    } catch (error) {
                      const detail = readableError(error);
                      setMessage(
                        detail.includes('RETRY_REQUIRES_OPERATOR_WORKFLOW')
                          ? '烧录或原步骤重试必须回到对应工作台，由操作员确认同一产物后启动。'
                          : detail
                      );
                    }
                    await refresh();
                  }}
                  onControl={async (runId, action) => {
                    try {
                      await invokeCommand('supervisor_control_run', { runId, action });
                      setMessage('监督状态已更新。');
                      await refresh();
                    } catch (error) {
                      setMessage(`监督状态更新失败：${readableError(error)}`);
                    }
                  }}
                  onResolve={async (requestId, resolution) => {
                    try {
                      await invokeCommand('supervisor_resolve_human_request', {
                        requestId,
                        resolution,
                      });
                      setMessage('人工处置结果已保存。');
                      await refresh();
                    } catch (error) {
                      setMessage(`人工处置失败：${readableError(error)}`);
                    }
                  }}
                />
              )}
            </>
          )}
        </>
      )}
    </div>
  );
};

const Overview: React.FC<{
  providers: AiProviderConfig[];
  policies: SupervisorPolicy[];
  ready: number;
  runs: SupervisorRun[];
  audit: SupervisorAuditRecord[];
  automaticTasks: SupervisorAutomaticTask[];
  lastRefreshAt: number | null;
  overview: {
    activeRunCount: number;
    waitingHumanCount: number;
    todayCallCount: number;
    todayEstimatedCost: number;
  };
}> = ({ providers, policies, ready, overview, runs, audit, automaticTasks, lastRefreshAt }) => {
  const defaultConnection = providers.find((item) => item.isDefault) ?? providers[0];
  const enabledPolicy = policies.find((item) => item.enabled);
  const activeRuns = runs.filter((run) =>
    ['running', 'paused', 'waiting_approval', 'waiting_human'].includes(run.status)
  );
  const attentionRun =
    runs.find((run) => run.status === 'waiting_human') ??
    runs.find((run) => run.status === 'waiting_approval') ??
    runs.find((run) => run.status === 'failed');
  return (
    <div className="ais-overview">
      <div className="ais-kpis" aria-label="今日监督指标">
        <Metric icon={'agent'} label="今日 AI 调用" value={String(overview.todayCallCount)} />
        <Metric icon={'runtime'} label="运行中监督" value={String(overview.activeRunCount)} />
        <Metric
          icon={'person-settings'}
          label="等待人工"
          value={String(overview.waitingHumanCount)}
          tone={overview.waitingHumanCount ? 'warning' : 'success'}
        />
        <Metric
          icon={'money'}
          label="今日估算费用"
          value={formatCost(overview.todayEstimatedCost)}
        />
      </div>

      <div className="ais-overview-layout">
        <section className={`ais-card ais-attention ${attentionRun ? 'has-item' : ''}`}>
          <div className="ais-card-title">
            <div>
              <h3>需要关注</h3>
              <p>只突出需要审批、人工操作或失败复核的真实任务。</p>
            </div>
            <StatusBadge
              label={attentionRun ? statusLabel(attentionRun.status) : '当前无待办'}
              tone={attentionRun ? statusTone(attentionRun.status) : 'success'}
            />
          </div>
          {attentionRun ? (
            <div className="ais-attention-task">
              <LynxIcon name={'error'} size={16} />
              <div>
                <strong>{attentionRun.taskId}</strong>
                <span>{attentionRun.boardId}</span>
                <p>{attentionRun.finalReason || '等待操作员查看当前运行证据并处理。'}</p>
                <small>运行 {attentionRun.id}</small>
              </div>
            </div>
          ) : (
            <p className="ais-empty-copy">所有监督运行都不需要立即处理。</p>
          )}
        </section>
        <section className="ais-card ais-overview-runs">
          <div className="ais-card-title">
            <div>
              <h3>监督运行态势</h3>
              <p>只展示真实任务记录；成功必须来自确定性验收。</p>
            </div>
            <StatusBadge
              label={activeRuns.length ? `${activeRuns.length} 个活动任务` : '当前空闲'}
              tone={activeRuns.length ? 'info' : 'neutral'}
            />
          </div>
          {runs.length === 0 ? (
            <EmptyState
              compact
              icon={'runtime'}
              title="还没有监督运行"
              detail="自动任务启动后立即进入监督总览；失败时继续在同一运行记录中展示 AI 恢复与人工介入。"
            />
          ) : (
            <div className="ais-compact-table">
              <div className="ais-table-head">
                <span>任务 / 板卡</span>
                <span>状态</span>
                <span>尝试</span>
                <span>费用</span>
              </div>
              {runs.slice(0, 6).map((run) => (
                <div key={run.id}>
                  <span>
                    <strong>{run.taskId}</strong>
                    <small>{run.boardId}</small>
                  </span>
                  <StatusBadge label={statusLabel(run.status)} tone={statusTone(run.status)} />
                  <span>{run.attemptCount}</span>
                  <span>{formatCost(run.estimatedCost)}</span>
                </div>
              ))}
            </div>
          )}
        </section>

        <aside className="ais-overview-side">
          <section className="ais-card ais-readiness">
            <div className="ais-card-title">
              <div>
                <h3>接入就绪度</h3>
                <p>开始无人值守监督前必须全部满足。</p>
              </div>
              <strong>
                {
                  [ready > 0, Boolean(enabledPolicy), enabledPolicy?.boardScope.length].filter(
                    Boolean
                  ).length
                }
                /3
              </strong>
            </div>
            <ReadinessItem
              ready={ready > 0}
              label="AI 连接已实测"
              detail={
                defaultConnection
                  ? `${defaultConnection.name} · ${defaultConnection.model}`
                  : '请先创建 OpenAI 兼容连接'
              }
            />
            <ReadinessItem
              ready={Boolean(enabledPolicy)}
              label="监督策略已启用"
              detail={enabledPolicy?.name ?? '当前没有启用策略'}
            />
            <ReadinessItem
              ready={Boolean(enabledPolicy?.boardScope.length)}
              label="策略已绑定板卡"
              detail={enabledPolicy?.boardScope.join('、') || '作用域为空，不会自动介入任何板卡'}
            />
          </section>

          <section className="ais-card ais-budget-summary">
            <div className="ais-card-title">
              <div>
                <h3>默认恢复预算</h3>
                <p>{enabledPolicy?.name ?? '尚无启用策略'}</p>
              </div>
            </div>
            {enabledPolicy ? (
              <div className="ais-budget-facts">
                <span>
                  <strong>{enabledPolicy.maxAttempts}</strong>AI 尝试
                </span>
                <span>
                  <strong>{Math.round(enabledPolicy.maxDurationSeconds / 60)}</strong>分钟
                </span>
                <span>
                  <strong>{enabledPolicy.maxPowerCycles}</strong>电源循环
                </span>
                <span>
                  <strong>{enabledPolicy.maxFlashRetries}</strong>烧录重试
                </span>
              </div>
            ) : (
              <p className="ais-empty-copy">没有策略时，任务失败不会启动 AI 自主恢复。</p>
            )}
          </section>
        </aside>

        <section className="ais-card ais-recent-audit">
          <div className="ais-card-title">
            <div>
              <h3>最近 AI 调用</h3>
              <p>输入已脱敏；模型输出仍需策略和验收器复核。</p>
            </div>
            <span className="ais-last-refresh">
              刷新于 {lastRefreshAt ? new Date(lastRefreshAt).toLocaleTimeString() : '—'}
            </span>
          </div>
          {audit.length === 0 ? (
            <EmptyState
              compact
              icon={'chart'}
              title="今天还没有 AI 调用"
              detail="监督器不会为了健康检查而消耗模型调用。"
            />
          ) : (
            <div className="ais-audit-strip">
              {audit.slice(0, 5).map((record) => (
                <div key={record.id}>
                  <span>{new Date(record.createdAt * 1000).toLocaleTimeString()}</span>
                  <strong>{record.boardId || '无板卡作用域'}</strong>
                  <StatusBadge
                    label={statusLabel(record.result)}
                    tone={statusTone(record.result)}
                  />
                  <span>{record.inputTokens + record.outputTokens} tokens</span>
                  <span>{formatCost(record.estimatedCost)}</span>
                </div>
              ))}
            </div>
          )}
        </section>

        <section className="ais-card ais-automation-summary">
          <div className="ais-card-title">
            <div>
              <h3>任务概况</h3>
              <p>计划、Webhook、设备事件与手动/API 触发。</p>
            </div>
            <StatusBadge
              label={`${automaticTasks.filter((task) => task.enabled).length}/${automaticTasks.length} 已启用`}
              tone={automaticTasks.some((task) => task.enabled) ? 'success' : 'neutral'}
            />
          </div>
          <p className="ais-safety-line">
            <LynxIcon name={'shield'} size={16} /> API Key 仅存系统密钥环 ·
            动作必须通过白名单、board scope 和预算 · AI 不能自行确认硬件成功
          </p>
        </section>
      </div>
    </div>
  );
};

const Metric = ({
  icon,
  label,
  value,
  tone = 'info',
}: {
  icon: LynxIconName;
  label: string;
  value: string;
  tone?: StatusTone;
}) => (
  <div className={`ais-metric ais-metric--${tone}`}>
    <LynxIcon name={icon} size={16} />
    <span>
      <strong>{value}</strong>
      <small>{label}</small>
    </span>
  </div>
);

const ReadinessItem = ({
  ready,
  label,
  detail,
}: {
  ready: boolean;
  label: string;
  detail: string;
}) => (
  <div className="ais-readiness-item">
    <LynxIcon name={ready ? 'check-circle' : 'error'} size={16} />
    <span>
      <strong>{label}</strong>
      <small>{detail}</small>
    </span>
    <StatusBadge label={ready ? '就绪' : '待配置'} tone={ready ? 'success' : 'warning'} />
  </div>
);

type ConfigCenterTab = 'providers' | 'policy' | 'notifications';

const ConfigurationCenter: React.FC<{
  activeTab: ConfigCenterTab;
  onTabChange: (tab: ConfigCenterTab) => void;
  providers: AiProviderConfig[];
  providerDraft: SaveAiProviderRequest;
  setProviderDraft: React.Dispatch<React.SetStateAction<SaveAiProviderRequest>>;
  policies: SupervisorPolicy[];
  policyDraft: SupervisorPolicy;
  setPolicyDraft: React.Dispatch<React.SetStateAction<SupervisorPolicy>>;
  devices: WorkbenchDevice[];
  mcuBoards: McuBoardConfig[];
  busy: boolean;
  onSaveProvider: () => Promise<void>;
  onTestProvider: () => Promise<void>;
  onNewProvider: () => void;
  onSelectProvider: (provider: AiProviderConfig) => void;
  onDeleteProvider: (id: number) => Promise<void>;
  onSavePolicy: () => Promise<void>;
  onNewPolicy: () => void;
}> = (props) => (
  <div className="ais-config-center">
    <nav className="ais-section-tabs" aria-label="配置中心分类">
      <button
        data-testid="ai-config-tab-providers"
        className={props.activeTab === 'providers' ? 'active' : ''}
        onClick={() => props.onTabChange('providers')}
      >
        <LynxIcon name={'cloud'} size={16} /> AI 服务
      </button>
      <button
        data-testid="ai-config-tab-policy"
        className={props.activeTab === 'policy' ? 'active' : ''}
        onClick={() => props.onTabChange('policy')}
      >
        <LynxIcon name={'shield'} size={16} /> 策略与安全
      </button>
      <button
        data-testid="ai-config-tab-notifications"
        className={props.activeTab === 'notifications' ? 'active' : ''}
        onClick={() => props.onTabChange('notifications')}
      >
        <LynxIcon name={'person-settings'} size={16} /> 通知
      </button>
    </nav>
    {props.activeTab === 'providers' && (
      <ProviderEditor
        providers={props.providers}
        draft={props.providerDraft}
        setDraft={props.setProviderDraft}
        busy={props.busy}
        onSave={props.onSaveProvider}
        onTest={props.onTestProvider}
        onNew={props.onNewProvider}
        onSelect={props.onSelectProvider}
        onDelete={props.onDeleteProvider}
      />
    )}
    {props.activeTab === 'policy' && (
      <PolicyEditor
        providers={props.providers}
        policies={props.policies}
        draft={props.policyDraft}
        setDraft={props.setPolicyDraft}
        devices={props.devices}
        mcuBoards={props.mcuBoards}
        busy={props.busy}
        onSave={props.onSavePolicy}
        onNew={props.onNewPolicy}
      />
    )}
    {props.activeTab === 'notifications' && (
      <NotificationEditor
        draft={props.policyDraft}
        setDraft={props.setPolicyDraft}
        busy={props.busy}
        onSave={props.onSavePolicy}
      />
    )}
  </div>
);

const EmptyState = ({
  icon,
  title,
  detail,
  compact = false,
}: {
  icon: LynxIconName;
  title: string;
  detail: string;
  compact?: boolean;
}) => (
  <div className={`ais-empty-state ${compact ? 'ais-empty-state--compact' : ''}`}>
    <LynxIcon name={icon} size={16} />
    <div>
      <strong>{title}</strong>
      <p>{detail}</p>
    </div>
  </div>
);

interface ProviderEditorProps {
  providers: AiProviderConfig[];
  draft: SaveAiProviderRequest;
  setDraft: React.Dispatch<React.SetStateAction<SaveAiProviderRequest>>;
  busy: boolean;
  onSave: () => void;
  onTest: () => void;
  onNew: () => void;
  onSelect: (provider: AiProviderConfig) => void;
  onDelete: (id: number) => Promise<void>;
}

const ProviderEditor: React.FC<ProviderEditorProps> = (props) => {
  const update = <K extends keyof SaveAiProviderRequest>(key: K, value: SaveAiProviderRequest[K]) =>
    props.setDraft((current) => ({ ...current, [key]: value }));
  const selected = props.providers.find((provider) => provider.id === props.draft.id);
  const connectionReady = Boolean(
    props.draft.baseUrl && props.draft.model && (props.draft.apiKey || selected?.hasApiKey)
  );
  return (
    <div className="ais-provider-workspace">
      <aside className="ais-card ais-list ais-provider-list">
        <div className="ais-card-title">
          <div>
            <span className="ais-section-kicker">CONNECTIONS</span>
            <h3>服务连接</h3>
          </div>
          <button onClick={props.onNew} aria-label="新建 AI 服务连接" title="新建连接">
            <LynxIcon name={'add'} size={16} />
          </button>
        </div>
        {props.providers.map((provider) => (
          <button
            key={provider.id}
            className={provider.id === props.draft.id ? 'selected' : ''}
            onClick={() => props.onSelect(provider)}
          >
            <span className="ais-provider-logo">
              <LynxIcon name={'cloud'} size={16} />
            </span>
            <span>
              <strong>{provider.name}</strong>
              <small>{provider.model}</small>
            </span>
            <i className={provider.hasApiKey ? 'online' : ''} />
          </button>
        ))}
        {props.providers.length === 0 && <p className="ais-empty-copy">尚未配置兼容连接。</p>}
      </aside>
      <section className="ais-card ais-form ais-provider-editor">
        <header className="ais-provider-hero">
          <span className="ais-provider-hero-icon">
            <LynxIcon name={'cloud'} size={16} />
          </span>
          <div>
            <span className="ais-section-kicker">AI PROVIDER</span>
            <h3>{props.draft.name || '新建服务连接'}</h3>
            <p>{props.draft.model || '选择服务预设并填写运行模型'}</p>
          </div>
          <StatusBadge
            label={connectionReady ? '配置就绪' : '待完善'}
            tone={connectionReady ? 'success' : 'warning'}
          />
        </header>

        <div className="ais-provider-section">
          <div className="ais-provider-section-heading">
            <span>01</span>
            <div>
              <h4>连接信息</h4>
              <p>选择兼容服务并确认 API 接入地址。</p>
            </div>
          </div>
          <div className="ais-provider-fields ais-provider-fields--identity">
            <label>
              服务预设
              <select
                value={props.draft.preset}
                onChange={(event) => {
                  const preset = event.target.value;
                  const defaults = PRESETS[preset] ?? PRESETS.custom;
                  props.setDraft((current) => ({ ...current, preset, ...defaults }));
                }}
              >
                <option value="openai">OpenAI</option>
                <option value="deepseek">DeepSeek</option>
                <option value="codingplan">Coding Plan</option>
                <option value="custom">自定义兼容接口</option>
              </select>
            </label>
            <label>
              连接名称
              <input value={props.draft.name} onChange={(e) => update('name', e.target.value)} />
            </label>
            <label className="ais-provider-field-wide">
              API Base URL
              <input
                value={props.draft.baseUrl}
                placeholder="https://provider.example/v1"
                onChange={(e) => update('baseUrl', e.target.value)}
              />
            </label>
          </div>
        </div>

        <div className="ais-provider-section">
          <div className="ais-provider-section-heading">
            <span>02</span>
            <div>
              <h4>凭据与模型</h4>
              <p>密钥只写入系统密钥环，不在界面或日志中回显。</p>
            </div>
          </div>
          <div className="ais-provider-fields">
            <label className="ais-provider-field-wide">
              API Key
              <div className="ais-secret">
                <LynxIcon name={'key'} size={16} />
                <input
                  type="password"
                  autoComplete="new-password"
                  value={props.draft.apiKey ?? ''}
                  placeholder={
                    props.draft.id ? '留空保留密钥环中的 Key' : '输入后直接保存到系统密钥环'
                  }
                  onChange={(e) => update('apiKey', e.target.value)}
                />
              </div>
            </label>
            <label className="ais-provider-field-wide">
              模型
              <input value={props.draft.model} onChange={(e) => update('model', e.target.value)} />
            </label>
          </div>
        </div>

        <details className="ais-provider-advanced">
          <summary>
            <span className="ais-provider-advanced-icon">
              <LynxIcon name={'runtime'} size={16} />
            </span>
            <span>
              <strong>运行限制与费用</strong>
              <small>超时、Token 上限和单任务预算</small>
            </span>
            <em>展开配置</em>
          </summary>
          <div className="ais-provider-advanced-body">
            <div className="ais-form-row ais-form-row--three">
              <label>
                超时（秒）
                <input
                  aria-label="连接超时秒数"
                  type="number"
                  value={props.draft.timeoutSeconds}
                  onChange={(e) => update('timeoutSeconds', Number(e.target.value))}
                />
              </label>
              <label>
                最大 Token
                <input
                  aria-label="最大 Token"
                  type="number"
                  value={props.draft.maxTokens ?? ''}
                  onChange={(e) =>
                    update('maxTokens', e.target.value ? Number(e.target.value) : null)
                  }
                />
              </label>
              <label>
                单任务费用上限
                <input
                  aria-label="单任务费用上限"
                  type="number"
                  min="0"
                  step="0.01"
                  value={props.draft.taskCostLimit ?? ''}
                  onChange={(e) =>
                    update('taskCostLimit', e.target.value ? Number(e.target.value) : null)
                  }
                />
              </label>
            </div>
            <div className="ais-form-row">
              <label>
                输入费率 / 百万 Token
                <input
                  aria-label="输入费率每百万 Token"
                  type="number"
                  min="0"
                  step="0.01"
                  value={props.draft.inputCostPerMillion ?? ''}
                  onChange={(e) =>
                    update('inputCostPerMillion', e.target.value ? Number(e.target.value) : null)
                  }
                />
              </label>
              <label>
                输出费率 / 百万 Token
                <input
                  aria-label="输出费率每百万 Token"
                  type="number"
                  min="0"
                  step="0.01"
                  value={props.draft.outputCostPerMillion ?? ''}
                  onChange={(e) =>
                    update('outputCostPerMillion', e.target.value ? Number(e.target.value) : null)
                  }
                />
              </label>
            </div>
            <p className="ais-note">设置费用上限时至少填写一个费率；未填写的方向按 0 估算。</p>
          </div>
        </details>

        <div className="ais-provider-switches">
          <label>
            <span>
              <strong>启用此连接</strong>
              <small>允许任务使用该服务</small>
            </span>
            <input
              type="checkbox"
              checked={props.draft.enabled}
              onChange={(e) => update('enabled', e.target.checked)}
            />
          </label>
          <label>
            <span>
              <strong>设为默认连接</strong>
              <small>新任务优先选择该服务</small>
            </span>
            <input
              type="checkbox"
              checked={props.draft.isDefault}
              onChange={(e) => update('isDefault', e.target.checked)}
            />
          </label>
        </div>

        <footer className="ais-provider-actions">
          {props.draft.id && (
            <button
              className="danger ais-provider-delete"
              aria-label="删除当前连接"
              title="删除连接"
              onClick={() => void props.onDelete(props.draft.id!)}
            >
              <LynxIcon name={'delete'} size={16} />
            </button>
          )}
          <span />
          <button disabled={props.busy || !props.draft.id} onClick={props.onTest}>
            <LynxIcon name={'refresh'} size={16} /> 测试连接
          </button>
          <button className="primary" disabled={props.busy} onClick={props.onSave}>
            <LynxIcon name={'save'} size={16} /> 保存连接
          </button>
        </footer>
      </section>
      <aside className="ais-provider-side">
        <section className="ais-card ais-provider-status-card">
          <div className="ais-card-title">
            <div>
              <span className="ais-section-kicker">HEALTH</span>
              <h3>连接状态</h3>
              <p>
                {selected
                  ? `最后测试 ${formatTime(selected.lastTestAt)}`
                  : '保存后可执行真实连接测试'}
              </p>
            </div>
            {selected && (
              <StatusBadge
                label={statusLabel(selected.lastTestStatus || '未测试')}
                tone={statusTone(selected.lastTestStatus)}
              />
            )}
          </div>
          <dl className="ais-fact-list">
            <div>
              <dt>API Key</dt>
              <dd>{selected?.hasApiKey ? '已存入系统密钥环' : '未配置'}</dd>
            </div>
            <div>
              <dt>运行模型</dt>
              <dd>{props.draft.model || '未填写'}</dd>
            </div>
            <div>
              <dt>默认连接</dt>
              <dd>{props.draft.isDefault ? '是' : '否'}</dd>
            </div>
          </dl>
          {selected?.lastTestMessage && (
            <p className="ais-test-message">{selected.lastTestMessage}</p>
          )}
        </section>
        <section className="ais-card ais-provider-boundary-card">
          <span className="ais-section-kicker">PRIVACY</span>
          <h3>数据边界</h3>
          <div className="ais-boundary good">
            <LynxIcon name={'check-circle'} size={16} />
            <span>
              <strong>允许发送</strong>
              <small>任务事实、限定日志、允许动作和剩余预算</small>
            </span>
          </div>
          <div className="ais-boundary bad">
            <LynxIcon name={'shield'} size={16} />
            <span>
              <strong>始终留在本机</strong>
              <small>API Key、SSH 密码、私钥、其他板卡日志、环境变量</small>
            </span>
          </div>
          <p className="ais-note">
            Coding Plan 等服务请填写其提供的 OpenAI 兼容 Base URL 与模型名，LYNX
            不读取任何订阅客户端凭据。
          </p>
        </section>
      </aside>
    </div>
  );
};

interface PolicyEditorProps {
  providers: AiProviderConfig[];
  policies: SupervisorPolicy[];
  draft: SupervisorPolicy;
  setDraft: React.Dispatch<React.SetStateAction<SupervisorPolicy>>;
  devices: WorkbenchDevice[];
  mcuBoards: McuBoardConfig[];
  busy: boolean;
  onSave: () => void;
  onNew: () => void;
}

const PolicyEditor: React.FC<PolicyEditorProps> = (props) => {
  const update = <K extends keyof SupervisorPolicy>(key: K, value: SupervisorPolicy[K]) =>
    props.setDraft((current) => ({ ...current, [key]: value }));
  const actionMode = (action: string) =>
    props.draft.allowedActions.includes(action)
      ? 'automatic'
      : props.draft.approvalActions.includes(action)
        ? 'approval'
        : 'blocked';
  const setActionMode = (action: string, next: string) =>
    props.setDraft((current) => ({
      ...current,
      allowedActions:
        next === 'automatic'
          ? [...current.allowedActions.filter((item) => item !== action), action]
          : current.allowedActions.filter((item) => item !== action),
      approvalActions:
        next === 'approval'
          ? [...current.approvalActions.filter((item) => item !== action), action]
          : current.approvalActions.filter((item) => item !== action),
    }));
  const boardOptions = [
    ...props.devices
      .filter((device): device is WorkbenchDevice & { id: number } => device.id !== null)
      .map((device) => ({ id: `linux:${device.id}`, name: device.name, domain: 'Linux / SoC' })),
    ...props.mcuBoards.map((board) => ({
      id: `mcu:${board.id}`,
      name: board.name,
      domain: 'MCU',
    })),
  ];
  const toggleBoard = (boardId: string, selected: boolean) =>
    update(
      'boardScope',
      selected
        ? [...new Set([...props.draft.boardScope, boardId])]
        : props.draft.boardScope.filter((scope) => scope !== boardId)
    );
  return (
    <div className="ais-policy-unified">
      <aside className="ais-card ais-list">
        <div className="ais-card-title">
          <h3>策略列表</h3>
          <button onClick={props.onNew} aria-label="新建监督策略" title="新建策略">
            <LynxIcon name={'add'} size={16} />
          </button>
        </div>
        {props.policies.map((policy) => (
          <button
            key={policy.id}
            className={policy.id === props.draft.id ? 'selected' : ''}
            onClick={() => props.setDraft(policy)}
          >
            <LynxIcon name={'shield'} size={16} />
            <span>
              <strong>{policy.name}</strong>
              <small>{policy.enabled ? '已启用' : '已停用'}</small>
            </span>
          </button>
        ))}
      </aside>
      <section className="ais-card ais-form">
        <div className="ais-card-title">
          <div>
            <h3>编辑策略：{props.draft.name || '新策略'}</h3>
            <p>作用域、预算和停止条件与动作权限作为一个整体保存。</p>
          </div>
          <StatusBadge
            label={props.draft.enabled ? '已启用' : '已停用'}
            tone={props.draft.enabled ? 'success' : 'neutral'}
          />
        </div>
        <h4>基本信息</h4>
        <label>
          策略名称
          <input value={props.draft.name} onChange={(e) => update('name', e.target.value)} />
        </label>
        <label>
          兼容连接
          <select
            value={props.draft.providerId ?? ''}
            onChange={(e) => update('providerId', e.target.value ? Number(e.target.value) : null)}
          >
            <option value="">使用默认连接</option>
            {props.providers.map((provider) => (
              <option key={provider.id} value={provider.id ?? ''}>
                {provider.name} · {provider.model}
              </option>
            ))}
          </select>
        </label>
        <h4>板卡作用域</h4>
        <div className="ais-policy-board-scope" aria-label="板卡作用域">
          {boardOptions.map((board) => (
            <label key={board.id}>
              <input
                type="checkbox"
                checked={props.draft.boardScope.includes(board.id)}
                onChange={(event) => toggleBoard(board.id, event.target.checked)}
              />
              <span>
                <strong>{board.name}</strong>
                <small>{board.domain}</small>
              </span>
            </label>
          ))}
          {!boardOptions.length && (
            <p>尚未加载到已保存单板。请先在 Linux 或 MCU 工作台保存设备，然后刷新本页。</p>
          )}
        </div>
        <h4>恢复预算</h4>
        <div className="ais-budget-grid">
          <NumberField
            label="最多 AI 尝试"
            value={props.draft.maxAttempts}
            onChange={(value) => update('maxAttempts', value)}
          />
          <NumberField
            label="监督时长（秒）"
            value={props.draft.maxDurationSeconds}
            onChange={(value) => update('maxDurationSeconds', value)}
          />
          <NumberField
            label="同一错误上限"
            value={props.draft.maxSameError}
            onChange={(value) => update('maxSameError', value)}
          />
          <NumberField
            label="电源循环上限"
            value={props.draft.maxPowerCycles}
            onChange={(value) => update('maxPowerCycles', value)}
          />
          <NumberField
            label="重新烧录上限"
            value={props.draft.maxFlashRetries}
            onChange={(value) => update('maxFlashRetries', value)}
          />
          <NumberField
            label="等待设备（秒）"
            value={props.draft.waitDeviceSeconds}
            onChange={(value) => update('waitDeviceSeconds', value)}
          />
          <label>
            单任务费用上限
            <input
              type="number"
              min="0"
              step="0.01"
              value={props.draft.costLimit ?? ''}
              placeholder="使用连接上限"
              onChange={(event) =>
                update('costLimit', event.target.value ? Number(event.target.value) : null)
              }
            />
          </label>
        </div>
        <label className="ais-check">
          <input
            type="checkbox"
            checked={props.draft.enabled}
            onChange={(e) => update('enabled', e.target.checked)}
          />
          启用监督策略
        </label>
      </section>
      <aside className="ais-card ais-policy-permissions">
        <div className="ais-card-title">
          <div>
            <h3>动作权限矩阵</h3>
            <p>每个动作只能处于一种执行模式。</p>
          </div>
        </div>
        <div className="ais-permissions ais-permissions--compact">
          <div className="ais-permission-head">
            <span>动作</span>
            <span>用途</span>
            <span>执行模式</span>
          </div>
          {Object.entries(ACTION_LABELS).map(([action, label]) => (
            <div key={action}>
              <code>{action}</code>
              <strong>{label}</strong>
              <select
                aria-label={`${label}（${action}）执行模式`}
                value={actionMode(action)}
                onChange={(event) => setActionMode(action, event.target.value)}
              >
                <option value="automatic">自动允许</option>
                <option value="approval">需要人工批准</option>
                <option value="blocked">禁止</option>
              </select>
            </div>
          ))}
        </div>
        <div className="ais-deterministic-gate">
          <h4>确定性安全门禁</h4>
          <ul className="ais-rule-list">
            <li>动作参数必须通过 Schema 与白名单</li>
            <li>高风险动作必须取得人工批准</li>
            <li>硬件锁禁止同板并发操作</li>
            <li>AI 不能自行确认硬件成功</li>
          </ul>
        </div>
      </aside>
      <footer className="ais-policy-savebar">
        <span>
          <LynxIcon name={'key'} size={16} /> API Key 仅写入系统密钥环；策略保存不包含任何明文密钥。
        </span>
        <button className="primary" disabled={props.busy} onClick={props.onSave}>
          <LynxIcon name={'save'} size={16} /> 保存策略与权限
        </button>
      </footer>
    </div>
  );
};

const NumberField = ({
  label,
  value,
  onChange,
}: {
  label: string;
  value: number;
  onChange: (value: number) => void;
}) => (
  <label>
    {label}
    <input type="number" value={value} onChange={(event) => onChange(Number(event.target.value))} />
  </label>
);

const NotificationEditor: React.FC<{
  draft: SupervisorPolicy;
  setDraft: React.Dispatch<React.SetStateAction<SupervisorPolicy>>;
  onSave: () => Promise<void>;
  busy: boolean;
}> = ({ draft, setDraft, onSave, busy }) => (
  <div className="ais-notification-layout">
    <section className="ais-card ais-form">
      <div className="ais-card-title">
        <div>
          <h3>通知与升级</h3>
          <p>人工介入、预算耗尽和终止结果共用当前策略的通知方式。</p>
        </div>
        <StatusBadge
          label={draft.enabled ? `策略：${draft.name}` : '策略未启用'}
          tone={draft.enabled ? 'success' : 'neutral'}
        />
      </div>
      <label>
        通知方式
        <select
          value={draft.notificationMode}
          onChange={(event) =>
            setDraft((current) => ({ ...current, notificationMode: event.target.value }))
          }
        >
          <option value="desktop">LYNX 桌面通知</option>
          <option value="desktop_email">LYNX + 邮件</option>
          <option value="none">仅保留介入队列</option>
        </select>
      </label>
      <label className="ais-check">
        <input
          type="checkbox"
          checked={draft.successSummary}
          onChange={(event) =>
            setDraft((current) => ({ ...current, successSummary: event.target.checked }))
          }
        />
        任务恢复成功后生成摘要通知
      </label>
      <div className="ais-actions">
        <button className="primary" disabled={busy} onClick={() => void onSave()}>
          <LynxIcon name={'save'} size={16} /> 保存通知设置
        </button>
      </div>
    </section>
    <aside className="ais-card">
      <h3>强制升级事件</h3>
      <ul className="ais-rule-list">
        <li>动作超出白名单或需要人工批准</li>
        <li>AI、时间、费用或硬件动作预算耗尽</li>
        <li>连续出现相同错误且没有新证据</li>
        <li>确定性验收失败或设备状态不可判定</li>
      </ul>
      <p className="ais-safety-line">通知不会自动批准、重试或确认任何硬件结果。</p>
    </aside>
  </div>
);

export const LegacyAutomaticEditor: React.FC<{
  tasks: SupervisorAutomaticTask[];
  policies: SupervisorPolicy[];
  draft: SupervisorAutomaticTask;
  setDraft: React.Dispatch<React.SetStateAction<SupervisorAutomaticTask>>;
  webhookSecret: string;
  setWebhookSecret: React.Dispatch<React.SetStateAction<string>>;
  busy: boolean;
  onNew: () => void;
  onSave: () => Promise<void>;
  onDelete: (id: number) => Promise<void>;
  onTrigger: (id: number) => Promise<void>;
}> = ({
  tasks,
  policies,
  draft,
  setDraft,
  webhookSecret,
  setWebhookSecret,
  busy,
  onNew,
  onSave,
  onDelete,
  onTrigger,
}) => {
  const update = <K extends keyof SupervisorAutomaticTask>(
    key: K,
    value: SupervisorAutomaticTask[K]
  ) => setDraft((current) => ({ ...current, [key]: value }));
  return (
    <div className="ais-three-column">
      <aside className="ais-card ais-list">
        <div className="ais-card-title">
          <h3>已创建任务</h3>
          <button onClick={onNew} aria-label="新建自动任务" title="新建自动任务">
            <LynxIcon name={'add'} size={16} />
          </button>
        </div>
        {tasks.map((task) => (
          <button
            key={task.id}
            className={task.id === draft.id ? 'selected' : ''}
            onClick={() => {
              setDraft(task);
              setWebhookSecret('');
            }}
          >
            <LynxIcon name={'refresh'} size={16} />
            <span>
              <strong>{task.name}</strong>
              <small>
                {task.triggerType} · {task.enabled ? '已启用' : '草稿'}
              </small>
            </span>
          </button>
        ))}
        {tasks.length === 0 && <p className="ais-empty-copy">尚无自动任务。</p>}
      </aside>
      <section className="ais-card ais-form">
        <h3>触发器与固定板卡作用域</h3>
        <label>
          任务名称
          <input value={draft.name} onChange={(e) => update('name', e.target.value)} />
        </label>
        <div className="ais-form-row">
          <label>
            触发类型
            <select
              value={draft.triggerType}
              onChange={(e) =>
                update('triggerType', e.target.value as SupervisorAutomaticTask['triggerType'])
              }
            >
              <option value="schedule">定时</option>
              <option value="webhook">Webhook</option>
              <option value="device">设备事件</option>
              <option value="manual_api">手动/API</option>
            </select>
          </label>
          <label>
            监督策略
            <select
              value={draft.policyId ?? ''}
              onChange={(e) => update('policyId', e.target.value ? Number(e.target.value) : null)}
            >
              <option value="">未选择</option>
              {policies.map((policy) => (
                <option key={policy.id} value={policy.id ?? ''}>
                  {policy.name}
                </option>
              ))}
            </select>
          </label>
        </div>
        <label>
          固定 board_id
          <input
            value={draft.boardId}
            placeholder="必须绑定一块已登记板卡"
            onChange={(e) => update('boardId', e.target.value)}
          />
        </label>
        <details className="ais-advanced">
          <summary>高级触发参数</summary>
          <label>
            触发参数 JSON
            <JsonEditor
              value={draft.trigger}
              onChange={(value) => update('trigger', value as Record<string, unknown>)}
              rows={5}
            />
          </label>
        </details>
        {draft.triggerType === 'webhook' && (
          <label>
            Webhook Bearer 密钥
            <input
              type="password"
              autoComplete="new-password"
              value={webhookSecret}
              placeholder={draft.id ? '留空保留系统密钥环中的密钥' : '至少 16 个字符'}
              onChange={(event) => setWebhookSecret(event.target.value)}
            />
            <small>
              调用路径：/supervisor/webhook/{draft.id ?? '{任务ID}'}；密钥只写入系统密钥环
            </small>
          </label>
        )}
        <details className="ais-advanced">
          <summary>高级任务模板</summary>
          <label>
            任务模板 JSON
            <JsonEditor
              value={draft.taskTemplate}
              onChange={(value) => update('taskTemplate', value as Record<string, unknown>)}
              rows={7}
            />
            <small>
              仅支持 workflow；必须提供正整数 project_id、device_id，且 board_id 必须等于
              linux:device_id。
            </small>
          </label>
        </details>
        <details className="ais-advanced">
          <summary>确定性成功条件</summary>
          <label>
            验收条件 JSON
            <JsonEditor
              value={draft.successConditions}
              onChange={(value) =>
                update('successConditions', value as Array<Record<string, unknown>>)
              }
              rows={5}
            />
          </label>
        </details>
        <label className="ais-check">
          <input
            type="checkbox"
            checked={draft.enabled}
            onChange={(e) => update('enabled', e.target.checked)}
          />
          验证配置后启用
        </label>
        <div className="ais-actions">
          {draft.id && (
            <button onClick={() => void onTrigger(draft.id!)}>
              <LynxIcon name={'refresh'} size={16} /> 立即运行
            </button>
          )}
          <button className="primary" disabled={busy} onClick={() => void onSave()}>
            <LynxIcon name={'save'} size={16} /> 保存任务
          </button>
          {draft.id && (
            <button
              className="danger"
              aria-label="删除当前自动任务"
              title="删除任务"
              onClick={() => void onDelete(draft.id!)}
            >
              <LynxIcon name={'delete'} size={16} />
            </button>
          )}
        </div>
      </section>
      <aside className="ais-automation-side">
        <section className="ais-card">
          <div className="ais-card-title">
            <div>
              <h3>执行摘要</h3>
              <p>保存前检查固定作用域。</p>
            </div>
            <StatusBadge
              label={draft.enabled ? '已启用' : '未启用'}
              tone={draft.enabled ? 'success' : 'neutral'}
            />
          </div>
          <dl className="ais-fact-list">
            <div>
              <dt>触发方式</dt>
              <dd>
                {
                  {
                    schedule: '定时',
                    webhook: 'Webhook',
                    device: '设备事件',
                    manual_api: '手动 / API',
                  }[draft.triggerType]
                }
              </dd>
            </div>
            <div>
              <dt>目标板卡</dt>
              <dd>{draft.boardId || '未绑定'}</dd>
            </div>
            <div>
              <dt>监督策略</dt>
              <dd>{policies.find((policy) => policy.id === draft.policyId)?.name ?? '未选择'}</dd>
            </div>
            <div>
              <dt>上次运行</dt>
              <dd>{formatTime(draft.lastRunAt)}</dd>
            </div>
            <div>
              <dt>下次运行</dt>
              <dd>{formatTime(draft.nextRunAt)}</dd>
            </div>
          </dl>
        </section>
        <section className="ais-card">
          <h3>启用前检查</h3>
          <ReadinessItem
            ready={Boolean(draft.boardId)}
            label="已绑定固定板卡"
            detail={draft.boardId || '需要 board_id'}
          />
          <ReadinessItem
            ready={Boolean(draft.policyId)}
            label="已选择监督策略"
            detail={policies.find((policy) => policy.id === draft.policyId)?.name ?? '尚未选择'}
          />
          <ReadinessItem
            ready={draft.successConditions.length > 0}
            label="已定义确定性验收"
            detail={`${draft.successConditions.length} 条成功条件`}
          />
        </section>
      </aside>
    </div>
  );
};

const JsonEditor: React.FC<{
  value: unknown;
  onChange: (value: unknown) => void;
  rows: number;
}> = ({ value, onChange, rows }) => {
  const serialized = useMemo(() => JSON.stringify(value, null, 2), [value]);
  const [text, setText] = useState(serialized);
  const [valid, setValid] = useState(true);

  useEffect(() => {
    setText(serialized);
    setValid(true);
  }, [serialized]);

  return (
    <>
      <textarea
        className={valid ? '' : 'invalid'}
        value={text}
        onChange={(event) => {
          const next = event.target.value;
          setText(next);
          try {
            onChange(JSON.parse(next));
            setValid(true);
          } catch {
            setValid(false);
          }
        }}
        rows={rows}
      />
      {!valid && <small className="ais-json-error">JSON 尚未完成或格式不正确</small>}
    </>
  );
};

const JsonFacts = ({ value }: { value: unknown }) => (
  <pre className="ais-json-facts">{JSON.stringify(value, null, 2)}</pre>
);

type RunCenterFilter = 'all' | 'running' | 'approval' | 'human' | 'finished';
type RunCenterTab = 'timeline' | 'human' | 'calls' | 'audit';

const RunCenter: React.FC<{
  runs: SupervisorRun[];
  rounds: SupervisorRound[];
  workflowSteps: WorkbenchWorkflowStep[];
  audit: SupervisorAuditRecord[];
  automaticTasks: SupervisorAutomaticTask[];
  providers: AiProviderConfig[];
  humanRequests: SupervisorHumanRequest[];
  selectedRunId: string;
  onSelectRun: (runId: string) => void;
  onApprove: (run: SupervisorRun) => Promise<void>;
  onControl: (
    runId: string,
    action: 'pause' | 'resume' | 'takeover' | 'terminate'
  ) => Promise<void>;
  onResolve: (requestId: number, resolution: 'continue' | 'manual' | 'terminate') => Promise<void>;
}> = ({
  runs,
  rounds,
  workflowSteps,
  audit,
  automaticTasks,
  providers,
  humanRequests,
  selectedRunId,
  onSelectRun,
  onApprove,
  onControl,
  onResolve,
}) => {
  const [filter, setFilter] = useState<RunCenterFilter>('all');
  const [tab, setTab] = useState<RunCenterTab>('timeline');
  const selected = runs.find((run) => run.id === selectedRunId) ?? runs[0];
  const request =
    humanRequests.find((item) => item.runId === selected?.id && item.status === 'waiting') ??
    humanRequests.find((item) => item.runId === selected?.id);
  const runAudit = audit.filter((item) => item.runId === selected?.id);
  const latestRound = rounds[rounds.length - 1];
  const automaticTaskForRun = (run: SupervisorRun) => {
    const triggerRecord = audit.find((record) => {
      if (record.eventType !== 'automatic_trigger') return false;
      if (record.runId === run.id) return true;
      return String(record.output.task_id ?? record.output.taskId ?? '') === run.taskId;
    });
    const automaticTaskId = Number(
      triggerRecord?.redactedInput.automatic_task_id ??
        triggerRecord?.redactedInput.automaticTaskId ??
        Number.NaN
    );
    return automaticTasks.find((task) => task.id === automaticTaskId);
  };
  const taskDisplayName = (run: SupervisorRun) => {
    const automaticName = automaticTaskForRun(run)?.name;
    if (automaticName) return automaticName;
    if (run.taskId.startsWith('flash-')) return '固件烧录';
    if (run.taskId.startsWith('device-flow-')) return '设备自动任务';
    if (run.taskId.startsWith('live-')) return '实时监督任务';
    return '监督任务';
  };
  const matchesFilter = (run: SupervisorRun, targetFilter = filter) => {
    if (targetFilter === 'running') return ['running', 'paused'].includes(run.status);
    if (targetFilter === 'approval') return run.status === 'waiting_approval';
    if (targetFilter === 'human') return run.status === 'waiting_human';
    if (targetFilter === 'finished')
      return ['success', 'failed', 'terminated', 'cancelled', 'manual'].includes(run.status);
    return true;
  };
  const visibleRuns = runs.filter((run) => matchesFilter(run));
  const counts: Record<RunCenterFilter, number> = {
    all: runs.length,
    running: runs.filter((run) => ['running', 'paused'].includes(run.status)).length,
    approval: runs.filter((run) => run.status === 'waiting_approval').length,
    human: runs.filter((run) => run.status === 'waiting_human').length,
    finished: runs.filter((run) =>
      ['success', 'failed', 'terminated', 'cancelled', 'manual'].includes(run.status)
    ).length,
  };
  const chooseRun = (run: SupervisorRun) => {
    onSelectRun(run.id);
    if (run.status === 'waiting_human') setTab('human');
    else if (tab === 'human') setTab('timeline');
  };
  const chooseFilter = (nextFilter: RunCenterFilter) => {
    setFilter(nextFilter);
    const first = runs.find((run) => matchesFilter(run, nextFilter));
    if (first && (!selected || !matchesFilter(selected, nextFilter))) chooseRun(first);
  };

  useEffect(() => {
    if (selected?.status === 'waiting_human' && request?.status === 'waiting') setTab('human');
  }, [request?.status, selected?.id, selected?.status]);

  if (!selected) {
    return (
      <EmptyState
        icon={'runtime'}
        title="当前没有监督运行"
        detail="任务失败且匹配监督策略后，运行、人工介入和审计证据会在这里共享同一个任务上下文。"
      />
    );
  }

  return (
    <div className="ais-run-center">
      <nav className="ais-run-filters" aria-label="运行状态筛选">
        {(
          [
            ['all', '全部'],
            ['running', '运行中'],
            ['approval', '等待审批'],
            ['human', '等待人工'],
            ['finished', '已结束'],
          ] as Array<[RunCenterFilter, string]>
        ).map(([key, label]) => (
          <button
            key={key}
            className={`${filter === key ? 'active' : ''} ${key === 'human' && counts.human ? 'attention' : ''}`}
            onClick={() => chooseFilter(key)}
          >
            {label} <strong>{counts[key]}</strong>
          </button>
        ))}
      </nav>

      <div className="ais-run-center-layout">
        <aside className="ais-card ais-run-queue">
          <div className="ais-card-title">
            <div>
              <h3>运行队列</h3>
              <p>选择一次，所有证据视图同步切换。</p>
            </div>
          </div>
          <div className="ais-run-queue-items">
            {visibleRuns.map((run) => (
              <article key={run.id} className={run.id === selected.id ? 'selected' : ''}>
                <button className="ais-run-queue-select" onClick={() => chooseRun(run)}>
                  <span>
                    <strong title={taskDisplayName(run)}>{taskDisplayName(run)}</strong>
                  </span>
                </button>
                {['waiting_human', 'waiting_approval'].includes(run.status) ? (
                  <button className="ais-run-queue-action" onClick={() => chooseRun(run)}>
                    {run.status === 'waiting_human' ? '处理' : '审批'}
                  </button>
                ) : (
                  <span className="ais-run-queue-status">
                    <StatusBadge label={statusLabel(run.status)} tone={statusTone(run.status)} />
                  </span>
                )}
              </article>
            ))}
            {!visibleRuns.length && <p className="ais-empty-copy">当前筛选下没有运行记录。</p>}
          </div>
        </aside>

        <main className="ais-card ais-run-context">
          <div className="ais-run-identity">
            <div className="ais-run-identity-heading">
              <div>
                <small>当前任务</small>
                <h3>{taskDisplayName(selected)}</h3>
              </div>
              <StatusBadge
                label={statusLabel(selected.status)}
                tone={statusTone(selected.status)}
              />
            </div>
            <dl className="ais-run-identity-facts">
              <div>
                <dt>板卡</dt>
                <dd>{selected.boardId}</dd>
              </div>
              <div>
                <dt>尝试</dt>
                <dd>第 {selected.attemptCount} 次</dd>
              </div>
              <div className="ais-run-identity-time">
                <dt>操作时间</dt>
                <dd>{formatTime(selected.startedAt)}</dd>
              </div>
              <div className="ais-run-identity-id">
                <dt>运行编号</dt>
                <dd title={selected.id}>{selected.id}</dd>
              </div>
            </dl>
          </div>
          <nav className="ais-context-tabs" aria-label="当前运行详情">
            {(
              [
                ['timeline', '运行时间线'],
                ['human', `人工处理${request?.status === 'waiting' ? ' · 1' : ''}`],
                ['calls', `AI 调用 · ${runAudit.length}`],
                ['audit', '完整审计'],
              ] as Array<[RunCenterTab, string]>
            ).map(([key, label]) => (
              <button
                key={key}
                className={`${tab === key ? 'active' : ''} ${key === 'human' && request?.status === 'waiting' ? 'attention' : ''}`}
                onClick={() => setTab(key)}
              >
                {label}
              </button>
            ))}
          </nav>

          {tab === 'timeline' && (
            <div className="ais-runtime-live">
              {!workflowSteps.length && !rounds.length ? (
                <EmptyState
                  compact
                  icon={'list-check'}
                  title="暂无任务执行步骤"
                  detail="任务尚未进入执行器，或任务步骤记录尚未加载。"
                />
              ) : (
                <>
                  {workflowSteps.length > 0 && (
                    <section className="ais-runtime-console" aria-label="设备任务执行步骤">
                      <header>
                        <div>
                          <i />
                          <strong>设备任务实时步骤</strong>
                        </div>
                        <span>{workflowSteps.length} 个执行步骤</span>
                      </header>
                      <div className="ais-runtime-console-body">
                        {workflowSteps.map((step) => (
                          <div className="ais-runtime-log" key={step.id}>
                            <time>[{new Date(step.startedAt * 1000).toLocaleTimeString()}]</time>
                            <span className="is-action">设备执行</span>
                            <code>
                              {step.stepName.split(':').slice(1).join(':') || step.stepName}
                            </code>
                            <StatusBadge
                              label={statusLabel(step.status)}
                              tone={statusTone(step.status)}
                            />
                            {step.error && <code>{step.error}</code>}
                          </div>
                        ))}
                      </div>
                    </section>
                  )}

                  {rounds.length > 0 && (
                    <section className="ais-runtime-console">
                      <header>
                        <div>
                          <i />
                          <strong>实时日志与恢复轨迹</strong>
                        </div>
                        <span>{rounds.length} 个监督回合</span>
                      </header>
                      <div className="ais-runtime-console-body">
                        {rounds.map((round) => (
                          <div className="ais-runtime-log" key={round.id}>
                            <time>[{new Date(round.createdAt * 1000).toLocaleTimeString()}]</time>
                            <span className="is-fact">系统事实</span>
                            <code>{round.failureFingerprint || '无错误指纹'}</code>
                            <time>[{new Date(round.createdAt * 1000).toLocaleTimeString()}]</time>
                            <span className="is-ai">AI 决策</span>
                            <code>
                              {ACTION_LABELS[round.decision.decision] ?? round.decision.decision}
                            </code>
                            <time>[{new Date(round.createdAt * 1000).toLocaleTimeString()}]</time>
                            <span className="is-action">执行结果</span>
                            <code>{round.actionResult || '等待执行'}</code>
                            <time>[{new Date(round.createdAt * 1000).toLocaleTimeString()}]</time>
                            <span className="is-verify">执行与验收</span>
                            <code>{round.verificationResult || '等待验收'}</code>
                          </div>
                        ))}
                      </div>
                    </section>
                  )}

                  {rounds.length > 0 && (
                    <section className="ais-recovery-track" aria-label="恢复轨迹">
                      {rounds.map((round, index) => (
                        <article
                          key={round.id}
                          className={index === rounds.length - 1 ? 'current' : ''}
                        >
                          <span className="ais-recovery-index">{round.roundIndex}</span>
                          <div>
                            <small>{formatTime(round.createdAt)}</small>
                            <strong>
                              {ACTION_LABELS[round.decision.decision] ?? round.decision.decision}
                            </strong>
                            <span>
                              {round.verificationResult || round.actionResult || '等待执行'}
                            </span>
                          </div>
                          <StatusBadge
                            label={round.verificationResult ? '已验收' : '进行中'}
                            tone={round.verificationResult ? 'success' : 'info'}
                          />
                        </article>
                      ))}
                    </section>
                  )}
                </>
              )}
            </div>
          )}

          {tab === 'human' &&
            (request ? (
              <div className="ais-run-human">
                <section className="ais-instruction">
                  <LynxIcon name={'person-settings'} size={16} />
                  <div>
                    <h4>{request.reason}</h4>
                    <p>{request.instruction}</p>
                  </div>
                </section>
                <div className="ais-human-evidence">
                  <section className="ais-evidence ais-evidence--fact">
                    <h4>系统事实</h4>
                    {request.evidence.length ? (
                      <ul>
                        {request.evidence.map((item) => (
                          <li key={item}>{item}</li>
                        ))}
                      </ul>
                    ) : (
                      <p>未记录额外系统证据。</p>
                    )}
                  </section>
                  <section className="ais-evidence ais-evidence--inference">
                    <h4>监督结论</h4>
                    <p>{request.reason}</p>
                    <small>这是请求人工介入的原因，不代表硬件状态已经验收。</small>
                  </section>
                </div>
                {request.resolution && (
                  <section className="ais-resolution">
                    <strong>处理结果</strong>
                    <p>{request.resolution}</p>
                    <span>{formatTime(request.resolvedAt)}</span>
                  </section>
                )}
                {request.status === 'waiting' && (
                  <>
                    <div className="ais-actions ais-human-actions">
                      <button
                        className="primary"
                        onClick={() => void onResolve(request.id, 'continue')}
                      >
                        <LynxIcon name={'check-circle'} size={16} /> 已处理，继续监督
                      </button>
                      <button onClick={() => void onResolve(request.id, 'manual')}>
                        <LynxIcon name={'person-settings'} size={16} /> 转为手动模式
                      </button>
                      <button
                        className="danger"
                        onClick={() => void onResolve(request.id, 'terminate')}
                      >
                        <LynxIcon name={'prohibited'} size={16} /> 终止任务
                      </button>
                    </div>
                    <p className="ais-note">
                      继续监督只恢复监督状态，不会自动重试原烧录或其他硬件步骤。
                    </p>
                  </>
                )}
              </div>
            ) : (
              <EmptyState
                compact
                icon={'check-circle'}
                title="当前运行不需要人工介入"
                detail="等待审批和人工操作会在这里显示，不再跳转到另一套任务列表。"
              />
            ))}

          {tab === 'calls' && (
            <div className="ais-run-calls">
              {!runAudit.length ? (
                <EmptyState
                  compact
                  icon={'agent'}
                  title="当前运行没有 AI 调用"
                  detail="监督器不会为了健康检查而消耗模型调用。"
                />
              ) : (
                runAudit.map((record) => {
                  const provider = providers.find((item) => item.id === record.providerId);
                  return (
                    <article key={record.id}>
                      <div>
                        <strong>{provider?.name ?? `Provider #${record.providerId ?? '—'}`}</strong>
                        <small>{formatTime(record.createdAt)}</small>
                      </div>
                      <StatusBadge
                        label={statusLabel(record.result)}
                        tone={statusTone(record.result)}
                      />
                      <span>{record.inputTokens + record.outputTokens} Tokens</span>
                      <span>{formatCost(record.estimatedCost)}</span>
                    </article>
                  );
                })
              )}
            </div>
          )}

          {tab === 'audit' && <AuditTable records={runAudit} providers={providers} compact />}
        </main>

        <aside className="ais-run-center-side">
          <section className="ais-card ais-current-judgement">
            <div className="ais-card-title">
              <div>
                <h3>AI 当前判断</h3>
                <p>事实、推断和验收严格分层</p>
              </div>
              <LynxIcon name={'brain'} size={16} />
            </div>
            {latestRound ? (
              <>
                <div className="ais-judgement-block is-fact">
                  <span>系统事实</span>
                  <strong>{latestRound.failureFingerprint || '无错误指纹'}</strong>
                  <JsonFacts value={latestRound.redactedContext} />
                </div>
                <div className="ais-judgement-block is-ai">
                  <span>AI 推断</span>
                  <p>{latestRound.decision.reason}</p>
                  <small>推断不代表硬件已经成功。</small>
                </div>
                <div className="ais-next-action">
                  <span>下一动作</span>
                  <strong>
                    {ACTION_LABELS[latestRound.decision.decision] ?? latestRound.decision.decision}
                  </strong>
                  <div>
                    <span>置信度 {Math.round(latestRound.decision.confidence * 100)}%</span>
                    <span>{latestRound.policyResult || '等待策略校验'}</span>
                  </div>
                </div>
              </>
            ) : (
              <p className="ais-empty-copy">产生第一个监督回合后显示当前判断。</p>
            )}
          </section>
          <section className="ais-card ais-run-actions">
            <h3>处理动作</h3>
            {selected.status === 'waiting_approval' && (
              <button className="primary" onClick={() => void onApprove(selected)}>
                <LynxIcon name={'check-circle'} size={16} /> 批准当前动作
              </button>
            )}
            {request?.status === 'waiting' && (
              <>
                <button className="primary" onClick={() => void onResolve(request.id, 'continue')}>
                  <LynxIcon name={'play'} size={16} /> 已处理，继续监督
                </button>
                <button onClick={() => void onResolve(request.id, 'manual')}>
                  <LynxIcon name={'person-settings'} size={16} /> 转为手动模式
                </button>
                <button className="danger" onClick={() => void onResolve(request.id, 'terminate')}>
                  <LynxIcon name={'prohibited'} size={16} /> 终止任务
                </button>
              </>
            )}
            {request?.status !== 'waiting' && (
              <>
                <button onClick={() => void onControl(selected.id, 'takeover')}>
                  <LynxIcon name={'person-settings'} size={16} /> 转为手动模式
                </button>
                <button
                  onClick={() =>
                    void onControl(selected.id, selected.status === 'paused' ? 'resume' : 'pause')
                  }
                >
                  <LynxIcon name={selected.status === 'paused' ? 'play' : 'pause'} size={16} />{' '}
                  {selected.status === 'paused' ? '恢复监督' : '暂停监督'}
                </button>
                <button className="danger" onClick={() => void onControl(selected.id, 'terminate')}>
                  <LynxIcon name={'prohibited'} size={16} /> 终止任务
                </button>
              </>
            )}
            <p className="ais-note">这些操作只改变监督状态，不强制打断已授权硬件步骤。</p>
          </section>
          <section className="ais-card ais-run-budget">
            <h3>恢复预算</h3>
            <div className="ais-budget-facts ais-budget-facts--stack">
              <span>
                <strong>{selected.attemptCount}</strong>AI 调用
              </span>
              <span>
                <strong>{selected.powerCycleCount}</strong>电源循环
              </span>
              <span>
                <strong>{selected.flashRetryCount}</strong>烧录重试
              </span>
              <span>
                <strong>{formatCost(selected.estimatedCost)}</strong>估算费用
              </span>
            </div>
          </section>
          <section className="ais-card ais-run-audit-summary">
            <h3>审计摘要</h3>
            <dl className="ais-fact-list">
              <div>
                <dt>AI 调用</dt>
                <dd>{runAudit.length}</dd>
              </div>
              <div>
                <dt>Token</dt>
                <dd>{selected.totalInputTokens + selected.totalOutputTokens}</dd>
              </div>
              <div>
                <dt>最终原因</dt>
                <dd>{selected.finalReason || '运行中'}</dd>
              </div>
            </dl>
            <button onClick={() => setTab('audit')}>查看完整审计</button>
          </section>
        </aside>
      </div>
    </div>
  );
};

const AuditTable: React.FC<{
  records: SupervisorAuditRecord[];
  providers: AiProviderConfig[];
  compact?: boolean;
}> = ({ records, providers, compact = false }) => {
  const [selectedId, setSelectedId] = useState<number | null>(records[0]?.id ?? null);
  useEffect(() => {
    if (!records.some((record) => record.id === selectedId)) setSelectedId(records[0]?.id ?? null);
  }, [records, selectedId]);
  const selected = records.find((record) => record.id === selectedId) ?? records[0];
  const provider = providers.find((item) => item.id === selected?.providerId);
  const totalTokens = records.reduce((sum, item) => sum + item.inputTokens + item.outputTokens, 0);
  const totalCost = records.reduce((sum, item) => sum + item.estimatedCost, 0);
  if (!records.length) {
    return (
      <EmptyState
        icon={'database'}
        title="尚无 AI 调用审计"
        detail="监督器调用模型后，脱敏输入、结构化输出、Token 和费用会出现在这里。"
      />
    );
  }
  return (
    <div className={`ais-audit-page ${compact ? 'is-compact' : ''}`}>
      {!compact && (
        <div className="ais-kpis ais-kpis--compact">
          <Metric icon={'database'} label="审计记录" value={String(records.length)} />
          <Metric icon={'list-check'} label="Token 合计" value={totalTokens.toLocaleString()} />
          <Metric icon={'money'} label="估算费用" value={formatCost(totalCost)} />
        </div>
      )}
      <div className="ais-master-detail">
        <section className="ais-card ais-audit-master">
          <div className="ais-card-title">
            <div>
              <h3>调用记录</h3>
              <p>按时间倒序，点击查看完整证据链。</p>
            </div>
          </div>
          <div className="ais-audit-table">
            <div className="header">
              <span>时间</span>
              <span>板卡 / 事件</span>
              <span>结果</span>
              <span>Token</span>
              <span>费用</span>
            </div>
            {records.map((record) => (
              <button
                key={record.id}
                className={record.id === selected?.id ? 'selected' : ''}
                onClick={() => setSelectedId(record.id)}
              >
                <span>{formatTime(record.createdAt)}</span>
                <span>
                  <strong>{record.boardId || '无板卡作用域'}</strong>
                  <small>{record.eventType}</small>
                </span>
                <StatusBadge label={statusLabel(record.result)} tone={statusTone(record.result)} />
                <span>{record.inputTokens + record.outputTokens}</span>
                <span>{formatCost(record.estimatedCost)}</span>
              </button>
            ))}
          </div>
        </section>
        {selected && (
          <aside className="ais-card ais-audit-detail">
            <div className="ais-card-title">
              <div>
                <h3>审计详情 #{selected.id}</h3>
                <p>{formatTime(selected.createdAt)}</p>
              </div>
              <StatusBadge
                label={statusLabel(selected.result)}
                tone={statusTone(selected.result)}
              />
            </div>
            <dl className="ais-fact-list">
              <div>
                <dt>运行 ID</dt>
                <dd>{selected.runId ?? '无'}</dd>
              </div>
              <div>
                <dt>板卡</dt>
                <dd>{selected.boardId || '无板卡作用域'}</dd>
              </div>
              <div>
                <dt>服务 / 模型</dt>
                <dd>
                  {provider
                    ? `${provider.name} / ${provider.model}`
                    : `Provider #${selected.providerId ?? '未记录'}`}
                </dd>
              </div>
              <div>
                <dt>Token</dt>
                <dd>
                  {selected.inputTokens} 输入 / {selected.outputTokens} 输出
                </dd>
              </div>
            </dl>
            <section className="ais-evidence ais-evidence--fact">
              <h4>系统事实 · 脱敏输入</h4>
              <JsonFacts value={selected.redactedInput} />
            </section>
            <section className="ais-evidence ais-evidence--inference">
              <h4>AI 推断 · 结构化输出</h4>
              <JsonFacts value={selected.output} />
            </section>
            <p className="ais-safety-line">
              <LynxIcon name={'key'} size={16} /> API Key 和请求头不写入审计库。
            </p>
          </aside>
        )}
      </div>
    </div>
  );
};

export const LegacyRunTable: React.FC<{
  runs: SupervisorRun[];
  rounds: SupervisorRound[];
  selectedRunId: string;
  onSelectRun: (runId: string) => void;
  onApprove: (run: SupervisorRun) => Promise<void>;
  onControl: (
    runId: string,
    action: 'pause' | 'resume' | 'takeover' | 'terminate'
  ) => Promise<void>;
}> = ({ runs, rounds, selectedRunId, onSelectRun, onApprove, onControl }) => {
  const selected = runs.find((run) => run.id === selectedRunId) ?? runs[0];
  if (!selected)
    return (
      <EmptyState
        icon={'runtime'}
        title="当前没有监督运行"
        detail="任务失败且匹配监督策略后，实时恢复轮次会在这里展开。"
      />
    );
  return (
    <div className="ais-runtime-layout">
      <aside className="ais-card ais-list ais-run-selector">
        <div className="ais-card-title">
          <div>
            <h3>监督任务</h3>
            <p>{runs.length} 条运行记录</p>
          </div>
        </div>
        {runs.map((run) => (
          <button
            key={run.id}
            className={run.id === selected.id ? 'selected' : ''}
            onClick={() => onSelectRun(run.id)}
          >
            <LynxIcon name={'agent'} size={16} />
            <span>
              <strong>{run.taskId}</strong>
              <small>
                {run.boardId} · {formatTime(run.startedAt)}
              </small>
            </span>
            <StatusBadge label={statusLabel(run.status)} tone={statusTone(run.status)} />
          </button>
        ))}
      </aside>
      <main className="ais-card ais-round-workspace">
        <div className="ais-card-title">
          <div>
            <h3>{selected.taskId}</h3>
            <p>
              {selected.boardId} · 运行 {selected.id}
            </p>
          </div>
          <StatusBadge label={statusLabel(selected.status)} tone={statusTone(selected.status)} />
        </div>
        {rounds.length === 0 ? (
          <EmptyState
            compact
            icon={'list-check'}
            title="暂无恢复回合"
            detail="该运行还没有发起 AI 决策，或回合记录尚未加载。"
          />
        ) : (
          <div className="ais-round-timeline">
            {rounds.map((round) => (
              <article key={round.id}>
                <div className="ais-round-marker">{round.roundIndex}</div>
                <div className="ais-round-body">
                  <div className="ais-round-title">
                    <strong>
                      {ACTION_LABELS[round.decision.decision] ?? round.decision.decision}
                    </strong>
                    <span>
                      {formatTime(round.createdAt)} · 置信度{' '}
                      {Math.round(round.decision.confidence * 100)}%
                    </span>
                  </div>
                  <div className="ais-evidence-grid">
                    <section className="ais-evidence ais-evidence--fact">
                      <h4>系统事实</h4>
                      <p className="ais-fingerprint">{round.failureFingerprint || '无错误指纹'}</p>
                      <JsonFacts value={round.redactedContext} />
                    </section>
                    <section className="ais-evidence ais-evidence--inference">
                      <h4>AI 推断</h4>
                      <p>{round.decision.reason}</p>
                      {round.decision.evidence.length > 0 && (
                        <ul>
                          {round.decision.evidence.map((item) => (
                            <li key={item}>{item}</li>
                          ))}
                        </ul>
                      )}
                    </section>
                    <section className="ais-evidence ais-evidence--policy">
                      <h4>策略校验</h4>
                      <p>Schema：{round.schemaResult || '未记录'}</p>
                      <p>Policy：{round.policyResult || '未记录'}</p>
                    </section>
                    <section className="ais-evidence ais-evidence--result">
                      <h4>执行结果</h4>
                      <p>{round.actionResult || '尚未执行'}</p>
                      <p>验收：{round.verificationResult || '尚未验收'}</p>
                    </section>
                  </div>
                </div>
              </article>
            ))}
          </div>
        )}
      </main>
      <aside className="ais-card ais-run-control">
        <h3>运行预算</h3>
        <div className="ais-budget-facts ais-budget-facts--stack">
          <span>
            <strong>{selected.attemptCount}</strong>AI 尝试
          </span>
          <span>
            <strong>{selected.powerCycleCount}</strong>电源循环
          </span>
          <span>
            <strong>{selected.flashRetryCount}</strong>烧录重试
          </span>
          <span>
            <strong>{selected.totalInputTokens + selected.totalOutputTokens}</strong>Token
          </span>
          <span>
            <strong>{formatCost(selected.estimatedCost)}</strong>估算费用
          </span>
        </div>
        <h3>受控操作</h3>
        <div className="ais-control-stack">
          {selected.status === 'waiting_approval' && (
            <button className="primary" onClick={() => void onApprove(selected)}>
              <LynxIcon name={'check-circle'} size={16} /> 批准当前动作
            </button>
          )}
          <button
            onClick={() =>
              void onControl(selected.id, selected.status === 'paused' ? 'resume' : 'pause')
            }
          >
            <LynxIcon name={selected.status === 'paused' ? 'play' : 'pause'} size={16} />{' '}
            {selected.status === 'paused' ? '恢复监督' : '暂停监督'}
          </button>
          <button onClick={() => void onControl(selected.id, 'takeover')}>
            <LynxIcon name={'person-settings'} size={16} /> 人工接管
          </button>
          <button className="danger" onClick={() => void onControl(selected.id, 'terminate')}>
            <LynxIcon name={'prohibited'} size={16} /> 终止监督
          </button>
        </div>
        <p className="ais-note">暂停或终止只更改监督状态，不强制打断已授权的硬件步骤。</p>
      </aside>
    </div>
  );
};

export const LegacyHumanIntervention: React.FC<{
  requests: SupervisorHumanRequest[];
  runs: SupervisorRun[];
  onResolve: (requestId: number, resolution: 'continue' | 'manual' | 'terminate') => Promise<void>;
}> = ({ requests, runs, onResolve }) => {
  const [selectedId, setSelectedId] = useState<number | null>(requests[0]?.id ?? null);
  useEffect(() => {
    if (!requests.some((request) => request.id === selectedId))
      setSelectedId(requests[0]?.id ?? null);
  }, [requests, selectedId]);
  const selected = requests.find((request) => request.id === selectedId) ?? requests[0];
  const run = runs.find((item) => item.id === selected?.runId);
  if (!selected)
    return (
      <EmptyState
        icon={'check-circle'}
        title="没有待处理的人工介入"
        detail="监督器未遇到超出白名单、预算或硬件自动化边界的问题。"
      />
    );
  return (
    <div className="ais-human-layout">
      <aside className="ais-card ais-list">
        <div className="ais-card-title">
          <div>
            <h3>介入队列</h3>
            <p>{requests.filter((item) => item.status === 'waiting').length} 项待处理</p>
          </div>
        </div>
        {requests.map((request) => (
          <button
            key={request.id}
            className={request.id === selected.id ? 'selected' : ''}
            onClick={() => setSelectedId(request.id)}
          >
            <LynxIcon name={'person-settings'} size={16} />
            <span>
              <strong>{request.reason}</strong>
              <small>{formatTime(request.createdAt)}</small>
            </span>
            <StatusBadge
              label={request.status === 'waiting' ? '待处理' : '已处理'}
              tone={request.status === 'waiting' ? 'warning' : 'neutral'}
            />
          </button>
        ))}
      </aside>
      <main className="ais-card ais-human-detail">
        <div className="ais-card-title">
          <div>
            <h3>{selected.reason}</h3>
            <p>
              {run?.taskId ?? selected.runId} · {run?.boardId ?? '板卡信息不可用'}
            </p>
          </div>
          <StatusBadge
            label={selected.status === 'waiting' ? '需要你处理' : '已结束'}
            tone={selected.status === 'waiting' ? 'warning' : 'neutral'}
          />
        </div>
        <section className="ais-instruction">
          <LynxIcon name={'person-settings'} size={16} />
          <div>
            <h4>请执行以下操作</h4>
            <p>{selected.instruction}</p>
          </div>
        </section>
        <div className="ais-human-evidence">
          <section className="ais-evidence ais-evidence--fact">
            <h4>系统事实</h4>
            {selected.evidence.length ? (
              <ul>
                {selected.evidence.map((item) => (
                  <li key={item}>{item}</li>
                ))}
              </ul>
            ) : (
              <p>未记录额外系统证据。</p>
            )}
          </section>
          <section className="ais-evidence ais-evidence--inference">
            <h4>监督结论</h4>
            <p>{selected.reason}</p>
            <small>这是请求人工介入的原因，不代表硬件状态已被验收。</small>
          </section>
        </div>
        {selected.resolution && (
          <section className="ais-resolution">
            <strong>处理结果</strong>
            <p>{selected.resolution}</p>
            <span>{formatTime(selected.resolvedAt)}</span>
          </section>
        )}
        {selected.status === 'waiting' && (
          <div className="ais-actions ais-human-actions">
            <button className="primary" onClick={() => void onResolve(selected.id, 'continue')}>
              <LynxIcon name={'check-circle'} size={16} /> 已处理，继续任务
            </button>
            <button onClick={() => void onResolve(selected.id, 'manual')}>
              <LynxIcon name={'person-settings'} size={16} /> 转为手动模式
            </button>
            <button className="danger" onClick={() => void onResolve(selected.id, 'terminate')}>
              <LynxIcon name={'prohibited'} size={16} /> 终止任务
            </button>
          </div>
        )}
      </main>
    </div>
  );
};

export default AISupervisor;
