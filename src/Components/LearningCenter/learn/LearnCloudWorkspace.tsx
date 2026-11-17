import { useCallback, useEffect, useRef, useState } from 'react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { invokeCommand } from '../../../Platform/IPC/Client';
import type {
  CloudLearningEnrollment,
  CloudLearningUnitDetail,
  UpdateLearningUnitProgress,
} from '../../../Platform/IPC/Commands';
import type { LearningCourse } from '../types';
import { formatLearnError } from './errors';
import { LearnBreadcrumb, LearnBadge, LearnFooter } from './LearnChrome';
import { SafeMarkdown } from './LearnMarkdown';
import './LearnHub.css';

const LEARNING_SITE_BASE = 'https://www.100ask.net/ai-agent';
export function LearnCloudWorkspace({
  active,
  course,
  onBack,
  onLab,
  loggedIn,
  onLogin,
}: {
  active: boolean;
  course: LearningCourse;
  onBack: () => void;
  onLab: (id: number) => boolean;
  loggedIn: boolean;
  onLogin: () => void;
}) {
  const [enrollment, setEnrollment] = useState<CloudLearningEnrollment>();
  const [unitKey, setUnitKey] = useState('');
  const [detail, setDetail] = useState<CloudLearningUnitDetail | null>(null);
  const [tab, setTab] = useState<'document' | 'workspace' | 'lab' | 'summary'>('document');
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState('');
  const [message, setMessage] = useState('');
  const [question, setQuestion] = useState('');
  const [answer, setAnswer] = useState('');
  const [workspacePath, setWorkspacePath] = useState('');
  const [reload, setReload] = useState(0);
  const epoch = useRef(0);
  const lock = useRef(false);
  const unit = enrollment?.units.find((u) => u.unitKey === unitKey);
  const loadHome = useCallback(async () => {
    const home = await invokeCommand('learning_get_home');
    const found = home.enrollments.find((e) =>
      course.enrollmentId
        ? e.enrollmentId === course.enrollmentId
        : e.bundleKey === course.slug && e.version === course.version
    );
    return found;
  }, [course]);
  useEffect(() => {
    if (!active || !loggedIn) return;
    let cancelled = false;
    setLoading(true);
    setMessage('');
    void loadHome()
      .then((found) => {
        if (cancelled) return;
        setEnrollment(found);
        setUnitKey((current) =>
          found?.units.some((u) => u.unitKey === current)
            ? current
            : found?.units.find((u) => u.unitKey === found.currentUnit)?.unitKey ||
              found?.units[0]?.unitKey ||
              ''
        );
      })
      .catch((e) => {
        if (!cancelled) setMessage(formatLearnError(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [active, loggedIn, loadHome, reload]);
  useEffect(() => {
    const counter = epoch;
    const token = ++counter.current;
    setDetail(null);
    setAnswer('');
    setQuestion('');
    setWorkspacePath('');
    if (!active || !enrollment?.enrollmentId || !unitKey) return;
    setLoading(true);
    void invokeCommand('learning_get_unit', { enrollmentId: enrollment.enrollmentId, unitKey })
      .then((result) => {
        if (token === epoch.current) setDetail(result);
      })
      .catch((e) => {
        if (token === epoch.current) setMessage(formatLearnError(e));
      })
      .finally(() => {
        if (token === epoch.current) setLoading(false);
      });
    return () => {
      counter.current++;
    };
  }, [active, enrollment?.enrollmentId, unitKey, reload]);
  const run = async (name: string, fn: (token: number) => Promise<void>) => {
    if (!active || lock.current || !detail || !enrollment) return;
    lock.current = true;
    setBusy(name);
    setMessage('');
    const token = epoch.current;
    try {
      await fn(token);
    } catch (e) {
      if (token === epoch.current) setMessage(formatLearnError(e));
    } finally {
      lock.current = false;
      setBusy('');
    }
  };
  const updateProgress = (
    step: UpdateLearningUnitProgress['step'],
    state: UpdateLearningUnitProgress['state'],
    completionSource: UpdateLearningUnitProgress['completionSource']
  ) =>
    run('progress', async (token) => {
      if (!enrollment || !unit) return;
      await invokeCommand('learning_update_unit_progress', {
        enrollmentId: enrollment.enrollmentId,
        unitKey,
        progress: {
          step,
          state,
          completionSource,
          percent: state === 'completed' ? 100 : 10,
          revision:
            Math.max(
              enrollment.revision,
              unit.progress?.revision || 0,
              ...unit.stepProgress.map((p) => p.revision)
            ) + 1,
        },
      });
      const next = await loadHome();
      if (token === epoch.current) {
        setEnrollment(next);
        setMessage('学习进度已保存。');
      }
    });
  const openVideo = () =>
    run('video', async (token) => {
      const launch = await invokeCommand('learning_open_video', {
        enrollmentId: enrollment!.enrollmentId,
        unitKey,
      });
      if (token !== epoch.current) return;
      await openUrl(launch.url);
      setMessage('已请求打开本节视频；打开不等于学习完成。');
    });
  const installWorkspace = () =>
    run('workspace', async (token) => {
      const installed = await invokeCommand('learning_install_unit_workspace', {
        enrollmentId: enrollment!.enrollmentId,
        unitKey,
      });
      if (token !== epoch.current) return;
      setWorkspacePath(installed.installPath);
      setMessage(
        `代码快照 ${installed.commit} 已验签${installed.reused ? '并复用' : '并安装'}；安装不代表编译或实验通过。`
      );
    });
  const askTutor = () =>
    run('tutor', async (token) => {
      const result = await invokeCommand('learning_ask_tutor', {
        request: { enrollmentId: enrollment!.enrollmentId, unitKey, question: question.trim() },
      });
      if (token === epoch.current) setAnswer(result.answer);
    });
  const disabled = Boolean(busy) || loading || !detail;
  return (
    <div
      className="learn-journey learn-v2-page learn-cloud-workspace"
      data-testid="learn-cloud-workspace"
    >
      <LearnBreadcrumb items={['学习中心', course.title]}>
        <button className="learning-secondary" disabled={Boolean(busy)} onClick={onBack}>
          返回我的课程
        </button>
      </LearnBreadcrumb>
      <div className="learn-v2-body">
        <div className="learn-workspace-topbar">
          <strong>{course.title}</strong>
          <LearnBadge>v{enrollment?.version || course.version}</LearnBadge>
          <span>Ubuntu · 本课程工程待绑定</span>
          <span>开发板 · 未核验</span>
          <button
            className="learning-text-button"
            disabled={Boolean(busy) || loading}
            onClick={() => setReload((v) => v + 1)}
          >
            重新同步
          </button>
        </div>
        {message && (
          <p className="learn-notice" role="status">
            {message}
          </p>
        )}
        {!loggedIn ? (
          <section className="learn-panel learn-hub-section">
            <h2>登录后读取课程权益</h2>
            <button onClick={onLogin}>登录 100ASK 账号</button>
          </section>
        ) : !enrollment ? (
          <section className="learn-panel learn-hub-section">
            <h2>{loading ? '正在读取课程…' : '尚未取得这门课的学习版本'}</h2>
            <p>{course.summary}</p>
            <p>
              课程目录可预览。兑换订单并同步后，才能读取该账号对应的教材与工程；不会自动打开其他课程。
            </p>
            <ol>
              {course.chapters.map((c) => (
                <li key={c.id}>{c.title}</li>
              ))}
            </ol>
            <button onClick={() => void openUrl(`${LEARNING_SITE_BASE}/course/${course.slug}`)}>
              查看官网课程与权益
            </button>
          </section>
        ) : (
          <>
            <nav className="learn-course-chapters" aria-label="课程目录">
              {enrollment.units.map((u, i) => (
                <button
                  key={u.unitKey}
                  disabled={Boolean(busy)}
                  className={u.unitKey === unitKey ? 'active' : ''}
                  onClick={() => {
                    setUnitKey(u.unitKey);
                    setTab('document');
                  }}
                >
                  <span>{String(i + 1).padStart(2, '0')}</span>
                  {u.title}
                </button>
              ))}
            </nav>
            <div className="learn-cloud-grid">
              <nav className="learn-panel learn-cloud-steps" aria-label="实验学习步骤">
                <h3>本课学习步骤</h3>
                {(
                  [
                    ['document', '阅读与理解'],
                    ['workspace', '工程与版本'],
                    ['lab', '开发板验证'],
                    ['summary', '章节总结'],
                  ] as const
                ).map(([id, title], i) => (
                  <button
                    key={id}
                    className={tab === id ? 'active' : 'learning-secondary'}
                    aria-current={tab === id ? 'step' : undefined}
                    onClick={() => setTab(id)}
                  >
                    <span>{i + 1}</span>
                    {title}
                  </button>
                ))}
                <p>导航只切换内容，完成状态以实际记录为准。</p>
              </nav>
              <article className="learn-panel learn-cloud-content">
                <h1>{unit?.title || '课程暂未发布课时'}</h1>
                <p>{unit?.summary}</p>
                {loading ? (
                  <p role="status">正在校验并读取本节内容…</p>
                ) : !detail ? (
                  <p>本节内容尚未载入，请重新同步。</p>
                ) : tab === 'document' ? (
                  <>
                    <SafeMarkdown source={detail.documentMarkdown} />
                    <div className="learn-hub-actions">
                      <button
                        disabled={disabled}
                        onClick={() =>
                          void updateProgress('document', 'completed', 'user_confirmed')
                        }
                      >
                        我已阅读并理解
                      </button>
                      <button
                        className="learning-secondary"
                        disabled={disabled || !unit?.hasVideo}
                        onClick={() => void openVideo()}
                      >
                        打开本节视频
                      </button>
                      <button
                        className="learning-secondary"
                        disabled={disabled || !unit?.hasVideo}
                        onClick={() => void updateProgress('video', 'completed', 'user_confirmed')}
                      >
                        确认视频学完
                      </button>
                    </div>
                  </>
                ) : tab === 'workspace' ? (
                  <>
                    <h2>核对本课代码版本</h2>
                    <dl>
                      <dt>仓库</dt>
                      <dd>{unit?.workspaceRepository || '本节未提供'}</dd>
                      <dt>提交版本</dt>
                      <dd>{unit?.workspaceCommit || '待发布'}</dd>
                      <dt>快照摘要</dt>
                      <dd>{unit?.workspaceSha256 || '待发布'}</dd>
                    </dl>
                    <button
                      disabled={disabled || !unit?.hasWorkspace}
                      onClick={() => void installWorkspace()}
                    >
                      安装代码快照
                    </button>
                    {workspacePath && (
                      <p>
                        本地路径：<code>{workspacePath}</code>
                      </p>
                    )}
                    <p>
                      该快照保存在本机，不等于 Ubuntu 工程已同步。使用配套签名课程并绑定 Workbench
                      工程后，可进入 VS Code 远程开发。
                    </p>
                  </>
                ) : tab === 'lab' ? (
                  <>
                    <h2>按配套实验核对环境</h2>
                    <p>实验编号：{unit?.labId || '本节未配置'}</p>
                    <p>构建、部署与板端证据分别记录。进入页面不会建立硬件连接。</p>
                    <button
                      disabled={disabled || !unit?.labId}
                      onClick={() => {
                        if (unit && !onLab(unit.labId))
                          setMessage('本课实验尚未同步到目录，请返回后同步课程。');
                      }}
                    >
                      进入本课实验准备
                    </button>
                  </>
                ) : (
                  <>
                    <h2>章节总结与完成依据</h2>
                    {unit?.stepProgress.length ? (
                      unit.stepProgress.map((p) => (
                        <div className="learn-session-row" key={p.step}>
                          <strong>{p.step}</strong>
                          <span>{p.state}</span>
                          <small>{p.completionSource}</small>
                        </div>
                      ))
                    ) : (
                      <p>还没有已保存的学习进度。</p>
                    )}
                    <p>
                      人工阅读确认、代码快照安装和真机验证分别记录。尚无证据的项目不会标为通过。
                    </p>
                  </>
                )}
              </article>
              <aside className="learn-panel learn-cloud-agent">
                <h2>AI 学习助手</h2>
                <LearnBadge>只读辅导 · 当前课时</LearnBadge>
                <p>结合本课文档和版本解释问题。不会执行命令或代替真实验收。</p>
                <textarea
                  value={question}
                  maxLength={1000}
                  rows={6}
                  placeholder="哪里不明白？试着描述你的判断和现象。"
                  onChange={(e) => setQuestion(e.target.value)}
                />
                <button disabled={disabled || !question.trim()} onClick={() => void askTutor()}>
                  {busy === 'tutor' ? '正在分析…' : '向 AI 助教提问'}
                </button>
                {answer && <p className="learn-agent-answer">{answer}</p>}
              </aside>
            </div>
          </>
        )}
      </div>
      <LearnFooter />
    </div>
  );
}
