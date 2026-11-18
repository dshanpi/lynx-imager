import type {
  AiProviderConfig,
  SaveAiProviderRequest,
  SupervisorPolicy,
} from '../../Platform/IPC/Commands';

export const PRESETS: Record<string, Pick<SaveAiProviderRequest, 'name' | 'baseUrl' | 'model'>> = {
  openai: { name: 'OpenAI', baseUrl: 'https://api.openai.com/v1', model: 'gpt-4.1-mini' },
  deepseek: { name: 'DeepSeek', baseUrl: 'https://api.deepseek.com/v1', model: 'deepseek-chat' },
  codingplan: { name: 'Coding Plan', baseUrl: '', model: '' },
  custom: { name: '自定义兼容接口', baseUrl: '', model: '' },
};

export const ACTION_LABELS: Record<string, string> = {
  retry_step: '重试当前步骤',
  rescan_usb: '重新扫描 USB',
  wait_device: '等待设备枚举',
  power_cycle: '板卡电源循环',
  reset_board: '复位板卡',
  reconnect_uart: '重新连接 UART',
  reconnect_adb: '重新连接 ADB',
  check_swd: '检查 SWD',
  retry_flash: '重试相同固件烧录',
  retry_download: '重新下载相同哈希产物',
  retry_build: '重试既有构建步骤',
  collect_logs: '扩大日志窗口',
  finish_success: '提交成功候选（仍需验收器确认）',
  finish_failed: '结束并标记失败',
  request_human: '请求人工介入',
};

export const defaultProvider = (): SaveAiProviderRequest => ({
  id: null,
  preset: 'deepseek',
  ...PRESETS.deepseek,
  timeoutSeconds: 120,
  maxTokens: 4096,
  taskCostLimit: null,
  inputCostPerMillion: null,
  outputCostPerMillion: null,
  enabled: true,
  isDefault: true,
  apiKey: '',
});

export const defaultPolicy = (): SupervisorPolicy => ({
  id: null,
  name: '默认无人值守策略',
  enabled: true,
  providerId: null,
  maxAttempts: 6,
  maxDurationSeconds: 1800,
  maxSameError: 2,
  maxPowerCycles: 3,
  maxFlashRetries: 2,
  waitDeviceSeconds: 30,
  costLimit: null,
  allowedActions: [
    'retry_step',
    'rescan_usb',
    'wait_device',
    'collect_logs',
    'finish_failed',
    'request_human',
  ],
  approvalActions: ['power_cycle', 'reset_board', 'retry_flash'],
  boardScope: [],
  successSummary: true,
  notificationMode: 'desktop',
});

export const providerToDraft = (provider: AiProviderConfig): SaveAiProviderRequest => ({
  id: provider.id,
  name: provider.name,
  preset: provider.preset,
  baseUrl: provider.baseUrl,
  model: provider.model,
  timeoutSeconds: provider.timeoutSeconds,
  maxTokens: provider.maxTokens,
  taskCostLimit: provider.taskCostLimit,
  inputCostPerMillion: provider.inputCostPerMillion,
  outputCostPerMillion: provider.outputCostPerMillion,
  enabled: provider.enabled,
  isDefault: provider.isDefault,
  apiKey: '',
});
