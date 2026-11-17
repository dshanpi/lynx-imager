import { useLayoutEffect, useState, type CSSProperties, type RefObject } from 'react';
import { LynxIcon, type LynxIconName } from '../../CoreUI';
import type { DeviceFlowEdge, DeviceFlowTemplate } from './deviceFlowModel';

export interface DeviceFlowNodeView {
  tone: string;
  group: string;
  description: string;
  icon: LynxIconName;
  highRisk?: boolean;
}

export interface DeviceFlowLinkDrag {
  source: string;
  pointerId: number;
  x: number;
  y: number;
  target?: string;
  branch?: DeviceFlowEdge['branch'];
}

interface DeviceFlowCanvasProps {
  flow: DeviceFlowTemplate;
  zoom: number;
  canvasRef: RefObject<HTMLDivElement | null>;
  selectedNodeId: string;
  selectedEdgeId: string;
  linkSource: string;
  linkBranch?: DeviceFlowEdge['branch'];
  linkDrag: DeviceFlowLinkDrag | null;
  nodeView: (type: string) => DeviceFlowNodeView | undefined;
  boardDisplayName: (scope: string) => string;
  onClearSelection: () => void;
  onDropNode: (type: string, x: number, y: number) => void;
  onSelectEdge: (edgeId: string) => void;
  onDeleteEdge: (edgeId: string) => void;
  onSelectNode: (nodeId: string) => void;
  onConnectTarget: (nodeId: string) => void;
  onNodeDragStart: (nodeId: string, pointerId: number, offsetX: number, offsetY: number) => void;
  onNodeDragMove: (nodeId: string, pointerId: number, clientX: number, clientY: number) => void;
  onNodeDragEnd: (nodeId: string, pointerId: number) => void;
  onNodeDragCancel: () => void;
  onLinkStart: (drag: DeviceFlowLinkDrag) => void;
  onLinkMove: (drag: DeviceFlowLinkDrag) => void;
  onLinkComplete: (sourceId: string, targetId: string, branch?: DeviceFlowEdge['branch']) => void;
  onLinkPortSelected: (
    sourceId: string,
    branch?: DeviceFlowEdge['branch'],
    toggle?: boolean
  ) => void;
  onLinkCancel: () => void;
}

interface NodeAnchors {
  inputX: number;
  inputY: number;
  outputY: number;
  defaultX: number;
  successX: number;
  failureX: number;
}

const INITIAL_NODE_ANCHORS: NodeAnchors = {
  inputX: 95,
  inputY: 0,
  outputY: 76,
  defaultX: 95,
  successX: 57,
  failureX: 133,
};

