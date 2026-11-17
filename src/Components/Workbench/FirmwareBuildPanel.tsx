import { useEffect, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { LynxIcon } from '../../CoreUI';
import { invokeCommand } from '../../Platform/IPC';
import type {
  WorkbenchDevice,
  WorkbenchProject,
  WorkbenchHost,
  FirmwareBuildRecord,
} from '../../Platform/IPC/contracts/Workbench';
import FirmwareLifecyclePanel, {
  useFirmwareWorkspace,
  firmwareError,
  stateLabel,
} from './FirmwareLifecyclePanel';

const stageNames: Record<string, string> = {
  source_lock: '源码锁定',
  application: '应用构建',
  boot_config: '引导配置',
  boot: 'SyterKit 引导',
  package: '镜像打包',
  structure: '结构校验',
};
export default function FirmwareBuildPanel({
  board,
  project,
  host,
}: {
  board: WorkbenchDevice;
  project?: WorkbenchProject;
  host?: WorkbenchHost;
}) {
  const { workspace, refresh, error: readError } = useFirmwareWorkspace(board);
  const [selected, setSelected] = useState('');
  const [requestId, setRequestId] = useState(() => crypto.randomUUID());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [received, setReceived] = useState<FirmwareBuildRecord | null>(null);
  const build =
    received?.id === selected
      ? received
      : workspace?.builds.find((b) => b.id === selected) || workspace?.builds[0];
  useEffect(() => {
    if (!build || build.receipt?.status === 'success' || build.receipt?.status === 'failed') return;
    let cancelled = false;
    const id = build.id;
    const query = async () => {
      try {
        const b = await invokeCommand('workbench_firmware_build_status', {
          deviceId: board.id!,
          buildId: id,
        });
        if (!cancelled) {
          setReceived(b);
          setSelected(id);
          setError('');
        }
      } catch (e) {
        if (!cancelled) setError(`连接状态未知，保留原任务：${firmwareError(e)}`);
      }
    };
    const timer = setInterval(() => void query(), 5000);
    void query();
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [board.id, build?.id, build?.receipt?.status]);
  async function action(fn: () => Promise<void>) {
    setBusy(true);
    setError('');
    try {
      await fn();
      await refresh();
    } catch (e) {
      setError(firmwareError(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <div className="fw-lifecycle fw-view-build" data-testid="firmware-build-page">
      <header className="fw-page-title">
        <h3>F101 / NekoBox 构建</h3>
        <button
          disabled={
            busy ||
            !project?.id ||
            project.deviceId !== board.id ||
            workspace?.builds.some((b) => b.status === 'running')
          }
          onClick={() =>
            void action(async () => {
              const b = await invokeCommand('workbench_firmware_build_start', {
                deviceId: board.id!,
                projectId: project!.id!,
                requestId,
              });
              setSelected(b.id);
              setReceived(b);
              setRequestId(crypto.randomUUID());
            })
          }
        >
          开始新构建
        </button>
      </header>
      <div className="fw-build-environment">
        <LynxIcon name="desktop" size={20} />
        <strong>{host ? `${host.username}@${host.address}` : '尚未绑定 SDK 主机'}</strong>
        <span>工作区</span>
        <code title={project?.sdkDir}>{project?.sdkDir || '未配置'}</code>
      </div>
      <div className="fw-build-specs">
        <div>
          <LynxIcon name="circle" size={20} />
          <label>
            目标<strong>yuzukineko</strong>
          </label>
        </div>
        <div>
          <LynxIcon name="overview" size={20} />
          <label>
            应用<strong>zephyr-components/nekobox</strong>
          </label>
        </div>
        <div>
          <LynxIcon name="archive" size={20} />
          <label>
            工具链
            <strong title={build?.receipt?.toolchain?.version}>
              {build?.receipt?.toolchain?.version || '构建开始后读取实际版本'}
            </strong>
          </label>
        </div>
      </div>
      <details className="fw-build-history">
        <summary>构建历史与输入快照</summary>
        <select
          aria-label="构建历史"
          value={build?.id || ''}
          onChange={(e) => {
            setSelected(e.target.value);
            setReceived(null);
          }}
        >
          <option value="">选择构建记录</option>
          {workspace?.builds.map((b) => (
            <option key={b.id} value={b.id}>
              {b.id} · {stateLabel(b.status)}
            </option>
          ))}
        </select>
      </details>
      {(error || readError) && (
        <p role="alert" className="firmware-error">
          {error || readError}
        </p>
      )}
      <div className="fw-build-stages">
        {Object.entries(stageNames).map(([id, label]) => (
          <div key={id}>
            <LynxIcon
              name={
                build?.receipt?.stages.find((s) => s.name === id)?.status === 'success'
                  ? 'check-circle'
                  : 'clock'
              }
              size={20}
            />
            {label}
            <small>
              {stateLabel(build?.receipt?.stages.find((s) => s.name === id)?.status || 'pending')}
            </small>
          </div>
        ))}
      </div>
      <div className="fw-two-columns fw-build-workspace">
        <section className="fw-card">
          <h4>
            构建日志 <small>（{stateLabel(build?.status || 'pending')}）</small>
          </h4>
          <p>
            任务：{build?.id || '尚未开始'} · {stateLabel(build?.status || 'pending')}
          </p>
          <pre className="fw-build-log">
            {build?.logTail || build?.receipt?.error || '构建开始后显示实际日志。'}
          </pre>
          <details>
            <summary>源码、依赖与产物快照</summary>
            <pre>
              {build?.receipt ? JSON.stringify(build.receipt, null, 2) : '尚未生成输入快照'}
            </pre>
          </details>
          <button
            disabled={busy || !build}
            onClick={() =>
              void action(async () => {
                setReceived(
                  await invokeCommand('workbench_firmware_build_status', {
                    deviceId: board.id!,
                    buildId: build!.id,
                  })
                );
              })
            }
          >
            查询原任务 / 恢复记录
          </button>
        </section>
        <aside className="fw-card">
          <h4>
            本次产物 <span className="firmware-app-route">XIP / 0x80000</span>
          </h4>
          <table>
            <thead>
              <tr>
                <th>文件</th>
                <th>校验</th>
              </tr>
            </thead>
            <tbody>
              {build?.receipt?.artifacts.map((a) => (
                <tr key={a.fileName}>
                  <td>{a.fileName}</td>
                  <td>
                    <code title={a.sha256}>{a.sha256.slice(0, 16)}…</code>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <button
            className="fw-primary"
            disabled={busy || build?.receipt?.status !== 'success'}
            onClick={() =>
              void action(async () => {
                const loader = await open({
                  title: '选择本地 F101 RAM FES Loader',
                  multiple: false,
                });
                if (typeof loader !== 'string') return;
                const c = await invokeCommand('workbench_firmware_build_candidate', {
                  deviceId: board.id!,
                  buildId: build!.id,
                  loaderPath: loader,
                });
                window.dispatchEvent(
                  new CustomEvent('lynx:firmware-candidate', {
                    detail: { deviceId: board.id, candidateId: c.id },
                  })
                );
              })
            }
          >
            <LynxIcon name="play" size={16} /> 下载产物并送至当前单板测试
          </button>
          <p className="fw-warning">
            送测只进入烧录确认，不会直接写入设备。构建通过不能直接标记为正式版。
          </p>
          <p>每次构建使用独立输出目录；源码、依赖和工具链锁定，现有 SDK 终端保持可用。</p>
        </aside>
      </div>
    </div>
  );
}
export function FirmwareRuntimePanel({ boards }: { boards: WorkbenchDevice[] }) {
  const [id, setId] = useState<number | null>(null);
  const board = boards.find((b) => b.id === id) || boards[0];
  return (
    <div>
      <label className="fw-row">
        固件任务所属单板
        <select value={board?.id || ''} onChange={(e) => setId(Number(e.target.value))}>
          {boards.map((b) => (
            <option key={b.id} value={b.id!}>
              {b.name}
            </option>
          ))}
        </select>
      </label>
      {board ? (
        <FirmwareLifecyclePanel key={board.boardUid} board={board} initialView="publish" />
      ) : (
        <p>请先保存 Linux 单板，才能查看固件任务。</p>
      )}
    </div>
  );
}
