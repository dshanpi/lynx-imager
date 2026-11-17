import React, { useRef } from 'react';
import { LynxIcon } from '../../CoreUI';
import type { DeviceFlowSequenceNodeView } from './DeviceFlowSequence';

export type DeviceFlowPaletteNode = DeviceFlowSequenceNodeView & {
  deviceDomain?: 'linux' | 'mcu';
  highRisk?: boolean;
  executable?: boolean;
  config?: Record<string, unknown>;
};

interface DeviceFlowPaletteProps {
  mode: 'sequence' | 'canvas';
  search: string;
  groups: Array<{ group: string; items: DeviceFlowPaletteNode[] }>;
  onSearchChange: (value: string) => void;
  onAddNode: (kind: DeviceFlowPaletteNode) => void;
}

export const DeviceFlowPalette: React.FC<DeviceFlowPaletteProps> = ({
  mode,
  search,
  groups,
  onSearchChange,
  onAddNode,
}) => {
  const paletteDragRef = useRef(false);
  return (
    <>
      <div className="ais-flow-palette-title">
        <h3>{mode === 'sequence' ? '步骤库' : '节点库'}</h3>
        <span>{mode === 'sequence' ? '点击后自动接续' : '点击添加或拖入'}</span>
      </div>
      <label className="ais-flow-search">
        <LynxIcon name="search" size={16} />
        <input
          value={search}
          onChange={(event) => onSearchChange(event.target.value)}
          placeholder="搜索设备能力"
        />
      </label>
      <div className="ais-flow-palette">
        {groups.map(({ group, items }) => (
          <details key={`${mode}-${group}`} open={mode === 'canvas' || group === '串口交互'}>
            <summary>
              {group}
              <LynxIcon name="chevron-down" size={16} />
            </summary>
            {items.map((kind) => (
              <div
                key={kind.type}
                draggable={mode === 'canvas' && kind.executable !== false}
                role="button"
                tabIndex={kind.executable === false ? -1 : 0}
                aria-disabled={kind.executable === false}
                aria-label={`${kind.label}：${kind.description}`}
                onClick={() => {
                  if (!paletteDragRef.current) onAddNode(kind);
                }}
                onKeyDown={(event) => {
                  if (event.key === 'Enter' || event.key === ' ') {
                    event.preventDefault();
                    onAddNode(kind);
                  }
                }}
                onDragStart={(event) => {
                  if (kind.executable === false) {
                    event.preventDefault();
                    return;
                  }
                  paletteDragRef.current = true;
                  event.dataTransfer.effectAllowed = 'copy';
                  event.dataTransfer.setData('application/x-lynx-flow-node', kind.type);
                  // WebKit on Windows may expose only a standard text payload.
                  event.dataTransfer.setData('text/plain', `lynx-flow-node:${kind.type}`);
                }}
                onDragEnd={() =>
                  window.setTimeout(() => {
                    paletteDragRef.current = false;
                  }, 0)
                }
                className={`ais-palette-node ais-node--${kind.tone} ${kind.executable === false ? 'is-unavailable' : ''}`}
              >
                <LynxIcon name={kind.icon} size={20} />
                <span>
                  <strong>{kind.label}</strong>
                  <small>
                    {kind.description}
                    {kind.executable === false ? ' · 尚未接入执行器' : ''}
                  </small>
                </span>
              </div>
            ))}
          </details>
        ))}
      </div>
    </>
  );
};
