import { useEffect, useRef, useState } from 'react';
import { invokeCommand } from '../../../Platform/IPC/Client';
import type {
  LocalEnvironmentSnapshot,
  LocalLearningStepView,
  WorkbenchProject,
} from '../../../Platform/IPC/Commands';
import { formatErrorMessage } from '../../../Utils';
import { LearnBadge } from './LearnChrome';
import { LynxIcon } from '../../../CoreUI';

export function LearnEngineering({
  step,
  project,
  active = true,
  attempt,
}: {
  step: LocalLearningStepView;
  project?: WorkbenchProject;
  active?: boolean;
  attempt?: number;
}) {
  const [snapshot, setSnapshot] = useState<LocalEnvironmentSnapshot | null>(null);
  const [busy, setBusy] = useState('');
  const [message, setMessage] = useState('');
  const [sshAlias, setSshAlias] = useState('');
  const [relativeFile, setRelativeFile] = useState('');
  const sequence = useRef(0);
  // Draft saves refresh updatedAt without changing the observed inputs.
  const context = JSON.stringify([
    step.session.sessionId,
    step.session.courseId,
    step.session.courseVersion,
    step.session.coursePackageSha256,
    step.session.projectId,
    step.session.deviceId,
    step.session.boardUid,
    step.session.currentStep,
    step.session.state,
    attempt,
    step.lab.environmentChecks,
    project,
    active,
  ]);
  useEffect(() => {
    ++sequence.current;
    setSnapshot(null);
    setMessage('');
    setBusy('');
    return () => {
      ++sequence.current;
    };
  }, [context]);
  useEffect(() => {
    setSshAlias('');
    setRelativeFile('');
  }, [step.session.sessionId, project?.id, project?.sdkDir, project?.hostId]);
  const perform = async (action: 'inspect' | 'ide') => {
    if (busy || !active || !project) return;
    const request = ++sequence.current;
    setBusy(action);
    setMessage('');
    setSnapshot(null);
    try {
      if (action === 'inspect') {
        const result = await invokeCommand('learning_inspect_project', {
          sessionId: step.session.sessionId,
        });
        if (request === sequence.current) setSnapshot(result);
      } else {
        await invokeCommand('learning_open_ide', {
          sessionId: step.session.sessionId,
          sshAlias,
          relativeFile,
        });
        if (request === sequence.current)
          setMessage(
            '已请求 VS Code 打开绑定的远程工程或文件。请在 VS Code 确认连接与文件保存；返回后点击检查源码。打开请求不代表连接、构建或验证通过。'
          );
      }
    } catch (reason) {
      if (request === sequence.current) setMessage(formatErrorMessage(reason));
    } finally {
      if (request === sequence.current) setBusy('');
    }
  };
  const image = step.lab.hilVerification?.image;
  const requirements = step.lab.environmentChecks || [];
  const sourceLabel = snapshot?.source
    ? {
        git: '已读取 Git 提交与工作区',
        unborn: 'Git 仓库尚无提交',
        directory: '普通目录（无 Git 元数据）',
        unavailable: '源码状态无法核验',
      }[snapshot.source.kind]
    : '已取得 Git 快照';
  const statusLabel = {
    MATCH: '符合声明',
    MISMATCH: '与要求不符',
    MISSING: '缺失',
    UNVERIFIED: '无法核验',
  };
  return (
    <section className="learn-engineering" data-testid="learn-engineering">
      <h1>先核对工程，再进入开发</h1>
      <p>确认工程目录与版本是否符合课程要求，避免版本差异影响后续步骤。</p>
      <section className="learn-panel learn-version-panel">
        <div className="learn-project-path">
          <strong>
            远端工程目录
            <br />
            <small>（Ubuntu 主机）</small>
          </strong>
          <code>{project?.sdkDir || '工程尚未配置'}</code>
          <LynxIcon name="folder" size={24} />
        </div>
        <div className="learn-table-wrap">
          <table className="learn-table">
            <thead>
              <tr>
                <th>项目</th>
                <th>课程要求</th>
                <th>当前检测</th>
                <th>状态</th>
              </tr>
            </thead>
            <tbody>
              <tr>
                <td>源码版本</td>
                <td>以本课教材为准</td>
                <td>
                  {snapshot ? (
                    <details>
                      <summary>{sourceLabel}</summary>
                      <pre>{snapshot.output}</pre>
                      {snapshot.source?.kind === 'directory' && (
                        <p>可以使用解压工程；目录没有 Git 提交，不将其当作已验证的源码版本。</p>
                      )}
                      {snapshot.source?.dirty !== null && snapshot.source?.dirty !== undefined && (
                        <p>
                          {snapshot.source.dirty ? '工作区有改动' : '本次未发现 Git 改动'}；
                          {snapshot.source.untracked ? '包含未跟踪文件' : '未发现未跟踪文件'}。
                        </p>
                      )}
                    </details>
                  ) : (
                    '尚未读取远程工程'
                  )}
                </td>
                <td>
                  <LearnBadge tone={snapshot ? 'blue' : 'orange'}>
                    {snapshot ? '待对照' : '待核验'}
                  </LearnBadge>
                </td>
              </tr>
              <tr>
                <td>Ubuntu 主机</td>
                <td>当前绑定的开发主机</td>
                <td>
                  {snapshot?.host
                    ? `${snapshot.host.os} · ${snapshot.host.architecture}`
                    : '尚未读取'}
                </td>
                <td>
                  <LearnBadge tone={snapshot?.host ? 'blue' : 'gray'}>
                    {snapshot?.host ? '已读取' : '待核验'}
                  </LearnBadge>
                </td>
              </tr>
              <tr>
                <td>板端系统</td>
                <td>{step.lab.hilVerification?.target || '按本课目标环境'}</td>
                <td>尚未读取板端系统</td>
                <td>
                  <LearnBadge tone="gray">待核验</LearnBadge>
                </td>
              </tr>
              <tr>
                <td>镜像摘要</td>
                <td>
                  {image ? (
                    <details>
                      <summary>{image.path}</summary>
                      <code>{image.sha256}</code>
                    </details>
                  ) : (
                    '本课未声明'
                  )}
                </td>
                <td>实际镜像尚未核验</td>
                <td>
                  <LearnBadge tone="gray">待核验</LearnBadge>
                </td>
              </tr>
              <tr>
                <td>构建产物</td>
                <td>{project?.firmwarePath || '未配置'}</td>
                <td>路径不代表构建完成</td>
                <td>
                  <LearnBadge tone="gray">待验证</LearnBadge>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
        <section data-testid="local-environment-checks">
          <h3>本活动的 SDK 与工具要求</h3>
          {!requirements.length ? (
            <p>
              本活动未声明可自动核验的环境要求。下方工具信息仅为当次观察，不代表 SDK 已准备完成。
            </p>
          ) : (
            <div className="learn-table-wrap">
              <table className="learn-table">
                <thead>
                  <tr>
                    <th>检查项与要求</th>
                    <th>实际观察</th>
                    <th>结果与下一步</th>
                  </tr>
                </thead>
                <tbody>
                  {requirements.map((spec) => {
                    const result = snapshot?.checks?.find((item) => item.spec.id === spec.id);
                    return (
                      <tr key={spec.id}>
                        <td>
                          <strong>{spec.title}</strong>
                          <p>
                            <code>{spec.selector}</code>
                          </p>
                          <small>
                            {spec.operator === 'present'
                              ? '系统工具已安装且能读取版本'
                              : `${spec.operator === 'sha256' ? 'SHA256' : '精确值'}：${spec.expected}`}
                          </small>
                        </td>
                        <td>
                          <code style={{ overflowWrap: 'anywhere', whiteSpace: 'pre-wrap' }}>
                            {result?.observed || '尚无有效观察'}
                          </code>
                        </td>
                        <td>
                          <LearnBadge tone={result?.status === 'MATCH' ? 'blue' : 'orange'}>
                            {result ? statusLabel[result.status] : '待核验'}
                          </LearnBadge>
                          {(!result || result.status !== 'MATCH') && <p>{spec.remediation}</p>}
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          )}
        </section>
        {snapshot?.tools && (
          <section data-testid="local-environment-tools">
            <h3>实际系统工具</h3>
            {snapshot.tools.map((tool) => (
              <details key={tool.name}>
                <summary>
                  {tool.name} ·{' '}
                  {tool.status === 'OBSERVED'
                    ? tool.version
                    : tool.status === 'MISSING'
                      ? '未安装'
                      : '无法读取'}
                </summary>
                <p>
                  <code>{tool.path || '无可核验路径'}</code>
                </p>
                <p>
                  程序 SHA256：
                  <code style={{ overflowWrap: 'anywhere' }}>{tool.sha256 || '未取得'}</code>
                </p>
                <small>
                  仅查询固定系统工具，不执行工程内的同名程序；工具存在不代表适用于目标板。
                </small>
              </details>
            ))}
          </section>
        )}
        <small>配置、读取结果与验证结论分别展示。不能从文件摘要推断板端系统。</small>
        <div className="learn-warning">
          <LynxIcon name="warning" size={24} />
          <div>
            先检查版本差异，再开始本课开发。<p>不会自动切换分支、覆盖已有修改或刷写镜像。</p>
          </div>
        </div>
        <div className="learn-ide-target" style={{ display: 'grid', gap: 12, marginBlock: 16 }}>
          <label style={{ display: 'grid', gap: 6 }}>
            SSH 别名（自定义端口必填）
            <input
              aria-label="SSH 别名"
              value={sshAlias}
              maxLength={128}
              disabled={!active || !project || Boolean(busy)}
              placeholder="例如 teaching-ubuntu"
              onChange={(event) => {
                setSshAlias(event.target.value);
                setMessage('');
                setSnapshot(null);
              }}
            />
          </label>
          <small>
            在本机 ~/.ssh/config 保存明确的 HostName、User、Port，并让 VS Code Remote SSH
            使用此配置。LYNX 只读核对该文件，不修改配置；Include、Match 和通配 Host
            暂不支持。别名仅用于本次打开。
          </small>
          <label style={{ display: 'grid', gap: 6 }}>
            工程内文件（相对上方工程目录，留空打开工程）
            <input
              aria-label="工程内文件"
              value={relativeFile}
              maxLength={2048}
              disabled={!active || !project || Boolean(busy)}
              placeholder="例如 lesson/src/main.c"
              onChange={(event) => {
                setRelativeFile(event.target.value);
                setMessage('');
              }}
            />
          </label>
        </div>
        <div className="learn-button-row">
          <button
            disabled={!active || !project || Boolean(busy)}
            onClick={() => void perform('inspect')}
          >
            <LynxIcon name="refresh" size={20} spin={busy === 'inspect'} />
            {busy === 'inspect' ? 'SSH 检查中…' : '连接 SSH 并检查源码'}
          </button>
          <button
            className="learning-secondary"
            disabled={!active || !project || Boolean(busy)}
            onClick={() => void perform('ide')}
          >
            <LynxIcon name="code" size={24} />在 VS Code 查看工程 ↗
          </button>
        </div>
        {snapshot && (
          <small>
            来源：SSH 只读工程与主机检查 · {new Date(snapshot.checkedAt * 1000).toLocaleString()}
            ；仅对本次保存的绑定与本活动声明负责，修改后请重新检查。不安装依赖、不推进课程，也不代表实机通过。
          </small>
        )}
        {message && (
          <p className="learn-notice" role="status">
            {message}
          </p>
        )}
        <div className="learn-project-tree">
          <h3>
            工程位置 <small>（已保存配置）</small>
          </h3>
          <p>
            <LynxIcon name="folder" size={20} /> {project?.name || '未绑定工程'}
          </p>
          <p className="learn-tree-child">
            <LynxIcon name="folder" size={20} />
            <code>{project?.sdkDir || '路径待配置'}</code>
          </p>
          <p className="learn-tree-child">
            <LynxIcon name="document" size={20} />
            <code>{project?.firmwarePath || '产物路径待配置'}</code>
          </p>
          <small>实际目录与文件请在 VS Code 中查看。修改工程配置请前往 Linux 工作台。</small>
        </div>
        <details>
          <summary>当前教材与规则版本</summary>
          <p>
            {step.session.courseId} · {step.session.courseVersion}
          </p>
          <code>{step.session.coursePackageSha256}</code>
        </details>
      </section>
    </section>
  );
}
