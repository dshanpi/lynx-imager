import { useEffect, useRef, useState } from 'react';
import { invokeCommand } from '../../Platform/IPC';
import type {
  SupervisorHumanRequest,
  SupervisorRound,
  SupervisorRun,
  WorkbenchDevice,
} from '../../Platform/IPC/Commands';
import { ACTION_LABELS } from '../AISupervisor/supervisorConfiguration';
import {
  AIFacts,
  AIMessage,
  AITable,
  safeDetail,
  timeLabel,
  useAIAction,
} from './imagerAIControls';

const executable = new Set([
  'collect_logs',
  'rescan_usb',
  'wait_device',
  'power_cycle',
  'reset_board',
]);
function scopeToken(
  run: SupervisorRun,
  round: SupervisorRound | undefined,
  request: SupervisorHumanRequest | undefined
) {
  return JSON.stringify([
    run.boardId,
    run.id,
    run.taskId,
    run.policyId,
    run.status,
    round?.id,
    round?.roundIndex,
    round?.decision.decision,
    round?.decision.parameters,
    round?.redactedContext.firmware_hash,
    round?.policyResult,
    round?.actionResult,
    request?.id,
    request?.status,
  ]);
}

export function ImagerAIApproval({
  device,
  reloadDevice,
  onPending,
  onOpenFlash,
}: {
  device: WorkbenchDevice | null;
  reloadDevice: () => void;
  onPending: (pending: boolean) => void;
  onOpenFlash?: (deviceId: number) => void;
}) {
  const board = device?.id ? 'linux:' + device.id : '';
  const [items, setItems] = useState<SupervisorRun[]>([]);
  const [human, setHuman] = useState<SupervisorHumanRequest[]>([]);
  const [rounds, setRounds] = useState<SupervisorRound[]>([]);
  const [selectedId, setSelectedId] = useState('');
  const [confirmation, setConfirmation] = useState('');
  const [issue, setIssue] = useState('');
  const [refresh, setRefresh] = useState(0);
  const [loading, setLoading] = useState(true);
  const current = useRef(true);
  useEffect(() => {
    current.current = true;
    return () => {
      current.current = false;
    };
  }, []);
  const { run: act, busy, notice } = useAIAction();
  useEffect(() => {
    let active = true;
    let timer: number | undefined;
    const read = async () => {
      try {
        const [runs, requests] = await Promise.all([
          invokeCommand('supervisor_list_runs', { limit: 200 }),
          invokeCommand('supervisor_list_human_requests', { waitingOnly: true }),
        ]);
        if (!active) return;
        const visible = runs.filter((r) => r.boardId === board);
        const target =
          visible.find((r) => r.id === selectedId) ||
          visible.find((r) => ['waiting_approval', 'waiting_human'].includes(r.status)) ||
          visible[0];
        const records = target
          ? await invokeCommand('supervisor_list_rounds', { runId: target.id, limit: 60 })
          : [];
        if (!active) return;
        setItems(visible);
        setHuman(requests);
        setRounds(records.filter((r) => r.runId === target?.id));
        if (target && target.id !== selectedId) setSelectedId(target.id);
        setIssue('');
        onPending(visible.some((r) => ['waiting_approval', 'waiting_human'].includes(r.status)));
      } catch (error) {
        if (active) {
          setIssue(safeDetail(error));
          setItems([]);
          setRounds([]);
          setHuman([]);
          setConfirmation('');
          onPending(false);
        }
      }
      if (active) {
        setLoading(false);
        timer = window.setTimeout(read, 5000);
      }
    };
    void read();
    return () => {
      active = false;
      window.clearTimeout(timer);
    };
  }, [selectedId, board, refresh, onPending]);
  const selected = items.find((r) => r.id === selectedId && r.boardId === board);
  const round = rounds
    .filter((r) => r.runId === selected?.id)
    .sort((a, b) => b.roundIndex - a.roundIndex)[0];
  const request = human.find((r) => r.runId === selected?.id && r.status === 'waiting');
  const key = selected ? scopeToken(selected, round, request) : '';
  const pending =
    selected?.status === 'waiting_approval' &&
    round?.policyResult === 'approval_required' &&
    round?.actionResult === 'not_executed';
  const humanPending = selected?.status === 'waiting_human' && !!request;
  const action = round?.decision.decision || '';
  const canApprove = pending && executable.has(action);
  const checked = !!key && key === confirmation;
  const manuallyFlash = pending && action === 'retry_flash';
  async function respond(approve: boolean, manual = false) {
    if (!selected || !(pending || humanPending) || (approve && !checked)) return;
    // Re-read the pending round immediately before applying this exact operator decision.
    const [runs, latestRounds, requests, devices, policies] = await Promise.all([
      invokeCommand('supervisor_list_runs', { limit: 200 }),
      invokeCommand('supervisor_list_rounds', { runId: selected.id, limit: 1 }),
      invokeCommand('supervisor_list_human_requests', { waitingOnly: true }),
      approve ? invokeCommand('workbench_list_devices') : [],
      approve ? invokeCommand('supervisor_list_policies') : [],
    ]);
    const latest = runs.find((r) => r.id === selected.id && r.boardId === board);
    const latestRequest = requests.find((r) => r.runId === selected.id && r.status === 'waiting');
    if (!current.current) return;
    setConfirmation('');
    if (!latest || scopeToken(latest, latestRounds[0], latestRequest) !== key) {
      setRefresh((n) => n + 1);
      throw new Error('请求已更新，请重新核对设备与动作。');
    }
    if (approve) {
      const currentDevice = devices.find((candidate) => candidate.id === device?.id);
      const fields = Object.keys(currentDevice || {}) as (keyof WorkbenchDevice)[];
      if (
        !currentDevice ||
        !device ||
        fields.some((field) => currentDevice[field] !== device[field])
      ) {
        reloadDevice();
        throw new Error('目标设备配置已变化，请重新核对接口与动作。');
      }
      const policy = policies.find((candidate) => candidate.id === latest.policyId);
      if (!policy?.enabled || (policy.boardScope.length && !policy.boardScope.includes(board))) {
        throw new Error('当前监督策略已停用或不再绑定此设备。');
      }
      if (canApprove && ![...policy.allowedActions, ...policy.approvalActions].includes(action)) {
        throw new Error('该动作已被当前监督策略禁止。');
      }
    }
    if (humanPending && latestRequest) {
      await invokeCommand('supervisor_resolve_human_request', {
        requestId: latestRequest.id,
        resolution: approve ? (manual ? 'manual' : 'continue') : 'terminate',
      });
    } else if (!approve) {
      await invokeCommand('supervisor_control_run', { runId: selected.id, action: 'terminate' });
    } else if (canApprove && round) {
      await invokeCommand('supervisor_execute_action', {
        runId: selected.id,
        roundIndex: round.roundIndex,
        approved: true,
      });
    } else {
      throw new Error('此动作需要操作员在相应的工作区重新发起。');
    }
    setRefresh((n) => n + 1);
    return approve ? '已处理本次请求。' : '已拒绝请求并停止本次监督。';
  }

  return (
    <>
      <p className="im-ai-banner">
        {issue
          ? '无法读取请求：' + issue
          : loading
            ? '正在读取监督请求…'
            : pending || humanPending
              ? '当前请求等待操作员确认，请核对设备、动作与依据。'
              : '当前设备没有需要确认的请求。'}
      </p>
      {items.length > 1 && (
        <label>
          监督任务
          <select
            value={selectedId}
            disabled={busy}
            onChange={(e) => {
              setRounds([]);
              setConfirmation('');
              setSelectedId(e.target.value);
            }}
          >
            {items.map((r) => (
              <option key={r.id} value={r.id}>
                {r.taskId} · {r.status}
              </option>
            ))}
          </select>
        </label>
      )}
      <div className="im-ai-columns">
        <section>
          <h2>调用记录</h2>
          <AITable
            columns={['时间', '动作', '状态']}
            empty={!rounds.length ? '尚无当前任务的监督回合。' : ''}
          >
            {rounds
              .filter((r) => r.runId === selectedId)
              .map((r) => (
                <tr key={r.id}>
                  <td>{timeLabel(r.createdAt * 1000)}</td>
                  <td>{ACTION_LABELS[r.decision.decision] || r.decision.decision}</td>
                  <td>
                    {r.actionResult === 'not_executed'
                      ? r.policyResult === 'approval_required'
                        ? '需要批准'
                        : '尚未执行'
                      : '已有执行记录'}
                  </td>
                </tr>
              ))}
          </AITable>
        </section>
        <section>
          <h2>请求参数</h2>
          <AIFacts
            columns={['核对项', '本次请求']}
            rows={[
              ['目标设备', device?.name || '未选择设备'],
              ['任务', selected?.taskId || '—'],
              ['动作', ACTION_LABELS[action] || action || '—'],
              [
                'SHA-256 指纹',
                <code key="firmware-hash">
                  {typeof round?.redactedContext.firmware_hash === 'string'
                    ? round.redactedContext.firmware_hash
                    : '未提供'}
                </code>,
              ],
              ['绑定接口', device?.usbBinding || '未提供'],
              ['处理依据', safeDetail(request?.instruction || round?.decision.reason || '—')],
            ]}
          />
        </section>
      </div>
      <section>
        <h2>操作员确认</h2>
        <label className="im-ai-check">
          <input
            type="checkbox"
            checked={checked}
            disabled={busy || !(canApprove || humanPending)}
            onChange={(e) => setConfirmation(e.target.checked ? key : '')}
          />
          我已核对目标设备、动作及本次请求的证据。
        </label>
        <p className="im-ai-muted">
          {manuallyFlash
            ? '固件重试须在烧录工作区重新选择产物，并核对目标设备后确认写入。'
            : pending && !canApprove
              ? '此动作需要在对应工作区由操作员处理。'
              : '批准仅用于当前任务的本次请求。'}
        </p>
        <div className="im-ai-actions">
          <button
            disabled={busy || !(pending || humanPending)}
            onClick={() => void act(() => respond(false))}
          >
            拒绝请求并终止监督
          </button>
          {manuallyFlash && onOpenFlash && device?.id ? (
            <button className="im-ai-primary" onClick={() => onOpenFlash(device.id!)}>
              转到烧录工作区
            </button>
          ) : (
            <button
              className="im-ai-primary"
              disabled={busy || !checked || !(canApprove || humanPending)}
              onClick={() => void act(() => respond(true))}
            >
              {humanPending ? '已处理，继续监督' : '允许本次动作'}
            </button>
          )}
          {humanPending && (
            <button disabled={busy || !checked} onClick={() => void act(() => respond(true, true))}>
              标记已人工完成
            </button>
          )}
          <button disabled={busy} onClick={() => setRefresh((n) => n + 1)}>
            刷新请求
          </button>
        </div>
      </section>
      <AIMessage>{notice}</AIMessage>
    </>
  );
}
