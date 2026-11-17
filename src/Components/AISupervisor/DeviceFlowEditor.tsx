import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { LynxIcon } from '../../CoreUI';
import { formatErrorMessage } from '../../Utils';
import type {
  McuBoardConfig,
  SupervisorAutomaticTask,
  SupervisorPolicy,
  WorkbenchDevice,
} from '../../Platform/IPC/Commands';
import { DeviceFlowCanvas, type DeviceFlowLinkDrag } from './DeviceFlowCanvas';
import { DeviceFlowInspector } from './DeviceFlowInspector';
import { DeviceFlowPalette } from './DeviceFlowPalette';
import { DeviceFlowSequence } from './DeviceFlowSequence';
import { DeviceFlowTaskList } from './DeviceFlowTaskList';
import { DeviceFlowToolbar } from './DeviceFlowToolbar';
import { useDeviceFlowHistory } from './useDeviceFlowHistory';
import {
  buildDeviceFlowBoardOptions,
  compatiblePolicyIdForBoard,
  deviceFlowBoardAvailable,
  deviceFlowBoardDisplayName,
  deviceFlowDraftSaveIssues,
  deviceFlowTaskIssues,
  taskWithDeviceFlow,
} from './deviceFlowTaskModel';
import { createBlankSequenceTemplate, createSerialLoginTemplate } from './deviceFlowTemplates';
import {
  DEVICE_FLOW_NODE_KINDS,
  DEVICE_FLOW_SEQUENCE_INSERT_KINDS,
  deviceFlowNodeKind,
  type DeviceFlowNodeKind,
} from './deviceFlowCatalog';
import {
  autoLayout,
  nodeId,
  placeWithoutCoveringExistingNode,
  primarySequence,
  readDeviceFlow,
  scopeMatchesDomain,
  validateDeviceFlow,
  type DeviceFlowEdge,
  type DeviceFlowNode,
  type DeviceFlowTemplate,
} from './deviceFlowModel';

export type { DeviceFlowEdge, DeviceFlowNode, DeviceFlowTemplate } from './deviceFlowModel';

type NodeKind = DeviceFlowNodeKind;

const nodeKind = deviceFlowNodeKind;

interface DeviceFlowEditorProps {
  tasks: SupervisorAutomaticTask[];
  policies: SupervisorPolicy[];
  devices: WorkbenchDevice[];
  mcuBoards: McuBoardConfig[];
  draft: SupervisorAutomaticTask;
  setDraft: React.Dispatch<React.SetStateAction<SupervisorAutomaticTask>>;
  webhookSecret: string;
  setWebhookSecret: (value: string) => void;
  busy: boolean;
  resetKey: number;
  onNew: () => void;
  onSave: (task: SupervisorAutomaticTask) => Promise<SupervisorAutomaticTask>;
  onDelete: (id: number) => Promise<void>;
  onTrigger: (id: number) => Promise<void>;
}

