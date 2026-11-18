import { useEffect, useState } from 'react';
import { confirm } from '@tauri-apps/plugin-dialog';
import { invokeCommand } from '../../Platform/IPC';
import type {
  AiProviderConfig,
  SaveAiProviderRequest,
  SupervisorPolicy,
  WorkbenchDevice,
} from '../../Platform/IPC/Commands';
import {
  ACTION_LABELS,
  PRESETS,
  defaultPolicy,
  defaultProvider,
  providerToDraft,
} from '../AISupervisor/supervisorConfiguration';
import {
  AIFacts,
  AIMessage,
  AITable,
  safeDetail,
  timeLabel,
  useAIAction,
} from './imagerAIControls';

type Section = 'services' | 'policy' | 'notifications';
const reviewActions = new Set(['retry_flash', 'reset_board', 'power_cycle']);
const budgets = [
  ['maxAttempts', '最多 AI 尝试'],
  ['maxDurationSeconds', '监督时长（秒）'],
  ['maxSameError', '同一错误上限'],
  ['waitDeviceSeconds', '等待设备（秒）'],
  ['maxPowerCycles', '电源循环上限'],
  ['maxFlashRetries', '重新烧录上限'],
] as const;

export function ImagerAIConfiguration({
  section,
  devices,
  device,
}: {
  section: Section;
  devices: WorkbenchDevice[];
  device: WorkbenchDevice | null;
}) {
  const [providers, setProviders] = useState<AiProviderConfig[]>([]);
  const [policies, setPolicies] = useState<SupervisorPolicy[]>([]);
  const [provider, setProvider] = useState(defaultProvider);
  const [policy, setPolicy] = useState(defaultPolicy);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState('');
  const [retry, setRetry] = useState(0);
  const { busy, notice, run } = useAIAction([provider.apiKey || '']);
  useEffect(() => {
    let current = true;
    setLoading(true);
    setLoadError('');
    // A provider page can load independently of the policy store, and vice versa.
    const load = async () => {
      const [connections, rules] = await Promise.all([
        section === 'notifications' ? [] : invokeCommand('supervisor_list_providers'),
        section === 'services' ? [] : invokeCommand('supervisor_list_policies'),
      ]);
      if (!current) return;
      setProviders(connections);
      setPolicies(rules);
      if (connections.length)
        setProvider(providerToDraft(connections.find((p) => p.isDefault) || connections[0]));
      const scoped = rules.find((p) => p.boardScope.includes('linux:' + device?.id)) || rules[0];
      setPolicy(scoped || defaultPolicy());
    };
    void load()
      .catch((e) => {
        if (current) setLoadError(safeDetail(e));
      })
      .finally(() => {
        if (current) setLoading(false);
      });
    return () => {
      current = false;
    };
  }, [section, retry, device?.id]);

  const saved = providers.find((p) => p.id === provider.id);
  const providerIsDirty =
    !saved || JSON.stringify(providerToDraft(saved)) !== JSON.stringify(provider);
  const lock = busy || loading || !!loadError;
  const providerField = <K extends keyof SaveAiProviderRequest>(
    key: K,
    value: SaveAiProviderRequest[K]
  ) => setProvider((p) => ({ ...p, [key]: value }));
  const policyField = <K extends keyof SupervisorPolicy>(key: K, value: SupervisorPolicy[K]) =>
    setPolicy((p) => ({ ...p, [key]: value }));
  const policySelect = policies.length > 0 && (
    <label>
      选择策略
      <select
        value={policy.id ?? ''}
        onChange={(e) => {
          const selected = policies.find((p) => p.id === Number(e.target.value));
          if (selected) setPolicy(selected);
        }}
      >
        {!policy.id && <option value="">新策略</option>}
        {policies.map((p) => (
          <option key={p.id} value={p.id!}>
            {p.name}
          </option>
        ))}
      </select>
    </label>
  );

  async function saveProvider() {
    const destination = new URL(provider.baseUrl);
    if (
      !['http:', 'https:'].includes(destination.protocol) ||
      destination.username ||
      destination.password ||
      destination.search ||
      destination.hash
    ) {
      throw new Error('Base URL 应为服务地址，凭据请填写到 API Key。');
    }
    if (
      saved?.hasApiKey &&
      new URL(saved.baseUrl).origin !== destination.origin &&
      !provider.apiKey?.trim()
    ) {
      throw new Error('服务主机已更改，请为新主机填写 API Key。');
    }
    const next = await invokeCommand('supervisor_save_provider', { request: provider });
    setProvider(providerToDraft(next));
    setProviders(await invokeCommand('supervisor_list_providers'));
    return '服务连接已保存。';
  }
  async function savePolicy() {
    if (policy.allowedActions.some((a) => reviewActions.has(a)))
      throw new Error('写入、复位和电源循环须选择人工批准或禁止。');
    const next = await invokeCommand('supervisor_save_policy', { policy });
    setPolicy(next);
    setPolicies(await invokeCommand('supervisor_list_policies'));
    return '策略与动作权限已保存。';
  }
  async function saveNotification() {
    const latest = (await invokeCommand('supervisor_list_policies')).find(
      (p) => p.id === policy.id
    );
    if (!latest) throw new Error('请先创建或选择一个已保存的策略。');
    const next = await invokeCommand('supervisor_save_policy', {
      policy: {
        ...latest,
        notificationMode: policy.notificationMode,
        successSummary: policy.successSummary,
      },
    });
    setPolicy(next);
    setPolicies((items) => items.map((p) => (p.id === next.id ? next : p)));
    return '通知设置已保存。';
  }

  return (
    <>
      {loadError && (
        <AIMessage>
          {loadError} <button onClick={() => setRetry((n) => n + 1)}>重新读取配置</button>
        </AIMessage>
      )}
      {loading && <AIMessage>正在读取配置…</AIMessage>}
      <fieldset className="im-ai-content-fields" disabled={lock}>
        {section === 'services' ? (
          <>
            <section>
              <h2>服务连接</h2>
              <AITable
                columns={['连接名称', '模型', '用途', '上次测试']}
                empty={!providers.length ? '尚无已保存的模型服务。' : ''}
              >
                {providers.map((p) => (
                  <tr key={p.id} className={p.id === provider.id ? 'im-ai-selected' : ''}>
                    <td>
                      <button
                        className="im-ai-text"
                        onClick={() => setProvider(providerToDraft(p))}
                      >
                        {p.name}
                      </button>
                    </td>
                    <td>
                      <code>{p.model}</code>
                    </td>
                    <td>{!p.enabled ? '已停用' : p.isDefault ? '默认连接' : '备用连接'}</td>
                    <td>
                      {p.lastTestAt
                        ? timeLabel(p.lastTestAt * 1000) + ' · ' + p.lastTestStatus
                        : '尚未测试'}
                    </td>
                  </tr>
                ))}
              </AITable>
              <div className="im-ai-actions">
                <button
                  onClick={() =>
                    setProvider({ ...defaultProvider(), isDefault: !providers.length })
                  }
                >
                  新建连接
                </button>
              </div>
            </section>
            <form
              onSubmit={(e) => {
                e.preventDefault();
                void run(saveProvider);
              }}
            >
              <div className="im-ai-columns">
                <section>
                  <h2>{provider.id ? '编辑 ' + provider.name : '新建服务连接'}</h2>
                  <div className="im-ai-fields">
                    <label>
                      服务预设
                      <select
                        value={provider.preset}
                        onChange={(e) => {
                          const preset = e.target.value;
                          setProvider((p) => ({ ...p, ...PRESETS[preset], preset, apiKey: '' }));
                        }}
                      >
                        {Object.entries(PRESETS).map(([key, v]) => (
                          <option value={key} key={key}>
                            {v.name}
                          </option>
                        ))}
                      </select>
                    </label>
                    <label>
                      连接名称
                      <input
                        required
                        value={provider.name}
                        onChange={(e) => providerField('name', e.target.value)}
                      />
                    </label>
                    <label>
                      Base URL
                      <input
                        type="url"
                        required
                        value={provider.baseUrl}
                        onChange={(e) => providerField('baseUrl', e.target.value)}
                      />
                    </label>
                    <label>
                      运行模型
                      <input
                        required
                        value={provider.model}
                        onChange={(e) => providerField('model', e.target.value)}
                      />
                    </label>
                  </div>
                </section>
                <section>
                  <h2>凭据与运行限制</h2>
                  <label>
                    API Key
                    <input
                      type="password"
                      autoComplete="off"
                      value={provider.apiKey || ''}
                      placeholder={saved?.hasApiKey ? '留空保留已保存的密钥' : '填写服务密钥'}
                      onChange={(e) => providerField('apiKey', e.target.value)}
                    />
                  </label>
                  <div className="im-ai-fields">
                    <label>
                      超时（秒）
                      <input
                        type="number"
                        min="1"
                        required
                        value={provider.timeoutSeconds || ''}
                        onChange={(e) => providerField('timeoutSeconds', Number(e.target.value))}
                      />
                    </label>
                    <label>
                      最大 Token
                      <input
                        type="number"
                        min="1"
                        value={provider.maxTokens ?? ''}
                        onChange={(e) =>
                          providerField('maxTokens', e.target.value ? Number(e.target.value) : null)
                        }
                      />
                    </label>
                  </div>
                  <label className="im-ai-check">
                    <input
                      type="checkbox"
                      checked={provider.enabled}
                      onChange={(e) => providerField('enabled', e.target.checked)}
                    />
                    启用此连接
                  </label>
                  <label className="im-ai-check">
                    <input
                      type="checkbox"
                      checked={provider.isDefault}
                      onChange={(e) => providerField('isDefault', e.target.checked)}
                    />
                    设为默认连接
                  </label>
                </section>
              </div>
              <details>
                <summary>服务费用限制</summary>
                <div className="im-ai-fields">
                  {(
                    [
                      ['taskCostLimit', '单任务费用上限'],
                      ['inputCostPerMillion', '每百万输入 Token 费用'],
                      ['outputCostPerMillion', '每百万输出 Token 费用'],
                    ] as const
                  ).map(([field, label]) => (
                    <label key={field}>
                      {label}
                      <input
                        type="number"
                        step="any"
                        min="0"
                        value={provider[field] ?? ''}
                        onChange={(e) =>
                          providerField(field, e.target.value ? Number(e.target.value) : null)
                        }
                      />
                    </label>
                  ))}
                </div>
              </details>
              <p className="im-ai-muted">API Key 保存在本机系统密钥环。测试使用已保存的连接。</p>
              <div className="im-ai-actions">
                <button
                  type="button"
                  disabled={!provider.id || providerIsDirty}
                  onClick={() =>
                    void run(async () => {
                      const result = await invokeCommand('supervisor_test_provider', {
                        providerId: provider.id!,
                      });
                      setProviders(await invokeCommand('supervisor_list_providers'));
                      return result.ok
                        ? '连接测试通过，耗时 ' + result.latencyMs + ' ms。'
                        : '连接测试失败：' + safeDetail(result.message);
                    })
                  }
                >
                  测试连接
                </button>
                <button className="im-ai-primary" type="submit">
                  保存连接
                </button>
                {provider.id && (
                  <button
                    type="button"
                    onClick={() =>
                      void run(async () => {
                        if (
                          !(await confirm('删除服务连接「' + provider.name + '」？', {
                            kind: 'warning',
                          }))
                        )
                          return;
                        await invokeCommand('supervisor_delete_provider', {
                          providerId: provider.id!,
                        });
                        setProvider(defaultProvider());
                        setRetry((n) => n + 1);
                        return '已删除服务连接。';
                      })
                    }
                  >
                    删除连接
                  </button>
                )}
              </div>
            </form>
          </>
        ) : section === 'policy' ? (
          <form
            onSubmit={(e) => {
              e.preventDefault();
              void run(savePolicy);
            }}
          >
            <div className="im-ai-columns">
              <section>
                <h2>监督策略</h2>
                {policySelect}
                <div className="im-ai-fields">
                  <label>
                    策略名称
                    <input
                      required
                      value={policy.name}
                      onChange={(e) => policyField('name', e.target.value)}
                    />
                  </label>
                  <label>
                    服务连接
                    <select
                      value={policy.providerId ?? ''}
                      onChange={(e) =>
                        policyField('providerId', e.target.value ? Number(e.target.value) : null)
                      }
                    >
                      <option value="">使用默认连接</option>
                      {providers.map((p) => (
                        <option key={p.id} value={p.id!}>
                          {p.name}
                        </option>
                      ))}
                    </select>
                  </label>
                </div>
                {devices.map((d) => {
                  const key = 'linux:' + d.id;
                  return (
                    <label className="im-ai-check" key={key}>
                      <input
                        type="checkbox"
                        checked={policy.boardScope.includes(key)}
                        onChange={(e) =>
                          policyField(
                            'boardScope',
                            e.target.checked
                              ? [...new Set([...policy.boardScope, key])]
                              : policy.boardScope.filter((s) => s !== key)
                          )
                        }
                      />
                      绑定设备 {d.name}
                    </label>
                  );
                })}
                <label className="im-ai-check">
                  <input
                    type="checkbox"
                    checked={policy.enabled}
                    onChange={(e) => policyField('enabled', e.target.checked)}
                  />
                  启用监督策略
                </label>
              </section>
              <section>
                <h2>恢复预算</h2>
                <div className="im-ai-fields">
                  {budgets.map(([field, label]) => (
                    <label key={field}>
                      {label}
                      <input
                        type="number"
                        min="0"
                        required
                        value={policy[field]}
                        onChange={(e) => policyField(field, Number(e.target.value))}
                      />
                    </label>
                  ))}
                  <label>
                    单任务费用上限
                    <input
                      type="number"
                      min="0"
                      step="any"
                      value={policy.costLimit ?? ''}
                      onChange={(e) =>
                        policyField('costLimit', e.target.value ? Number(e.target.value) : null)
                      }
                    />
                  </label>
                </div>
              </section>
            </div>
            <section>
              <h2>动作权限</h2>
              <AITable columns={['动作', '执行模式', '约束']}>
                {Object.entries(ACTION_LABELS).map(([action, label]) => {
                  const mode = policy.allowedActions.includes(action)
                    ? 'auto'
                    : policy.approvalActions.includes(action)
                      ? 'approval'
                      : 'blocked';
                  return (
                    <tr key={action}>
                      <td>{label}</td>
                      <td>
                        <select
                          aria-label={label + '执行模式'}
                          value={mode}
                          onChange={(e) => {
                            const next = e.target.value;
                            setPolicy((p) => ({
                              ...p,
                              allowedActions: [
                                ...p.allowedActions.filter((v) => v !== action),
                                ...(next === 'auto' ? [action] : []),
                              ],
                              approvalActions: [
                                ...p.approvalActions.filter((v) => v !== action),
                                ...(next === 'approval' ? [action] : []),
                              ],
                            }));
                          }}
                        >
                          <option value="auto" disabled={reviewActions.has(action)}>
                            自动允许
                          </option>
                          <option value="approval">需要人工批准</option>
                          <option value="blocked">禁止</option>
                        </select>
                      </td>
                      <td>
                        {action === 'retry_flash'
                          ? '每个任务最多 ' + policy.maxFlashRetries + ' 次，核对固件'
                          : action === 'power_cycle'
                            ? '每个任务最多 ' + policy.maxPowerCycles + ' 次'
                            : mode === 'blocked'
                              ? '当前策略未授权'
                              : reviewActions.has(action)
                                ? '逐次核对请求'
                                : '限定当前任务与设备作用域'}
                      </td>
                    </tr>
                  );
                })}
              </AITable>
            </section>
            <div className="im-ai-actions">
              <button type="button" onClick={() => setPolicy(defaultPolicy())}>
                新建策略
              </button>
              <button type="submit" className="im-ai-primary">
                保存策略与权限
              </button>
            </div>
          </form>
        ) : (
          <>
            <div className="im-ai-columns">
              <section>
                <h2>通知方式</h2>
                {policySelect}
                <label>
                  发送方式
                  <select
                    value={policy.notificationMode}
                    disabled={!policy.id}
                    onChange={(e) => policyField('notificationMode', e.target.value)}
                  >
                    <option value="desktop">LYNX 桌面通知</option>
                    <option value="desktop_email">LYNX + 邮件</option>
                    <option value="none">仅保留人工介入队列</option>
                  </select>
                </label>
                <label className="im-ai-check">
                  <input
                    type="checkbox"
                    disabled={!policy.id}
                    checked={policy.successSummary}
                    onChange={(e) => policyField('successSummary', e.target.checked)}
                  />
                  任务恢复成功后生成摘要通知
                </label>
              </section>
              <section>
                <h2>当前监督策略</h2>
                <AIFacts
                  columns={['项目', '当前设置']}
                  rows={[
                    ['策略名称', policy.id ? policy.name : '尚无已保存策略'],
                    [
                      '固定设备',
                      policy.id
                        ? policy.boardScope
                            .map((id) => devices.find((d) => 'linux:' + d.id === id)?.name || id)
                            .join('、') || '未限定'
                        : '—',
                    ],
                    ['状态', policy.id ? (policy.enabled ? '已启用' : '已停用') : '—'],
                  ]}
                />
              </section>
            </div>
            <section>
              <h2>需要人工介入的事件</h2>
              <AITable columns={['触发条件', '处理']}>
                {[
                  ['任务请求审批动作', '暂停当前步骤，等待操作员处理'],
                  ['恢复次数、时间或费用达到上限', '停止自动恢复并通知操作员'],
                  ['连续出现同一错误且无新证据', '移入人工检查'],
                  ['设备状态无法判定或验收失败', '保留现场记录并请求复核'],
                ].map(([a, b]) => (
                  <tr key={a}>
                    <td>{a}</td>
                    <td>{b}</td>
                  </tr>
                ))}
              </AITable>
            </section>
            <p className="im-ai-muted">
              {policy.id
                ? '收到通知后可进入对应任务，核对证据并处理请求。'
                : '请先在「策略与安全」创建并保存策略。'}
            </p>
            <button
              className="im-ai-primary"
              disabled={!policy.id}
              onClick={() => void run(saveNotification)}
            >
              保存通知设置
            </button>
          </>
        )}
      </fieldset>
      <AIMessage>{notice}</AIMessage>
    </>
  );
}
