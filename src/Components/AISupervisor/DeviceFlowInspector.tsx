import React from 'react';
import { LynxIcon } from '../../CoreUI';
import { DeviceFlowNodeFields } from './DeviceFlowNodeFields';
import type { DeviceFlowNode } from './deviceFlowModel';

type BoardOption = { value: string; label: string };
type NamedOption = { value: string | number; label: string };

interface DeviceFlowInspectorProps {
  selectedNode?: DeviceFlowNode;
  selectedNodeDescription?: string;
  selectedNodeDomain?: 'linux' | 'mcu';
  selectedNodeHighRisk: boolean;
  linuxDevices: NamedOption[];
  mcuBoards: NamedOption[];
  boardOptions: { linux: BoardOption[]; mcu: BoardOption[] };
  boardId: string;
  fixedBoardAvailable: boolean;
  policyId: number | null;
  policies: NamedOption[];
  triggerType: string;
  taskId: number | null;
  webhookSecret: string;
  enabled: boolean;
  issues: string[];
  onUpdateNode: (patch: Partial<DeviceFlowNode>) => void;
  onUpdateConfig: (key: string, value: unknown) => void;
  onBindBoard: (boardId: string) => void;
  onPolicyChange: (policyId: number | null) => void;
  onWebhookSecretChange: (value: string) => void;
  onEnabledChange: (enabled: boolean) => void;
  onRemoveNode: (nodeId: string) => void;
  onDeleteTask: (taskId: number) => void;
}

export const DeviceFlowInspector: React.FC<DeviceFlowInspectorProps> = ({
  selectedNode,
  selectedNodeDescription,
  selectedNodeDomain,
  selectedNodeHighRisk,
  linuxDevices,
  mcuBoards,
  boardOptions,
  boardId,
  fixedBoardAvailable,
  policyId,
  policies,
  triggerType,
  taskId,
  webhookSecret,
  enabled,
  issues,
  onUpdateNode,
  onUpdateConfig,
  onBindBoard,
  onPolicyChange,
  onWebhookSecretChange,
  onEnabledChange,
  onRemoveNode,
  onDeleteTask,
}) => (
  <aside className="ais-flow-inspector">
    <section className="ais-card ais-flow-properties">
      <div className="ais-card-title">
        <div>
          <h3>节点配置</h3>
          <p>{selectedNode ? selectedNodeDescription : '选择流程中的步骤'}</p>
        </div>
        {selectedNode && (
          <button
            className="danger"
            aria-label="删除节点"
            onClick={() => onRemoveNode(selectedNode.id)}
          >
            <LynxIcon name="delete" size={16} />
          </button>
        )}
      </div>
      {selectedNode ? (
        <div className="ais-flow-property-form">
          <label>
            节点名称
            <input
              value={selectedNode.label}
              onChange={(event) => onUpdateNode({ label: event.target.value })}
            />
          </label>
          {selectedNodeDomain === 'linux' && (
            <label>
              固定 Linux/SoC 设备
              <select
                value={String(selectedNode.config.boardId ?? '')}
                onChange={(event) => onBindBoard(event.target.value)}
              >
                <option value="">请选择</option>
                {linuxDevices.map((device) => (
                  <option key={device.value} value={`linux:${device.value}`}>
                    {device.label}
                  </option>
                ))}
              </select>
            </label>
          )}
          {selectedNodeDomain === 'mcu' && (
            <label>
              固定 MCU 板卡
              <select
                value={String(selectedNode.config.boardId ?? '')}
                onChange={(event) => onBindBoard(event.target.value)}
              >
                <option value="">请选择</option>
                {mcuBoards.map((board) => (
                  <option key={board.value} value={`mcu:${board.value}`}>
                    {board.label}
                  </option>
                ))}
              </select>
            </label>
          )}
          <DeviceFlowNodeFields node={selectedNode} update={onUpdateConfig} />
          {selectedNodeHighRisk && (
            <label>
              执行审批
              <select
                value={String(selectedNode.config.approvalMode ?? '')}
                onChange={(event) => onUpdateConfig('approvalMode', event.target.value)}
              >
                <option value="required">每次需要人工批准</option>
                <option value="policy">交由策略判定</option>
              </select>
            </label>
          )}
        </div>
      ) : (
        <div className="ais-flow-no-selection">
          <LynxIcon name="flow" size={40} />
          <p>选中节点后，在这里绑定设备、通道、命令、超时和验收条件。</p>
        </div>
      )}
    </section>
    <section className="ais-card ais-flow-task-settings">
      <h3>任务设置</h3>
      <label>
        固定设备
        <select value={boardId} onChange={(event) => onBindBoard(event.target.value)}>
          <option value="">请选择已保存的单板</option>
          {boardId && !fixedBoardAvailable && (
            <option value={boardId} disabled>
              原关联设备不可用
            </option>
          )}
          {boardOptions.linux.length > 0 && (
            <optgroup label="Linux / SoC">
              {boardOptions.linux.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </optgroup>
          )}
          {boardOptions.mcu.length > 0 && (
            <optgroup label="MCU">
              {boardOptions.mcu.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </optgroup>
          )}
        </select>
        {!boardOptions.linux.length && !boardOptions.mcu.length ? (
          <small className="ais-field-warning">
            未发现已保存单板，请先在 Linux 或 MCU 工作台新建单板。
          </small>
        ) : (
          boardId &&
          !fixedBoardAvailable && (
            <small className="ais-field-warning">
              原设备不在当前 Windows 设备库中，请重新选择或先在对应工作台恢复单板配置。
            </small>
          )
        )}
      </label>
      <label>
        监督策略
        <select
          value={policyId ?? ''}
          onChange={(event) =>
            onPolicyChange(event.target.value ? Number(event.target.value) : null)
          }
        >
          <option value="">请选择</option>
          {policies.map((policy) => (
            <option key={policy.value} value={policy.value}>
              {policy.label}
            </option>
          ))}
        </select>
      </label>
      {triggerType === 'webhook' && (
        <label>
          Webhook Bearer 密钥
          <input
            type="password"
            autoComplete="new-password"
            value={webhookSecret}
            placeholder={taskId ? '留空保留密钥环中的值' : '至少 16 个字符'}
            onChange={(event) => onWebhookSecretChange(event.target.value)}
          />
        </label>
      )}
      <label className="ais-check">
        <input
          type="checkbox"
          disabled={issues.length > 0}
          checked={enabled}
          onChange={(event) => onEnabledChange(event.target.checked)}
        />
        校验通过后启用
      </label>
      {taskId && (
        <button className="danger" onClick={() => onDeleteTask(taskId)}>
          <LynxIcon name="delete" size={16} /> 删除任务
        </button>
      )}
    </section>
    <section className={`ais-card ais-flow-validation ${issues.length ? 'has-errors' : ''}`}>
      <div className="ais-card-title">
        <div>
          <h3>流程校验</h3>
          <p>{issues.length ? '启用或运行前必须修复' : '结构、作用域和审批配置完整'}</p>
        </div>
        <span>{issues.length || <LynxIcon name="check" size={16} />}</span>
      </div>
      {issues.length ? (
        <ul>
          {issues.map((issue) => (
            <li key={issue}>{issue}</li>
          ))}
        </ul>
      ) : (
        <p className="ais-validation-ok">
          <LynxIcon name="check" size={16} /> 可保存为已启用任务
        </p>
      )}
    </section>
  </aside>
);