export const DeviceFlowEditor: React.FC<DeviceFlowEditorProps> = ({
  tasks,
  policies,
  devices,
  mcuBoards,
  draft,
  setDraft,
  webhookSecret,
  setWebhookSecret,
  busy,
  resetKey,
  onNew,
  onSave,
  onDelete,
  onTrigger,
}) => {
  const flowHistory = useDeviceFlowHistory(readDeviceFlow(draft, nodeKind));
  const { flow } = flowHistory;
  const [selectedNodeId, setSelectedNodeId] = useState('');
  const [linkSource, setLinkSource] = useState('');
  const [linkBranch, setLinkBranch] = useState<DeviceFlowEdge['branch']>();
  const [search, setSearch] = useState('');
  const [zoom, setZoom] = useState(1);
  const [dragging, setDragging] = useState<{
    id: string;
    pointerId: number;
    dx: number;
    dy: number;
    start: DeviceFlowTemplate;
  } | null>(null);
  const [selectedEdgeId, setSelectedEdgeId] = useState('');
  const [editorMode, setEditorMode] = useState<'sequence' | 'canvas'>('sequence');
  const [insertAfterNodeId, setInsertAfterNodeId] = useState<string | null>(null);
  const [linkFeedback, setLinkFeedback] = useState('');
  const [saveFeedback, setSaveFeedback] = useState('');
  const lastSavedTaskIdRef = useRef<number | null>(null);
  const [linkDrag, setLinkDrag] = useState<DeviceFlowLinkDrag | null>(null);
  const canvasRef = useRef<HTMLDivElement>(null);
  const selectedNode = flow.nodes.find((node) => node.id === selectedNodeId);
  const flowIssues = useMemo(() => validateDeviceFlow(flow, nodeKind), [flow]);
  const boardOptions = useMemo(
    () => buildDeviceFlowBoardOptions(devices, mcuBoards),
    [devices, mcuBoards]
  );
  const fixedBoardAvailable = deviceFlowBoardAvailable(boardOptions, draft.boardId);
  const boardDisplayName = useCallback(
    (scope: string) => deviceFlowBoardDisplayName(boardOptions, scope),
    [boardOptions]
  );
  const issues = useMemo(
    () =>
      deviceFlowTaskIssues({
        draft,
        flowIssues,
        fixedBoardAvailable,
        policies,
        boardDisplayName,
      }),
    [
      boardDisplayName,
      draft.boardId,
      draft.name,
      draft.policyId,
      fixedBoardAvailable,
      flowIssues,
      policies,
    ]
  );
  const draftSaveIssues = deviceFlowDraftSaveIssues(issues);
  const groups = useMemo(() => {
    const filtered = DEVICE_FLOW_NODE_KINDS.filter((item) =>
      `${item.group} ${item.label} ${item.description}`.toLowerCase().includes(search.toLowerCase())
    );
    return [...new Set(filtered.map((item) => item.group))].map((group) => ({
      group,
      items: filtered.filter((item) => item.group === group),
    }));
  }, [search]);
  const sequence = useMemo(() => primarySequence(flow), [flow]);
  const defaultInsertionSource = [...sequence]
    .reverse()
    .find((node) => !node.type.startsWith('result.'));

  useEffect(() => {
    const next = readDeviceFlow(draft, nodeKind);
    flowHistory.reset(next);
    setSelectedNodeId(next.nodes[0]?.id ?? '');
    setLinkSource('');
    setLinkBranch(undefined);
    setLinkFeedback('');
    if (draft.id === null || draft.id !== lastSavedTaskIdRef.current) setSaveFeedback('');
    setLinkDrag(null);
    setInsertAfterNodeId(null);
    // Switching task identity hydrates its saved graph. Draft field edits are
    // intentionally managed by the local flow state and must not reset it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [draft.id, resetKey]);

  useEffect(() => {
    if (draft.policyId || !policies.length) return;
    const policy =
      policies.find((item) => item.enabled && item.id) ?? policies.find((item) => item.id);
    if (!policy?.id) return;
    setDraft((current) =>
      current.policyId ? current : { ...current, policyId: policy.id ?? null }
    );
  }, [draft.policyId, policies, setDraft]);

  const commitFlow = (next: DeviceFlowTemplate) => {
    flowHistory.commit(next);
    setDraft(taskWithDeviceFlow(next, draft));
    setSaveFeedback('有未保存更改');
  };

  const undoFlow = () => {
    const previous = flowHistory.undo();
    if (!previous) return;
    setDraft(taskWithDeviceFlow(previous, draft));
    setSelectedEdgeId('');
    setLinkSource('');
    setLinkBranch(undefined);
    setLinkFeedback('已撤销上一步画布操作。');
  };

  const redoFlow = () => {
    const next = flowHistory.redo();
    if (!next) return;
    setDraft(taskWithDeviceFlow(next, draft));
    setSelectedEdgeId('');
    setLinkSource('');
    setLinkBranch(undefined);
    setLinkFeedback('已重做画布操作。');
  };

  const addNode = (kind: NodeKind, x: number, y: number) => {
    const id = nodeId();
    const position = placeWithoutCoveringExistingNode(flow.nodes, x, y);
    const next = {
      ...flow,
      nodes: [
        ...flow.nodes,
        {
          id,
          type: kind.type,
          label: kind.label,
          x: position.x,
          y: position.y,
          config: {
            ...(kind.config ?? {}),
            ...(scopeMatchesDomain(draft.boardId, kind.deviceDomain)
              ? { boardId: draft.boardId }
              : {}),
            ...(kind.highRisk ? { approvalMode: 'required' } : {}),
          },
        },
      ],
    };
    commitFlow(next);
    setSelectedNodeId(id);
  };

  const addPaletteNode = (kind: NodeKind) => {
    if (kind.executable === false) return;
    if (editorMode === 'sequence') {
      const selected = flow.nodes.find((node) => node.id === selectedNodeId);
      insertConnectedNode(
        kind,
        selected && !selected.type.startsWith('result.') ? selected.id : defaultInsertionSource?.id
      );
      return;
    }
    const canvas = canvasRef.current;
    const x = canvas ? (canvas.scrollLeft + canvas.clientWidth / 2) / zoom - 95 : 330;
    const y = canvas ? (canvas.scrollTop + canvas.clientHeight / 2) / zoom - 38 : 180;
    addNode(kind, Math.max(10, x), Math.max(10, y));
  };

  const insertConnectedNode = (kind: NodeKind, afterId?: string) => {
    if (kind.executable === false) return;
    const existing = flow.nodes.find((node) => {
      if (kind.type.startsWith('trigger.')) return node.type.startsWith('trigger.');
      if (kind.type.startsWith('result.')) return node.type === kind.type;
      return false;
    });
    if (existing) {
      setSelectedNodeId(existing.id);
      setInsertAfterNodeId(null);
      return;
    }
    const id = nodeId();
    const newNode: DeviceFlowNode = {
      id,
      type: kind.type,
      label: kind.label,
      x: 330,
      y: 180,
      config: {
        ...(kind.config ?? {}),
        ...(scopeMatchesDomain(draft.boardId, kind.deviceDomain) ? { boardId: draft.boardId } : {}),
        ...(kind.highRisk ? { approvalMode: 'required' } : {}),
      },
    };
    let edges = [...flow.edges];
    const sourceId =
      afterId && flow.nodes.some((node) => node.id === afterId) ? afterId : undefined;
    if (sourceId) {
      const outgoing = edges.filter((edge) => edge.source === sourceId);
      const primary =
        outgoing.find((edge) => !edge.branch || edge.branch === 'default') ??
        outgoing.find((edge) => edge.branch === 'success') ??
        outgoing[0];
      if (primary) {
        edges = edges.filter((edge) => edge.id !== primary.id);
        edges.push(
          {
            id: `edge-${sourceId}-${id}`,
            source: sourceId,
            target: id,
            branch: primary.branch,
          },
          { id: `edge-${id}-${primary.target}`, source: id, target: primary.target }
        );
      } else {
        edges.push({ id: `edge-${sourceId}-${id}`, source: sourceId, target: id });
      }
    } else if (kind.type.startsWith('trigger.')) {
      const first = flow.nodes.find((node) => !flow.edges.some((edge) => edge.target === node.id));
      if (first) edges.push({ id: `edge-${id}-${first.id}`, source: id, target: first.id });
    }
    const next = autoLayout({ ...flow, nodes: [...flow.nodes, newNode], edges });
    commitFlow(next);
    setSelectedNodeId(id);
    setInsertAfterNodeId(null);
  };

  const removeAndReconnectNode = (id: string) => {
    const incoming = flow.edges.filter((edge) => edge.target === id);
    const outgoing = flow.edges.filter((edge) => edge.source === id);
    const edges = flow.edges.filter((edge) => edge.source !== id && edge.target !== id);
    if (
      editorMode === 'sequence' &&
      incoming.length === 1 &&
      outgoing.length === 1 &&
      incoming[0].source !== outgoing[0].target
    ) {
      edges.push({
        id: `edge-${incoming[0].source}-${outgoing[0].target}`,
        source: incoming[0].source,
        target: outgoing[0].target,
        branch: incoming[0].branch ?? outgoing[0].branch,
      });
    }
    const next = { ...flow, nodes: flow.nodes.filter((node) => node.id !== id), edges };
    commitFlow(editorMode === 'sequence' ? autoLayout(next) : next);
    setSelectedNodeId(
      editorMode === 'sequence' ? (incoming[0]?.source ?? outgoing[0]?.target ?? '') : ''
    );
    setSelectedEdgeId('');
    setLinkFeedback(
      editorMode === 'sequence'
        ? '已删除步骤并接续前后关系。'
        : '只删除了选中节点及其直接连线，其他节点和位置保持不变。'
    );
    setInsertAfterNodeId(null);
  };

  useEffect(() => {
    if (editorMode !== 'canvas') return;
    const onKeyDown = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      const editing =
        target?.matches('input, textarea, select') || target?.isContentEditable === true;
      if (event.key === 'Escape') {
        setLinkSource('');
        setLinkBranch(undefined);
        setLinkDrag(null);
        setLinkFeedback('已取消当前连线操作。');
        return;
      }
      if (editing) return;
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'z') {
        event.preventDefault();
        if (event.shiftKey) redoFlow();
        else undoFlow();
        return;
      }
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'y') {
        event.preventDefault();
        redoFlow();
        return;
      }
      if (event.key !== 'Delete' && event.key !== 'Backspace') return;
      if (selectedEdgeId) {
        event.preventDefault();
        commitFlow({
          ...flow,
          edges: flow.edges.filter((edge) => edge.id !== selectedEdgeId),
        });
        setSelectedEdgeId('');
      } else if (selectedNodeId) {
        event.preventDefault();
        removeAndReconnectNode(selectedNodeId);
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
    // The editor callbacks intentionally bind to the latest graph snapshot.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editorMode, flow, selectedEdgeId, selectedNodeId]);

  const applySerialLoginTemplate = () => {
    const next = createSerialLoginTemplate(draft.boardId);
    commitFlow(next);
    setSelectedNodeId(next.nodes.find((node) => node.type === 'serial.login')?.id ?? '');
    setSelectedEdgeId('');
    setLinkSource('');
    setDraft((current) => ({
      ...taskWithDeviceFlow(next, current, current.boardId),
      name:
        current.name.trim() && !current.name.startsWith('新自动任务')
          ? current.name
          : `${boardDisplayName(current.boardId).split(' · ')[0]} 串口自动登录`,
    }));
  };

  const moveNode = (nodeId: string, pointerId: number, clientX: number, clientY: number) => {
    if (!dragging || dragging.id !== nodeId || dragging.pointerId !== pointerId) return;
    const canvas = canvasRef.current;
    const rect = canvas?.getBoundingClientRect();
    if (!canvas || !rect) return;
    flowHistory.replace((current) => ({
      ...current,
      nodes: current.nodes.map((node) =>
        node.id === nodeId
          ? {
              ...node,
              x: Math.max(10, (clientX - rect.left + canvas.scrollLeft) / zoom - dragging.dx),
              y: Math.max(10, (clientY - rect.top + canvas.scrollTop) / zoom - dragging.dy),
            }
          : node
      ),
    }));
  };

  const finishNodeDrag = (nodeId: string) => {
    if (dragging?.id === nodeId) {
      const before = dragging.start.nodes.find((item) => item.id === nodeId);
      const after = flow.nodes.find((item) => item.id === nodeId);
      if (before && after && (before.x !== after.x || before.y !== after.y)) {
        flowHistory.recordSnapshot(dragging.start);
        setDraft(taskWithDeviceFlow(flow, draft));
      }
    }
    setDragging(null);
  };

  const updateNode = (patch: Partial<DeviceFlowNode>) => {
    if (!selectedNode) return;
    commitFlow({
      ...flow,
      nodes: flow.nodes.map((node) => (node.id === selectedNode.id ? { ...node, ...patch } : node)),
    });
  };
  const updateConfig = (key: string, value: unknown) =>
    updateNode({ config: { ...selectedNode?.config, [key]: value } });

  const bindFixedBoard = (boardId: string) => {
    const next = {
      ...flow,
      nodes: flow.nodes.map((node) => {
        const domain = nodeKind(node.type)?.deviceDomain;
        if (!domain) return node;
        const config = { ...node.config };
        if (scopeMatchesDomain(boardId, domain)) config.boardId = boardId;
        else delete config.boardId;
        return { ...node, config };
      }),
    };
    flowHistory.replace(next);
    setSaveFeedback('有未保存更改');
    setDraft((current) => {
      return {
        ...taskWithDeviceFlow(next, current, boardId),
        policyId: compatiblePolicyIdForBoard(policies, current.policyId, boardId),
      };
    });
  };

  const connectNodes = (
    sourceId: string,
    target: string,
    requestedBranch?: DeviceFlowEdge['branch']
  ) => {
    if (!sourceId || sourceId === target) {
      setLinkSource('');
      setLinkBranch(undefined);
      return false;
    }
    const source = flow.nodes.find((node) => node.id === sourceId);
    const targetNode = flow.nodes.find((node) => node.id === target);
    if (!source || !targetNode) return false;
    if (source.type.startsWith('result.')) {
      setLinkFeedback('结果节点是流程终点，不能再连接后续步骤。');
      setLinkSource('');
      setLinkBranch(undefined);
      return false;
    }
    if (targetNode.type.startsWith('trigger.')) {
      setLinkFeedback(
        `“${targetNode.label}”只能作为流程起点；请从它的下方端口拖到“${source.label}”。`
      );
      setLinkSource('');
      setLinkBranch(undefined);
      return false;
    }
    const reachesSource = (nodeId: string, visited = new Set<string>()): boolean => {
      if (nodeId === sourceId) return true;
      if (visited.has(nodeId)) return false;
      visited.add(nodeId);
      return flow.edges
        .filter((edge) => edge.source === nodeId)
        .some((edge) => reachesSource(edge.target, visited));
    };
    if (reachesSource(target)) {
      setLinkFeedback('这条连线会形成无界循环；需要重试时请使用“有界重试”节点。');
      setLinkSource('');
      setLinkBranch(undefined);
      return false;
    }
    const branch =
      requestedBranch ??
      (source.type === 'verify.deterministic' || source.type === 'control.condition'
        ? targetNode.type === 'result.failed'
          ? 'failure'
          : 'success'
        : undefined);
    const outgoing = flow.edges.filter((edge) => edge.source === sourceId);
    const conflicting = branch
      ? outgoing.some((edge) => (edge.branch ?? 'success') === branch)
      : outgoing.length > 0;
    if (conflicting) {
      setLinkFeedback(
        branch
          ? `“${source.label}”已经有${branch === 'failure' ? '失败' : '成功'}分支，请先删除原连线。`
          : `“${source.label}”已经连接了后续节点，请先删除原连线。`
      );
      setLinkSource('');
      setLinkBranch(undefined);
      return false;
    }
    if (!flow.edges.some((edge) => edge.source === sourceId && edge.target === target)) {
      commitFlow({
        ...flow,
        edges: [
          ...flow.edges,
          { id: `edge-${sourceId}-${target}`, source: sourceId, target, branch },
        ],
      });
      setLinkFeedback(`已连接：${source.label} → ${targetNode?.label ?? target}`);
    } else {
      setLinkFeedback('这两个节点已经连接。');
    }
    setLinkSource('');
    setLinkBranch(undefined);
    return true;
  };

  const connect = (target: string) => {
    if (!linkSource) return;
    connectNodes(linkSource, target, linkBranch);
  };

  const fitCanvas = () => {
    const canvas = canvasRef.current;
    if (!canvas || !flow.nodes.length || canvas.clientWidth <= 80 || canvas.clientHeight <= 80)
      return;
    const minX = Math.min(...flow.nodes.map((node) => node.x));
    const maxX = Math.max(...flow.nodes.map((node) => node.x + 190));
    const minY = Math.min(...flow.nodes.map((node) => node.y));
    const maxY = Math.max(...flow.nodes.map((node) => node.y + 76));
    const nextZoom = Math.min(
      1.25,
      (canvas.clientWidth - 80) / (maxX - minX),
      (canvas.clientHeight - 80) / (maxY - minY)
    );
    setZoom(nextZoom);
    window.requestAnimationFrame(() => {
      canvas.scrollLeft = Math.max(0, ((minX + maxX) / 2) * nextZoom - canvas.clientWidth / 2);
      canvas.scrollTop = Math.max(0, ((minY + maxY) / 2) * nextZoom - canvas.clientHeight / 2);
    });
    setLinkFeedback('已将全部节点适配到当前视图。');
  };

  const saveFlow = async () => {
    const isDraft = issues.length > 0;
    const task = {
      ...taskWithDeviceFlow(flow, draft),
      enabled: isDraft ? false : draft.enabled,
    };
    setDraft(task);
    setSaveFeedback(isDraft ? '正在保存草稿…' : '正在保存…');
    try {
      const saved = await onSave(task);
      lastSavedTaskIdRef.current = saved.id;
      flowHistory.replace(readDeviceFlow(saved, nodeKind));
      setSaveFeedback(
        `${isDraft ? '草稿' : '流程'}已保存 · ${new Date().toLocaleTimeString([], {
          hour: '2-digit',
          minute: '2-digit',
        })}`
      );
    } catch (error) {
      setSaveFeedback(`保存失败：${formatErrorMessage(error)}`);
    }
  };

  return (
    <div className="ais-flow-page">
      <DeviceFlowTaskList
        tasks={tasks}
        selectedTaskId={draft.id}
        boardDisplayName={boardDisplayName}
        onNewTask={onNew}
        onSelectTask={setDraft}
      >
        <DeviceFlowPalette
          mode={editorMode}
          search={search}
          groups={groups}
          onSearchChange={setSearch}
          onAddNode={addPaletteNode}
        />
      </DeviceFlowTaskList>

      <main className="ais-flow-workspace">
        <DeviceFlowToolbar
          taskName={draft.name}
          issuesCount={issues.length}
          saveFeedback={saveFeedback}
          mode={editorMode}
          canUndo={flowHistory.canUndo}
          canRedo={flowHistory.canRedo}
          zoom={zoom}
          hasNodes={flow.nodes.length > 0}
          canUseSerialTemplate={draft.boardId.startsWith('linux:')}
          hasFixedBoard={Boolean(draft.boardId)}
          taskId={draft.id}
          busy={busy}
          draftSaveIssue={draftSaveIssues[0]}
          onTaskNameChange={(name) => {
            setDraft((current) => ({ ...current, name }));
            setSaveFeedback('有未保存更改');
          }}
          onModeChange={(mode) => {
            setEditorMode(mode);
            if (mode === 'sequence') {
              setLinkSource('');
              setLinkBranch(undefined);
              setSelectedEdgeId('');
            }
          }}
          onUndo={undoFlow}
          onRedo={redoFlow}
          onZoomOut={() => setZoom((value) => Math.min(value, Math.max(0.05, value - 0.1)))}
          onZoomIn={() => setZoom((value) => Math.min(1.5, value + 0.1))}
          onAutoLayout={() => commitFlow(autoLayout(flow))}
          onFitCanvas={fitCanvas}
          onApplySerialTemplate={applySerialLoginTemplate}
          onTrigger={(taskId) => void onTrigger(taskId)}
          onSave={() => void saveFlow()}
        />
        <section className="ais-flow-canvas-shell ais-card">
          {editorMode === 'sequence' ? (
            <DeviceFlowSequence
              flow={flow}
              sequence={sequence}
              boardId={draft.boardId}
              boardOptions={boardOptions}
              selectedNodeId={selectedNodeId}
              insertAfterNodeId={insertAfterNodeId}
              insertKinds={DEVICE_FLOW_SEQUENCE_INSERT_KINDS}
              nodeView={nodeKind}
              boardDisplayName={boardDisplayName}
              onBindBoard={bindFixedBoard}
              onApplySerialLoginTemplate={applySerialLoginTemplate}
              onCreateBlankTemplate={() => {
                const next = createBlankSequenceTemplate();
                commitFlow(next);
                setSelectedNodeId(next.nodes[0].id);
              }}
              onInsertMenuChange={setInsertAfterNodeId}
              onInsertNode={(kind, afterNodeId) => {
                const fullKind = nodeKind(kind.type);
                if (fullKind) {
                  insertConnectedNode(fullKind, afterNodeId ?? defaultInsertionSource?.id);
                }
              }}
              onSelectNode={setSelectedNodeId}
              onRemoveNode={removeAndReconnectNode}
            />
          ) : (
            <>
              <DeviceFlowCanvas
                flow={flow}
                zoom={zoom}
                canvasRef={canvasRef}
                selectedNodeId={selectedNodeId}
                selectedEdgeId={selectedEdgeId}
                linkSource={linkSource}
                linkBranch={linkBranch}
                linkDrag={linkDrag}
                nodeView={nodeKind}
                boardDisplayName={boardDisplayName}
                onClearSelection={() => {
                  setSelectedNodeId('');
                  setSelectedEdgeId('');
                  setLinkSource('');
                  setLinkBranch(undefined);
                  setLinkFeedback('');
                }}
                onDropNode={(type, x, y) => {
                  const kind = nodeKind(type);
                  if (kind) addNode(kind, x, y);
                }}
                onSelectEdge={(edgeId) => {
                  setSelectedEdgeId(edgeId);
                  setSelectedNodeId('');
                  setLinkSource('');
                  setLinkBranch(undefined);
                  setLinkFeedback('已选中连线；按 Delete 删除，或双击连线删除。');
                }}
                onDeleteEdge={(edgeId) => {
                  commitFlow({
                    ...flow,
                    edges: flow.edges.filter((edge) => edge.id !== edgeId),
                  });
                  setSelectedEdgeId('');
                }}
                onSelectNode={(nodeId) => {
                  setSelectedNodeId(nodeId);
                  setSelectedEdgeId('');
                }}
                onConnectTarget={connect}
                onNodeDragStart={(id, pointerId, dx, dy) => {
                  setSelectedNodeId(id);
                  setSelectedEdgeId('');
                  setDragging({ id, pointerId, dx, dy, start: flow });
                }}
                onNodeDragMove={(id, pointerId, clientX, clientY) =>
                  moveNode(id, pointerId, clientX, clientY)
                }
                onNodeDragEnd={(id) => finishNodeDrag(id)}
                onNodeDragCancel={() => setDragging(null)}
                onLinkStart={(drag) => {
                  setLinkSource(drag.source);
                  setLinkBranch(drag.branch);
                  const node = flow.nodes.find((item) => item.id === drag.source);
                  setLinkFeedback(
                    `正在连接“${node?.label ?? drag.source}”${drag.branch === 'success' ? '成功分支' : drag.branch === 'failure' ? '失败分支' : ''}，拖到目标节点后松开。`
                  );
                  setLinkDrag(drag);
                }}
                onLinkMove={setLinkDrag}
                onLinkComplete={(source, target, branch) => {
                  connectNodes(source, target, branch);
                }}
                onLinkPortSelected={(source, branch, toggle = false) => {
                  const node = flow.nodes.find((item) => item.id === source);
                  if (toggle && linkSource === source && linkBranch === branch) {
                    setLinkSource('');
                    setLinkBranch(undefined);
                  } else {
                    setLinkSource(source);
                    setLinkBranch(branch);
                    setLinkFeedback(
                      `已选中“${node?.label ?? source}”的输出端口；现在点击目标节点的顶部端口。`
                    );
                  }
                }}
                onLinkCancel={() => setLinkDrag(null)}
              />
              <footer>
                <span>
                  <LynxIcon name="flow" size={16} /> {flow.nodes.length} 节点 · {flow.edges.length}{' '}
                  连线
                </span>
                <span>
                  {selectedEdgeId ? (
                    <button
                      className="ais-flow-delete-edge"
                      onClick={() => {
                        commitFlow({
                          ...flow,
                          edges: flow.edges.filter((edge) => edge.id !== selectedEdgeId),
                        });
                        setSelectedEdgeId('');
                      }}
                    >
                      <LynxIcon name="delete" size={16} /> 删除选中连线
                    </button>
                  ) : linkFeedback ? (
                    <span className="ais-flow-link-feedback">{linkFeedback}</span>
                  ) : linkSource ? (
                    '拖到目标节点后松开，或点击目标节点顶部端口完成连接'
                  ) : (
                    '从节点下方端口拖到目标节点即可连接'
                  )}
                </span>
              </footer>
            </>
          )}
        </section>
      </main>

      <DeviceFlowInspector
        selectedNode={selectedNode}
        selectedNodeDescription={selectedNode && nodeKind(selectedNode.type)?.description}
        selectedNodeDomain={selectedNode && nodeKind(selectedNode.type)?.deviceDomain}
        selectedNodeHighRisk={Boolean(selectedNode && nodeKind(selectedNode.type)?.highRisk)}
        linuxDevices={devices
          .filter((device): device is WorkbenchDevice & { id: number } => device.id !== null)
          .map((device) => ({ value: device.id, label: device.name }))}
        mcuBoards={mcuBoards.map((board) => ({ value: board.id, label: board.name }))}
        boardOptions={boardOptions}
        boardId={draft.boardId}
        fixedBoardAvailable={fixedBoardAvailable}
        policyId={draft.policyId}
        policies={policies
          .filter((policy): policy is SupervisorPolicy & { id: number } => policy.id !== null)
          .map((policy) => ({ value: policy.id, label: policy.name }))}
        triggerType={draft.triggerType}
        taskId={draft.id}
        webhookSecret={webhookSecret}
        enabled={draft.enabled}
        issues={issues}
        onUpdateNode={updateNode}
        onUpdateConfig={updateConfig}
        onBindBoard={bindFixedBoard}
        onPolicyChange={(policyId) => {
          setDraft((current) => ({ ...current, policyId }));
          setSaveFeedback('有未保存更改');
        }}
        onWebhookSecretChange={(value) => {
          setWebhookSecret(value);
          setSaveFeedback('有未保存更改');
        }}
        onEnabledChange={(enabled) => {
          setDraft((current) => ({ ...current, enabled }));
          setSaveFeedback('有未保存更改');
        }}
        onRemoveNode={removeAndReconnectNode}
        onDeleteTask={(taskId) => void onDelete(taskId)}
      />
    </div>
  );
};
