import React from 'react';
import { LynxIcon } from '../../CoreUI';
import type { SupervisorAutomaticTask } from '../../Platform/IPC/Commands';

interface DeviceFlowTaskListProps {
  tasks: SupervisorAutomaticTask[];
  selectedTaskId: number | null;
  boardDisplayName: (scope: string) => string;
  onNewTask: () => void;
  onSelectTask: (task: SupervisorAutomaticTask) => void;
  children: React.ReactNode;
}

export const DeviceFlowTaskList: React.FC<DeviceFlowTaskListProps> = ({
  tasks,
  selectedTaskId,
  boardDisplayName,
  onNewTask,
  onSelectTask,
  children,
}) => (
  <aside className="ais-flow-task-list ais-card">
    <div className="ais-card-title">
      <div>
        <h3>设备任务</h3>
        <p>{tasks.length} 个已保存流程</p>
      </div>
      <button aria-label="新建设备流程" title="新建流程" onClick={onNewTask}>
        <LynxIcon name="add" size={16} />
      </button>
    </div>
    <div className="ais-flow-tasks">
      {tasks.map((task) => (
        <button
          key={task.id}
          data-testid={`ai-automatic-task-${task.id}`}
          className={task.id === selectedTaskId ? 'selected' : ''}
          onClick={() => onSelectTask(task)}
        >
          <LynxIcon name="flow" size={20} />
          <span>
            <strong>{task.name}</strong>
            <small>
              {task.enabled ? '已启用' : '草稿'} · {boardDisplayName(task.boardId)}
            </small>
          </span>
          <i className={task.enabled ? 'online' : ''} />
        </button>
      ))}
      {!tasks.length && <p className="ais-empty-copy">还没有设备自动化任务。</p>}
    </div>
    {children}
  </aside>
);
