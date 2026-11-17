import React from 'react';
import { LynxIcon } from '../../CoreUI';

interface DeviceFlowToolbarProps {
  taskName: string;
  issuesCount: number;
  saveFeedback: string;
  mode: 'sequence' | 'canvas';
  canUndo: boolean;
  canRedo: boolean;
  zoom: number;
  hasNodes: boolean;
  canUseSerialTemplate: boolean;
  hasFixedBoard: boolean;
  taskId: number | null;
  busy: boolean;
  draftSaveIssue?: string;
  onTaskNameChange: (value: string) => void;
  onModeChange: (mode: 'sequence' | 'canvas') => void;
  onUndo: () => void;
  onRedo: () => void;
  onZoomOut: () => void;
  onZoomIn: () => void;
  onAutoLayout: () => void;
  onFitCanvas: () => void;
  onApplySerialTemplate: () => void;
  onTrigger: (taskId: number) => void;
  onSave: () => void;
}

export const DeviceFlowToolbar: React.FC<DeviceFlowToolbarProps> = (props) => {
  const {
    taskName,
    issuesCount,
    saveFeedback,
    mode,
    canUndo,
    canRedo,
    zoom,
    hasNodes,
    canUseSerialTemplate,
    hasFixedBoard,
    taskId,
    busy,
    draftSaveIssue,
    onTaskNameChange,
    onModeChange,
    onUndo,
    onRedo,
    onZoomOut,
    onZoomIn,
    onAutoLayout,
    onFitCanvas,
    onApplySerialTemplate,
    onTrigger,
    onSave,
  } = props;
  return (
    <section className="ais-flow-toolbar ais-card">
      <div className="ais-flow-identity">
        <input
          aria-label="任务名称"
          value={taskName}
          onChange={(event) => onTaskNameChange(event.target.value)}
        />
        <span>{issuesCount ? `${issuesCount} 项待修复` : '流程校验通过'}</span>
        {saveFeedback && <small className="ais-flow-save-feedback">{saveFeedback}</small>}
      </div>
      <div className="ais-flow-toolbar-actions">
        <div className="ais-flow-mode-switch" aria-label="编辑方式">
          <button
            className={mode === 'sequence' ? 'active' : ''}
            onClick={() => onModeChange('sequence')}
          >
            顺序编排
          </button>
          <button
            className={mode === 'canvas' ? 'active' : ''}
            onClick={() => onModeChange('canvas')}
          >
            高级画布
          </button>
        </div>
        {mode === 'canvas' && (
          <>
            <button onClick={onUndo} disabled={!canUndo} aria-label="撤销" title="撤销（Ctrl+Z）">
              <LynxIcon name="arrow-undo" size={16} />
            </button>
            <button onClick={onRedo} disabled={!canRedo} aria-label="重做" title="重做（Ctrl+Y）">
              <LynxIcon name="arrow-redo" size={16} />
            </button>
            <button onClick={onZoomOut} aria-label="缩小">
              <LynxIcon name="zoom-out" size={16} />
            </button>
            <span>{Math.round(zoom * 100)}%</span>
            <button onClick={onZoomIn} aria-label="放大">
              <LynxIcon name="zoom-in" size={16} />
            </button>
            <button onClick={onAutoLayout}>
              <LynxIcon name="flow" size={16} /> 自动竖向布局
            </button>
            <button onClick={onFitCanvas} disabled={!hasNodes} title="适配全部节点">
              <LynxIcon name="fullscreen" size={16} /> 适配视图
            </button>
          </>
        )}
        <button
          disabled={!canUseSerialTemplate}
          title={hasFixedBoard ? '替换当前流程为串口自动登录' : '请先选择固定设备'}
          onClick={onApplySerialTemplate}
        >
          <LynxIcon name="terminal" size={16} /> 串口自动登录
        </button>
        {taskId && (
          <button
            data-testid="ai-automatic-run"
            disabled={busy || issuesCount > 0}
            onClick={() => onTrigger(taskId)}
          >
            <LynxIcon name="play" size={16} /> 立即运行
          </button>
        )}
        <button
          className="primary"
          disabled={busy || Boolean(draftSaveIssue)}
          title={
            draftSaveIssue ?? (issuesCount ? '当前流程未完成，将保存为禁用草稿' : '保存当前流程')
          }
          onClick={onSave}
        >
          <LynxIcon name="save" size={16} /> {issuesCount ? '保存草稿' : '保存流程'}
        </button>
      </div>
    </section>
  );
};
