import { useEffect, useRef, useState } from 'react';
import { invokeCommand } from '../../../Platform/IPC/Client';
import type {
  LearnCoursePin,
  LearnEnvironmentView,
  LearnEnvironmentInspection,
} from '../../../Platform/IPC/Commands';
import { learnServiceError } from './serviceStatus';

export function LearnGenericEnvironment({
  active,
  pin,
  profileId,
}: {
  active: boolean;
  pin: LearnCoursePin;
  profileId: string;
}) {
  const [view, setView] = useState<LearnEnvironmentView | null>(null);
  const [snapshot, setSnapshot] = useState<LearnEnvironmentInspection | null>(null);
  const [projectId, setProjectId] = useState('');
  const [alias, setAlias] = useState('');
  const [relativeFile, setRelativeFile] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [reload, setReload] = useState(0);
  const sequence = useRef(0);
  const { enrollmentId, releaseId, manifestSha256 } = pin;
  const request = { enrollmentId, releaseId, manifestSha256, profileId };
  useEffect(() => {
    const seq = ++sequence.current;
    setSnapshot(null);
    setView(null);
    setError('');
    setNotice('');
    setProjectId('');
    setAlias('');
    setRelativeFile('');
    setBusy(false);
    if (!active) return;
    setBusy(true);
    void invokeCommand('learn_get_environment', {
      request: { enrollmentId, releaseId, manifestSha256, profileId },
    })
      .then((v) => {
        if (seq !== sequence.current) return;
        setView(v);
        setProjectId(v.binding ? String(v.binding.projectId) : '');
        setAlias(v.binding?.sshAlias || '');
      })
      .catch((e) => {
        if (seq === sequence.current) setError(learnServiceError(e));
      })
      .finally(() => {
        if (seq === sequence.current) setBusy(false);
      });
    return () => {
      sequence.current++;
    };
  }, [active, enrollmentId, releaseId, manifestSha256, profileId, reload]);
  async function perform(kind: 'bind' | 'inspect' | 'ide') {
    if (!active || busy) return;
    const seq = ++sequence.current;
    setBusy(true);
    setError('');
    setNotice('');
    try {
      if (kind === 'bind') {
        const binding = await invokeCommand('learn_bind_environment', {
          request: { ...request, projectId: Number(projectId), sshAlias: alias.trim() },
        });
        if (seq === sequence.current) {
          setView((v) => (v ? { ...v, binding } : v));
          setSnapshot(null);
          setNotice('已保存当前课程版本的工程绑定，请核验实际环境。');
        }
      } else if (kind === 'inspect') {
        setSnapshot(null);
        const value = await invokeCommand('learn_inspect_environment', { request });
        if (seq === sequence.current) {
          setSnapshot(value);
          setNotice('已读取 Ubuntu 工程；板卡、镜像及 SDK 仍需对应验证。');
        }
      } else {
        await invokeCommand('learn_open_ide', { request, relativeFile: relativeFile.trim() });
        if (seq === sequence.current) {
          setSnapshot(null);
          setNotice('已请求 VS Code 打开。编辑后请返回并重新核验工程。');
        }
      }
    } catch (e) {
      if (seq === sequence.current) setError(learnServiceError(e));
    } finally {
      if (seq === sequence.current) setBusy(false);
    }
  }
  const unsaved =
    !!view?.binding &&
    (projectId !== String(view.binding.projectId) || alias.trim() !== view.binding.sshAlias);
  const bound = !!view?.binding && !view.binding.changed && !unsaved;
  return (
    <section className="learn-generic-environment learn-panel" aria-label="开发环境与工程">
      <h3>开发环境与工程</h3>
      <p>选择 Linux 工作台已有工程。本页核验 Ubuntu；板卡状态需要实际设备证据。</p>
      {view?.target === 'board' && (
        <p role="status">当前活动要求板卡环境；Ubuntu 工程检查不能代替实机验证。</p>
      )}
      {error && (
        <p role="alert" className="learn-notice">
          {error}
        </p>
      )}
      {notice && <p role="status">{notice}</p>}
      {unsaved && <p role="status">工程选择或 SSH 别名尚未保存，请先保存绑定再核验或打开 IDE。</p>}
      <label>
        课程工程
        <select
          aria-label="课程工程"
          disabled={!active || busy}
          value={projectId}
          onChange={(e) => {
            setProjectId(e.target.value);
            setSnapshot(null);
            setNotice('');
            setRelativeFile('');
          }}
        >
          <option value="">选择已有工程</option>
          {view?.projects.map((p) => (
            <option
              key={p.projectId}
              value={p.projectId}
              disabled={view.target === 'board' && !p.hasBoard}
            >
              {p.name} · {p.hostName}
              {view.target === 'board' && !p.hasBoard ? ' · 未绑定板卡' : ''}
            </option>
          ))}
        </select>
      </label>
      {!busy && view && !view.projects.length && (
        <p>还没有可用工程。请在 Linux 工作台添加 Ubuntu 主机和工程，然后刷新。</p>
      )}
      <label>
        VS Code SSH 别名（可选）
        <input
          aria-label="VS Code SSH 别名"
          value={alias}
          disabled={!active || busy}
          onChange={(e) => {
            setAlias(e.target.value);
            setSnapshot(null);
            setNotice('');
          }}
          placeholder="如 ubuntu-dev；非 22 端口请填写"
          maxLength={128}
        />
      </label>
      <small>
        使用本机 ~/.ssh/config 中明确配置的 Host、HostName、User 和 Port；复杂匹配配置需先核对。
      </small>
      <div className="learn-hub-actions">
        <button
          className="learning-secondary"
          disabled={!active || busy || !projectId}
          onClick={() => void perform('bind')}
        >
          保存工程绑定
        </button>
        <button
          className="learning-secondary"
          disabled={!active || busy}
          onClick={() => setReload((n) => n + 1)}
        >
          刷新工程列表
        </button>
      </div>
      {view?.binding && (
        <>
          <p>
            <strong>{view.binding.projectName}</strong> · {view.binding.hostName}
          </p>
          <code>{view.binding.workspacePath}</code>
          <p>
            {view.binding.boardName
              ? `目标板卡：${view.binding.boardName} · 待实测`
              : '本工程未绑定板卡'}
          </p>
          {view.binding.changed && <p role="alert">工程配置发生变化，请重新确认绑定。</p>}
        </>
      )}
      <label>
        打开工程内文件（可选）
        <input
          aria-label="工程内文件"
          value={relativeFile}
          disabled={!active || busy}
          onChange={(e) => setRelativeFile(e.target.value)}
          placeholder="如 src/main.c"
          maxLength={2048}
        />
      </label>
      <div className="learn-hub-actions">
        <button
          className="learning-primary"
          disabled={!active || busy || !bound || view?.target !== 'ubuntu'}
          onClick={() => void perform('inspect')}
        >
          {busy ? '处理中…' : '核验 Ubuntu 与代码版本'}
        </button>
        <button
          className="learning-secondary"
          disabled={!active || busy || !bound}
          onClick={() => void perform('ide')}
        >
          在 VS Code 打开
        </button>
      </div>
      {snapshot && (
        <div className="learn-environment-result">
          <dl>
            <dt>检查时间</dt>
            <dd>{new Date(snapshot.checkedAt).toLocaleString()}</dd>
            <dt>系统</dt>
            <dd>
              {snapshot.os} · {snapshot.architecture}
            </dd>
            <dt>源码提交</dt>
            <dd>
              <code>{snapshot.commit}</code>
            </dd>
            <dt>工作区</dt>
            <dd>
              {snapshot.dirty ? '有本地修改' : '工作区干净'}
              {snapshot.untracked ? '；有未跟踪文件，内容尚未验证' : ''}
            </dd>
            <dt>编译器</dt>
            <dd>{snapshot.compiler || '尚未检测到 cc'}</dd>
          </dl>
          {snapshot.checks.map((c, i) => (
            <p key={i}>
              {c.component}：要求 {c.expected} ·{' '}
              {c.status === 'matched'
                ? '符合'
                : c.status === 'mismatch'
                  ? `不匹配（${c.observed}）`
                  : '尚未验证'}
            </p>
          ))}
          <small>读取成功不代表构建、部署或板卡验收通过。</small>
        </div>
      )}
    </section>
  );
}
