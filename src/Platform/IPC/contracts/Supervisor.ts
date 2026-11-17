import type { IpcCommandSpec, IpcEventSpec } from '../Types';

export interface AiProviderConfig {
  id: number | null;
  name: string;
  preset: 'openai' | 'deepseek' | 'codingplan' | 'custom' | string;
  baseUrl: string;
  model: string;
  timeoutSeconds: number;
  maxTokens: number | null;
  taskCostLimit: number | null;
  inputCostPerMillion: number | null;
  outputCostPerMillion: number | null;
  enabled: boolean;
  isDefault: boolean;
  hasApiKey: boolean;
  lastTestStatus: string;
  lastTestMessage: string;
  lastTestAt: number | null;
}

export interface SaveAiProviderRequest {
  id: number | null;
  name: string;
  preset: string;
  baseUrl: string;
  model: string;
  timeoutSeconds: number;
  maxTokens: number | null;
  taskCostLimit: number | null;
  inputCostPerMillion: number | null;
  outputCostPerMillion: number | null;
  enabled: boolean;
  isDefault: boolean;
  apiKey?: string | null;
}

export interface SupervisorPolicy {
  id: number | null;
  name: string;
  enabled: boolean;
  providerId: number | null;
  maxAttempts: number;
  maxDurationSeconds: number;
  maxSameError: number;
  maxPowerCycles: number;
  maxFlashRetries: number;
  waitDeviceSeconds: number;
  costLimit: number | null;
  allowedActions: string[];
  approvalActions: string[];
  boardScope: string[];
  successSummary: boolean;
  notificationMode: string;
}

export interface SupervisorAutomaticTask {
  id: number | null;
  name: string;
  enabled: boolean;
  triggerType: 'schedule' | 'webhook' | 'device' | 'manual_api';
  trigger: Record<string, unknown>;
  boardId: string;
  taskTemplate: Record<string, unknown>;
  policyId: number | null;
  successConditions: Array<Record<string, unknown>>;
  nextRunAt: number | null;
  lastRunAt: number | null;
}

export interface SupervisorAuditRecord {
  id: number;
  runId: string | null;
  boardId: string;
  providerId: number | null;
  eventType: string;
  result: string;
  redactedInput: Record<string, unknown>;
  output: Record<string, unknown>;
  inputTokens: number;
  outputTokens: number;
  estimatedCost: number;
  createdAt: number;
}

export interface SupervisorRun {
  id: string;
  taskId: string;
  boardId: string;
  policyId: number;
  status: string;
  attemptCount: number;
  powerCycleCount: number;
  flashRetryCount: number;
  startedAt: number;
  finishedAt: number | null;
  totalInputTokens: number;
  totalOutputTokens: number;
  estimatedCost: number;
  finalReason: string;
}

export interface SupervisorRound {
  id: number;
  runId: string;
  roundIndex: number;
  failureFingerprint: string;
  redactedContext: Record<string, unknown>;
  decision: {
    decision: string;
    reason: string;
    confidence: number;
    evidence: string[];
    parameters: Record<string, unknown>;
    expected_result: string;
    need_human: boolean;
    human_instruction: string | null;
  };
  schemaResult: string;
  policyResult: string;
  actionResult: string;
  verificationResult: string;
  inputTokens: number;
  outputTokens: number;
  estimatedCost: number;
  createdAt: number;
}

export interface SupervisorHumanRequest {
  id: number;
  runId: string;
  status: string;
  reason: string;
  instruction: string;
  evidence: string[];
  resolution: string;
  createdAt: number;
  resolvedAt: number | null;
}

export interface SupervisorRuntimeEvent {
  id?: number;
  taskId?: string;
  runId?: string;
  boardId?: string;
  status?: string;
  source?: string;
  verified?: boolean;
  timeout?: boolean;
  error?: string;
  reason?: string;
}

export interface SupervisorActionResultEvent {
  runId: string;
  roundIndex: number;
  action: string;
  actionResult: Record<string, unknown>;
  verificationResult: Record<string, unknown>;
}

export interface SupervisorCommandMap {
  supervisor_list_providers: IpcCommandSpec<undefined, AiProviderConfig[]>;
  supervisor_save_provider: IpcCommandSpec<{ request: SaveAiProviderRequest }, AiProviderConfig>;
  supervisor_delete_provider: IpcCommandSpec<{ providerId: number }>;
  supervisor_test_provider: IpcCommandSpec<
    { providerId: number },
    {
      ok: boolean;
      status: string;
      message: string;
      latencyMs: number;
      modelsEndpoint: string;
      auditSessionId: string;
    }
  >;
  supervisor_list_policies: IpcCommandSpec<undefined, SupervisorPolicy[]>;
  supervisor_save_policy: IpcCommandSpec<{ policy: SupervisorPolicy }, SupervisorPolicy>;
  supervisor_delete_policy: IpcCommandSpec<{ policyId: number }>;
  supervisor_overview: IpcCommandSpec<
    undefined,
    {
      providerCount: number;
      readyProviderCount: number;
      enabledPolicyCount: number;
      activeRunCount: number;
      waitingHumanCount: number;
      todayCallCount: number;
      todayEstimatedCost: number;
    }
  >;
  supervisor_list_audit: IpcCommandSpec<{ limit?: number }, SupervisorAuditRecord[]>;
  supervisor_list_automatic_tasks: IpcCommandSpec<undefined, SupervisorAutomaticTask[]>;
  supervisor_save_automatic_task: IpcCommandSpec<
    { task: SupervisorAutomaticTask; webhookSecret?: string | null },
    SupervisorAutomaticTask
  >;
  supervisor_delete_automatic_task: IpcCommandSpec<{ taskId: number }>;
  supervisor_save_device_flow_credential: IpcCommandSpec<{
    reference: string;
    password?: string | null;
  }>;
  supervisor_trigger_automatic_task: IpcCommandSpec<
    { taskId: number; operatorApproved?: boolean },
    string
  >;
  supervisor_analyze_failure: IpcCommandSpec<{
    context: {
      runId?: string | null;
      taskId: string;
      boardId: string;
      policyId: number;
      taskKind: string;
      stepName: string;
      failureType: string;
      errorCode?: string | null;
      keyLogs: string[];
      deviceState: Record<string, unknown>;
      firmwareHash?: string | null;
    };
  }>;
  supervisor_list_runs: IpcCommandSpec<{ limit?: number }, SupervisorRun[]>;
  supervisor_list_rounds: IpcCommandSpec<{ runId: string; limit?: number }, SupervisorRound[]>;
  supervisor_control_run: IpcCommandSpec<{
    runId: string;
    action: 'pause' | 'resume' | 'takeover' | 'terminate';
  }>;
  supervisor_list_human_requests: IpcCommandSpec<
    { waitingOnly?: boolean },
    SupervisorHumanRequest[]
  >;
  supervisor_resolve_human_request: IpcCommandSpec<{
    requestId: number;
    resolution: 'continue' | 'manual' | 'terminate';
  }>;
  supervisor_execute_action: IpcCommandSpec<
    { runId: string; roundIndex: number; approved?: boolean },
    SupervisorActionResultEvent
  >;
}

export interface SupervisorEventMap {
  'supervisor-automatic-trigger': IpcEventSpec<SupervisorRuntimeEvent>;
  'supervisor-automatic-result': IpcEventSpec<SupervisorRuntimeEvent>;
  'supervisor-run-update': IpcEventSpec<SupervisorRuntimeEvent>;
  'supervisor-action-result': IpcEventSpec<SupervisorActionResultEvent>;
}