export const DeviceFlowCanvas = ({
  flow,
  zoom,
  canvasRef,
  selectedNodeId,
  selectedEdgeId,
  linkSource,
  linkBranch,
  linkDrag,
  nodeView,
  boardDisplayName,
  onClearSelection,
  onDropNode,
  onSelectEdge,
  onDeleteEdge,
  onSelectNode,
  onConnectTarget,
  onNodeDragStart,
  onNodeDragMove,
  onNodeDragEnd,
  onNodeDragCancel,
  onLinkStart,
  onLinkMove,
  onLinkComplete,
  onLinkPortSelected,
  onLinkCancel,
}: DeviceFlowCanvasProps) => {
  const linkEditingEnabled = zoom >= 0.5;
  const portSize = Math.max(36, 32 / zoom);
  const [nodeAnchors, setNodeAnchors] = useState<Record<string, NodeAnchors>>({});
  const nodeIds = flow.nodes.map((node) => node.id).join('|');
  useLayoutEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const nodes = [...canvas.querySelectorAll<HTMLElement>('[data-flow-node-id]')];
    const measure = () => {
      const next: Record<string, NodeAnchors> = {};
      for (const node of nodes) {
        const id = node.dataset.flowNodeId;
        if (!id) continue;
        next[id] = {
          inputX: node.clientLeft + node.clientWidth / 2,
          inputY: node.clientTop,
          outputY: node.clientTop + node.clientHeight,
          defaultX: node.clientLeft + node.clientWidth / 2,
          successX: node.clientLeft + node.clientWidth * 0.3,
          failureX: node.clientLeft + node.clientWidth * 0.7,
        };
      }
      setNodeAnchors((current) =>
        JSON.stringify(current) === JSON.stringify(next) ? current : next
      );
    };
    measure();
    const observer = new ResizeObserver(measure);
    nodes.forEach((node) => observer.observe(node));
    return () => observer.disconnect();
  }, [canvasRef, nodeIds]);
  const stagePoint = (clientX: number, clientY: number) => {
    const canvas = canvasRef.current;
    const rect = canvas?.getBoundingClientRect();
    if (!canvas || !rect) return { x: 0, y: 0 };
    return {
      x: (clientX - rect.left + canvas.scrollLeft) / zoom,
      y: (clientY - rect.top + canvas.scrollTop) / zoom,
    };
  };

  const targetAt = (clientX: number, clientY: number) =>
    document.elementFromPoint(clientX, clientY)?.closest<HTMLElement>('[data-flow-node-id]')
      ?.dataset.flowNodeId;

  return (
    <div
      ref={canvasRef}
      className={`ais-flow-canvas ${linkSource ? 'is-linking' : ''}`}
      onClick={(event) => {
        if (event.target === event.currentTarget) onClearSelection();
      }}
      onDragOver={(event) => {
        if (
          event.dataTransfer.types.includes('application/x-lynx-flow-node') ||
          event.dataTransfer.types.includes('text/plain')
        ) {
          event.preventDefault();
          event.dataTransfer.dropEffect = 'copy';
        }
      }}
      onDrop={(event) => {
        event.preventDefault();
        const customType = event.dataTransfer.getData('application/x-lynx-flow-node');
        const textType = event.dataTransfer.getData('text/plain');
        const type =
          customType ||
          (textType.startsWith('lynx-flow-node:') ? textType.slice('lynx-flow-node:'.length) : '');
        const canvas = canvasRef.current;
        const rect = canvas?.getBoundingClientRect();
        if (type && rect && canvas)
          onDropNode(
            type,
            (event.clientX - rect.left + canvas.scrollLeft) / zoom - 90,
            (event.clientY - rect.top + canvas.scrollTop) / zoom - 34
          );
      }}
    >
      {!linkEditingEnabled && (
        <div className="ais-flow-overview-hint" role="status">
          当前为缩略视图，放大至 50% 可编辑连线。
        </div>
      )}
      <div
        className="ais-flow-stage"
        style={
          {
            transform: `scale(${zoom})`,
            '--ais-port-size': `${portSize}px`,
            '--ais-port-dot-size': `${Math.max(12, 12 / zoom)}px`,
            '--ais-port-radius': `${portSize / 2}px`,
            '--ais-port-label-size': `${Math.max(12, 12 / zoom)}px`,
          } as CSSProperties
        }
      >
        <svg aria-hidden="true" width="1000" height="1600">
          <defs>
            <marker id="ais-arrow" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto">
              <path d="M0,0 L8,4 L0,8 z" />
            </marker>
          </defs>
          {flow.edges.map((edge) => {
            const source = flow.nodes.find((node) => node.id === edge.source);
            const target = flow.nodes.find((node) => node.id === edge.target);
            if (!source || !target) return null;
            const sourceHasBranches =
              source.type === 'verify.deterministic' || source.type === 'control.condition';
            const sourcePort = nodeAnchors[source.id] ?? INITIAL_NODE_ANCHORS;
            const targetPort = nodeAnchors[target.id] ?? INITIAL_NODE_ANCHORS;
            const x1 =
              source.x +
              (sourceHasBranches
                ? edge.branch === 'failure'
                  ? sourcePort.failureX
                  : sourcePort.successX
                : sourcePort.defaultX);
            const y1 = source.y + sourcePort.outputY;
            const x2 = target.x + targetPort.inputX;
            const y2 = target.y + targetPort.inputY;
            const bend = Math.max(45, Math.abs(y2 - y1) / 2);
            const path = `M ${x1} ${y1} C ${x1} ${y1 + bend}, ${x2} ${y2 - bend}, ${x2} ${y2}`;
            return (
              <g key={edge.id}>
                <path
                  className={`ais-flow-edge ${selectedEdgeId === edge.id ? 'selected' : ''}`}
                  d={path}
                  markerEnd="url(#ais-arrow)"
                />
                <path
                  className="ais-flow-edge-hit"
                  style={{ pointerEvents: linkEditingEnabled ? 'stroke' : 'none' }}
                  d={path}
                  onClick={() => onSelectEdge(edge.id)}
                  onDoubleClick={() => onDeleteEdge(edge.id)}
                >
                  <title>单击选中连线，双击删除</title>
                </path>
              </g>
            );
          })}
          {linkDrag &&
            (() => {
              const source = flow.nodes.find((node) => node.id === linkDrag.source);
              if (!source) return null;
              const target = flow.nodes.find((node) => node.id === linkDrag.target);
              const sourcePort = nodeAnchors[source.id] ?? INITIAL_NODE_ANCHORS;
              const targetPort = target ? (nodeAnchors[target.id] ?? INITIAL_NODE_ANCHORS) : null;
              const x1 =
                source.x +
                (linkDrag.branch === 'failure'
                  ? sourcePort.failureX
                  : linkDrag.branch === 'success'
                    ? sourcePort.successX
                    : sourcePort.defaultX);
              const y1 = source.y + sourcePort.outputY;
              const x2 = target && targetPort ? target.x + targetPort.inputX : linkDrag.x;
              const y2 = target && targetPort ? target.y + targetPort.inputY : linkDrag.y;
              const bend = Math.max(35, Math.abs(y2 - y1) / 2);
              return (
                <path
                  className="ais-flow-edge-preview"
                  d={`M ${x1} ${y1} C ${x1} ${y1 + bend}, ${x2} ${y2 - bend}, ${x2} ${y2}`}
                  markerEnd="url(#ais-arrow)"
                />
              );
            })()}
        </svg>
        {flow.nodes.map((node) => {
          const view = nodeView(node.type);
          return (
            <article
              key={node.id}
              data-flow-node-id={node.id}
              className={`ais-flow-node ais-node--${view?.tone ?? 'control'} ${node.id === selectedNodeId ? 'selected' : ''} ${linkDrag?.target === node.id ? 'link-target' : ''}`}
              style={{ left: node.x, top: node.y }}
              onClick={() => (linkSource ? onConnectTarget(node.id) : onSelectNode(node.id))}
              onPointerDown={(event) => {
                if ((event.target as HTMLElement).closest('button')) return;
                const rect = event.currentTarget.getBoundingClientRect();
                event.currentTarget.setPointerCapture(event.pointerId);
                onNodeDragStart(
                  node.id,
                  event.pointerId,
                  (event.clientX - rect.left) / zoom,
                  (event.clientY - rect.top) / zoom
                );
              }}
              onPointerMove={(event) => (
                event.preventDefault(),
                onNodeDragMove(node.id, event.pointerId, event.clientX, event.clientY)
              )}
              onPointerUp={(event) => {
                onNodeDragEnd(node.id, event.pointerId);
                if (event.currentTarget.hasPointerCapture(event.pointerId))
                  event.currentTarget.releasePointerCapture(event.pointerId);
              }}
              onPointerCancel={onNodeDragCancel}
            >
              {linkEditingEnabled && !node.type.startsWith('trigger.') && (
                <button
                  className="ais-node-input"
                  aria-label={`连接到${node.label}`}
                  title="拖到或点击此端口完成连接"
                  onClick={(event) => {
                    event.stopPropagation();
                    onConnectTarget(node.id);
                  }}
                />
              )}
              <header>
                <LynxIcon name={view?.icon ?? 'flow'} size={16} />
                <span>{view?.group ?? '兼容节点'}</span>
                {view?.highRisk && <LynxIcon name="shield" size={16} />}
              </header>
              <strong>{node.label}</strong>
              <small>
                {node.config.boardId
                  ? boardDisplayName(String(node.config.boardId))
                  : (view?.description ?? '')}
              </small>
              {linkEditingEnabled &&
                !node.type.startsWith('result.') &&
                (node.type === 'verify.deterministic' || node.type === 'control.condition'
                  ? (['success', 'failure'] as const)
                  : ([undefined] as const)
                ).map((branch) => (
                  <button
                    key={branch ?? 'default'}
                    className={`ais-node-output ${branch ?? 'default'} ${linkSource === node.id && linkBranch === branch ? 'active' : ''}`}
                    data-branch-label={
                      branch === 'success' ? '成功' : branch === 'failure' ? '失败' : undefined
                    }
                    aria-label={`从${node.label}${branch === 'success' ? '成功分支' : branch === 'failure' ? '失败分支' : ''}开始连线`}
                    title={`${branch === 'success' ? '成功分支：' : branch === 'failure' ? '失败分支：' : ''}按住并拖到目标节点`}
                    onPointerDown={(event) => {
                      event.preventDefault();
                      event.stopPropagation();
                      event.currentTarget.setPointerCapture(event.pointerId);
                      onLinkStart({
                        source: node.id,
                        pointerId: event.pointerId,
                        branch,
                        ...stagePoint(event.clientX, event.clientY),
                      });
                    }}
                    onPointerMove={(event) => {
                      if (
                        !linkDrag ||
                        linkDrag.source !== node.id ||
                        linkDrag.pointerId !== event.pointerId
                      )
                        return;
                      const target = targetAt(event.clientX, event.clientY);
                      onLinkMove({
                        ...linkDrag,
                        ...stagePoint(event.clientX, event.clientY),
                        target: target && target !== node.id ? target : undefined,
                      });
                    }}
                    onPointerUp={(event) => {
                      event.preventDefault();
                      event.stopPropagation();
                      const target = linkDrag?.target ?? targetAt(event.clientX, event.clientY);
                      if (target && target !== node.id) onLinkComplete(node.id, target, branch);
                      else onLinkPortSelected(node.id, branch, false);
                      if (event.currentTarget.hasPointerCapture(event.pointerId))
                        event.currentTarget.releasePointerCapture(event.pointerId);
                      onLinkCancel();
                    }}
                    onPointerCancel={(event) => {
                      if (event.currentTarget.hasPointerCapture(event.pointerId))
                        event.currentTarget.releasePointerCapture(event.pointerId);
                      onLinkCancel();
                    }}
                    onClick={(event) => {
                      event.stopPropagation();
                      if (event.detail === 0) onLinkPortSelected(node.id, branch, true);
                    }}
                  />
                ))}
            </article>
          );
        })}
        {!flow.nodes.length && (
          <div className="ais-flow-empty">
            <LynxIcon name="flow" size={40} />
            <strong>从左侧拖入第一个节点</strong>
            <span>建议从触发器开始，再连接设备动作与确定性验收。</span>
          </div>
        )}
      </div>
    </div>
  );
};
