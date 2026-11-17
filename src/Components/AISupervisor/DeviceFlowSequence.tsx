import React from 'react';
import { LynxIcon, type LynxIconName } from '../../CoreUI';
import type { DeviceFlowNode, DeviceFlowTemplate } from './deviceFlowModel';

export type DeviceFlowSequenceNodeView = {
  type: string;
  label: string;
  group: string;
  description: string;
  tone: string;
  icon: LynxIconName;
};

type BoardOption = { value: string; label: string };

interface DeviceFlowSequenceProps {
  flow: DeviceFlowTemplate;
  sequence: DeviceFlowNode[];
  boardId: string;
  boardOptions: { linux: BoardOption[]; mcu: BoardOption[] };
  selectedNodeId: string;
  insertAfterNodeId: string | null;
  insertKinds: DeviceFlowSequenceNodeView[];
  nodeView: (type: string) => DeviceFlowSequenceNodeView | undefined;
  boardDisplayName: (scope: string) => string;
  onBindBoard: (boardId: string) => void;
  onApplySerialLoginTemplate: () => void;
  onCreateBlankTemplate: () => void;
  onInsertMenuChange: (afterNodeId: string | null) => void;
  onInsertNode: (kind: DeviceFlowSequenceNodeView, afterNodeId?: string) => void;
  onSelectNode: (nodeId: string) => void;
  onRemoveNode: (nodeId: string) => void;
}

export const DeviceFlowSequence: React.FC<DeviceFlowSequenceProps> = ({
  flow,
  sequence,
  boardId,
  boardOptions,
  selectedNodeId,
  insertAfterNodeId,
  insertKinds,
  nodeView,
  boardDisplayName,
  onBindBoard,
  onApplySerialLoginTemplate,
  onCreateBlankTemplate,
  onInsertMenuChange,
  onInsertNode,
  onSelectNode,
  onRemoveNode,
}) => (
  <div className="ais-flow-sequence">
    {!flow.nodes.length ? (
      <div className="ais-flow-onboarding">
        <div className="ais-flow-onboarding-heading">
          <span>1</span>
          <div>
            <strong>选择这项任务要操作的设备</strong>
            <small>任务始终固定到这块单板，运行时不会临时换设备。</small>
          </div>
        </div>
        <select
          aria-label="新任务固定设备"
          value={boardId}
          onChange={(event) => onBindBoard(event.target.value)}
        >
          <option value="">请选择已保存的单板</option>
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
        <div className="ais-flow-onboarding-heading">
          <span>2</span>
          <div>
            <strong>选择一个起点</strong>
            <small>常用流程直接生成完整步骤，之后逐项修改即可。</small>
          </div>
        </div>
        <div className="ais-flow-starters">
          <button
            className="primary"
            disabled={!boardId.startsWith('linux:')}
            onClick={onApplySerialLoginTemplate}
          >
            <LynxIcon name="terminal" size={20} />
            <span>
              <strong>串口自动登录</strong>
              <small>打开串口 → 登录 Shell → 验收结果</small>
            </span>
          </button>
          <button onClick={onCreateBlankTemplate}>
            <LynxIcon name="flow" size={20} />
            <span>
              <strong>空白顺序流程</strong>
              <small>手动触发 → 添加步骤 → 验收结果</small>
            </span>
          </button>
        </div>
      </div>
    ) : (
      <>
        <header className="ais-flow-sequence-header">
          <div>
            <strong>执行顺序</strong>
            <small>点击步骤直接配置；新增和删除会自动维护前后关系。</small>
          </div>
          <button onClick={() => onInsertMenuChange(insertAfterNodeId === '' ? null : '')}>
            <LynxIcon name="add" size={16} /> 添加步骤
          </button>
        </header>
        {insertAfterNodeId === '' && (
          <SequenceInsertMenu
            kinds={insertKinds}
            onSelect={(kind) => onInsertNode(kind)}
            onCancel={() => onInsertMenuChange(null)}
          />
        )}
        <div className="ais-flow-sequence-list">
          {sequence.map((node, index) => {
            const kind = nodeView(node.type);
            const branches = flow.edges
              .filter((edge) => edge.source === node.id && edge.branch === 'failure')
              .map((edge) => flow.nodes.find((item) => item.id === edge.target))
              .filter((item): item is DeviceFlowNode => Boolean(item));
            return (
              <React.Fragment key={node.id}>
                <article
                  className={`ais-sequence-step ais-node--${kind?.tone ?? 'control'} ${selectedNodeId === node.id ? 'selected' : ''}`}
                  onClick={() => onSelectNode(node.id)}
                >
                  <span className="ais-sequence-index">{String(index + 1).padStart(2, '0')}</span>
                  <LynxIcon name={kind?.icon ?? 'flow'} size={20} />
                  <div>
                    <small>{kind?.group ?? '兼容节点'}</small>
                    <strong>{node.label}</strong>
                    <span>
                      {node.config.boardId
                        ? boardDisplayName(String(node.config.boardId))
                        : (kind?.description ?? '')}
                    </span>
                    {branches.map((branch) => (
                      <em key={branch.id}>失败 → {branch.label}</em>
                    ))}
                  </div>
                  <button
                    className="danger"
                    aria-label={`删除步骤${node.label}`}
                    title="删除并自动接续前后步骤"
                    onClick={(event) => {
                      event.stopPropagation();
                      onRemoveNode(node.id);
                    }}
                  >
                    <LynxIcon name="delete" size={16} />
                  </button>
                </article>
                {!node.type.startsWith('result.') && (
                  <div className="ais-sequence-insert">
                    <button
                      onClick={() =>
                        onInsertMenuChange(insertAfterNodeId === node.id ? null : node.id)
                      }
                    >
                      <LynxIcon name="add" size={16} /> 在此后添加步骤
                    </button>
                    {insertAfterNodeId === node.id && (
                      <SequenceInsertMenu
                        kinds={insertKinds}
                        onSelect={(kind) => onInsertNode(kind, node.id)}
                        onCancel={() => onInsertMenuChange(null)}
                      />
                    )}
                  </div>
                )}
              </React.Fragment>
            );
          })}
        </div>
        {flow.nodes.length > sequence.length && (
          <div className="ais-flow-branch-note">
            另有 {flow.nodes.length - sequence.length} 个分支节点；可在“高级画布”查看和调整分支。
          </div>
        )}
      </>
    )}
  </div>
);

const SequenceInsertMenu = ({
  kinds,
  onSelect,
  onCancel,
}: {
  kinds: DeviceFlowSequenceNodeView[];
  onSelect: (kind: DeviceFlowSequenceNodeView) => void;
  onCancel: () => void;
}) => (
  <div className="ais-sequence-add-menu">
    <div>
      <strong>选择要插入的步骤</strong>
      <button aria-label="关闭步骤选择" onClick={onCancel}>
        <LynxIcon name="dismiss" size={16} />
      </button>
    </div>
    <div className="ais-sequence-add-grid">
      {kinds.map((kind) => (
        <button key={kind.type} onClick={() => onSelect(kind)}>
          <LynxIcon name={kind.icon} size={20} />
          <span>
            <strong>{kind.label}</strong>
            <small>{kind.description}</small>
          </span>
        </button>
      ))}
    </div>
  </div>
);
