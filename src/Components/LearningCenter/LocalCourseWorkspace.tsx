import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { LynxIcon } from '../../CoreUI';
import { invokeCommand } from '../../Platform/IPC/Client';
import type {
  InstalledLocalCourse,
  LearnJourney,
  LearnAgentPlan,
  LocalLearningReport,
  LocalLearningSession,
  LocalLearningState,
  LocalLearningStepView,
  LocalHilApprovalResult,
  LocalHilPreparation,
  LocalStepActionResult,
  LocalTutorAnswer,
  LocalCommandDiagnostic,
  WorkbenchProject,
} from '../../Platform/IPC/Commands';
import { formatErrorMessage } from '../../Utils';
import { formatLocalLearningError } from './learn/errors';
import {
  localReportCurrencyText,
  localReportSoftwareResult,
  localReportProgress,
  learnLatestAttempt,
} from './learn/model';
import './LocalCourseWorkspace.css';
import { LearnFooter, LearnBadge } from './learn/LearnChrome';
import { LearnText } from './learn/LearnText';
import { LearnEngineering } from './learn/LearnEngineering';
import { LearnRecords } from './learn/LearnRecords';
import { LearnRevisit, LearnPrerequisites } from './learn/LearnRevisit';
import { LearnEvidence } from './learn/LearnEvidence';
import { SafeMarkdown } from './learn/LearnMarkdown';
import './learn/LearnJourney.css';
import './learn/LearnWorkspace.css';

const stateLabels: Record<LocalLearningState, string> = {
  reading: '阅读目标',
  checking_understanding: '理解检查',
  practicing: '动手实践',
  collecting_evidence: '收集证据',
  verifying: '核对结果',
  reflecting: '复盘反思',
  completed: '学习总结',
};

const journeyStages = [
  { title: '理解目标', detail: '了解本课目标与相关概念', view: 'text' },
  { title: '核对环境', detail: '确认开发主机、工程与板卡', view: 'engineering' },
  { title: '打开工程', detail: '阅读源码并确认开发位置', view: 'engineering' },
  { title: '构建运行', detail: '按课程规则完成开发实践', view: 'evidence' },
  { title: '观察与验证', detail: '结合日志与实际现象验证', view: 'evidence' },
  { title: '章节评测', detail: '回答问题并总结本课收获', view: 'assessment' },
] as const;

interface Props {
  active: boolean;
  initialSessionId?: string;
  initialReviewStepId?: string;
  initialCourseId?: string;
  initialCourseVersion?: string;
  onBack: () => void;
}

export const LocalCourseWorkspace: React.FC<Props> = ({
  active,
  onBack,
  initialSessionId,
  initialReviewStepId,
  initialCourseId,
  initialCourseVersion,
}) => {
  const operationLock = useRef(false);
  const [journey, setJourney] = useState<LearnJourney | null>(null);
  const [tab, setTab] = useState<'text' | 'engineering' | 'evidence' | 'assessment' | 'records'>(
    'text'
  );
  const [stageSelection, setStageSelection] = useState<{
    sessionId: string;
    index: number;
    view: string;
  } | null>(null);
  const [reviewLabId, setReviewLabId] = useState('');
  const [draftStatus, setDraftStatus] = useState('');
  const [courses, setCourses] = useState<InstalledLocalCourse[]>([]);
  const [sessions, setSessions] = useState<LocalLearningSession[]>([]);
  const [projects, setProjects] = useState<WorkbenchProject[]>([]);
  const [selectedCourse, setSelectedCourse] = useState(
    initialCourseId && initialCourseVersion ? `${initialCourseId}@${initialCourseVersion}` : ''
  );
  const [selectedProject, setSelectedProject] = useState('');
  const [step, setStep] = useState<LocalLearningStepView | null>(null);
  const [answer, setAnswer] = useState('');
  const [reflection, setReflection] = useState('');
  const [question, setQuestion] = useState('');
  const [hintLevel, setHintLevel] = useState<'L1' | 'L2' | 'L3' | 'L4'>('L1');
  const [agentPlan, setAgentPlan] = useState<LearnAgentPlan | null>(null);
  const [tutor, setTutor] = useState<LocalTutorAnswer | null>(null);
  const [selectedDiagnostic, setSelectedDiagnostic] = useState<LocalCommandDiagnostic | null>(null);
  const [action, setAction] = useState<LocalStepActionResult | null>(null);
  const [report, setReport] = useState<LocalLearningReport | null>(null);
  const reportSequence = useRef(0);
  const [hilPreparation, setHilPreparation] = useState<LocalHilPreparation | null>(null);
  const [hilApproval, setHilApproval] = useState<LocalHilApprovalResult | null>(null);
  const [hilConfirmation, setHilConfirmation] = useState('');
  const [hilEvidenceKind, setHilEvidenceKind] = useState('');
  const [hilEvidenceStatus, setHilEvidenceStatus] = useState<'PASS' | 'FAIL'>('PASS');
  const [hilSummary, setHilSummary] = useState('');
  const [hilObservation, setHilObservation] = useState('');
  const [hilArtifactSha256, setHilArtifactSha256] = useState('');
  const [hilDurationSeconds, setHilDurationSeconds] = useState('');
  const [busy, setBusy] = useState('');
  const [error, setError] = useState('');
  const navigationContext = useRef('');
  const navigationEpoch = useRef(0);
  useEffect(() => {
    navigationEpoch.current++;
  }, [
    active,
    initialSessionId,
    step?.session.sessionId,
    step?.session.currentStep,
    step?.session.state,
    journey?.attempt,
    reviewLabId,
  ]);
  navigationContext.current = JSON.stringify([
    active,
    step?.session.sessionId,
    step?.session.currentStep,
    step?.session.state,
    journey?.attempt,
    reviewLabId,
  ]);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  useEffect(() => {
    reportSequence.current++;
    setReport(null);
    return () => {
      reportSequence.current++;
    };
  }, [
    active,
    initialSessionId,
    step?.session.sessionId,
    step?.session.currentStep,
    step?.session.state,
    journey?.attempt,
  ]);

  useEffect(() => {
    setSelectedDiagnostic(null);
  }, [active, step?.session.sessionId, step?.session.currentStep, journey?.attempt]);

  useEffect(() => {
    if (!active || !step) return;
    let cancelled = false;
    setAgentPlan(null);
    void invokeCommand('learning_get_agent_plan', { sessionId: step.session.sessionId })
      .then((plan) => {
        if (!cancelled) setAgentPlan(plan);
      })
      .catch((reason) => {
        if (!cancelled) setError(formatErrorMessage(reason));
      });
    return () => {
      cancelled = true;
    };
  }, [active, step]);

  const refresh = useCallback(async () => {
    const [nextCourses, nextSessions, nextProjects] = await Promise.all([
      invokeCommand('learning_list_local_courses'),
      invokeCommand('learning_list_local_sessions'),
      invokeCommand('workbench_list_projects'),
    ]);
    setCourses(nextCourses);
    setSessions(nextSessions);
    setProjects(nextProjects);
    if (!selectedCourse && nextCourses[0]) {
      setSelectedCourse(`${nextCourses[0].courseId}@${nextCourses[0].version}`);
    }
    if (!selectedProject) {
      const first = nextProjects.find((item) => item.id !== null);
      if (first?.id) setSelectedProject(String(first.id));
    }
  }, [selectedCourse, selectedProject]);

  useEffect(() => {
    if (!active) return;
    void refresh().catch((reason) => setError(formatErrorMessage(reason)));
  }, [active, refresh]);

  const run = async (name: string, operation: () => Promise<void>) => {
    if (operationLock.current) return;
    operationLock.current = true;
    setBusy(name);
    setError('');
    try {
      await operation();
    } catch (reason) {
      setError(formatLocalLearningError(reason));
    } finally {
      operationLock.current = false;
      setBusy('');
    }
  };

  const loadStep = async (
    sessionId: string,
    reviewStepId?: string,
    isCurrent: () => boolean = () => true
  ) => {
    const [next, context] = await Promise.all([
      invokeCommand('learning_get_local_step', { sessionId }),
      invokeCommand('learning_get_journey', { sessionId }),
    ]);
    if (!isCurrent()) return;
    const selectedReview = context.labs.some((lab) => lab.stepId === reviewStepId)
      ? reviewStepId!
      : '';
    setStep(next);
    setJourney(context);
    setSelectedCourse(`${next.session.courseId}@${next.session.courseVersion}`);
    setSelectedProject(next.session.projectId > 0 ? String(next.session.projectId) : '');
    setAnswer(context.answer);
    setReflection(context.reflection);
    setReviewLabId(selectedReview);
    setAction(null);
    setReport(null);
    setTutor(null);
    setQuestion('');
    setHilPreparation(null);
    setHilApproval(null);
    setHilConfirmation('');
    setHilEvidenceKind(next.lab.hilVerification?.requiredEvidenceKinds[0] ?? '');
    setDraftStatus('已恢复本地记录');
    setTab(
      selectedReview
        ? 'text'
        : next.session.state === 'completed'
          ? 'records'
          : next.session.state === 'reflecting'
            ? 'assessment'
            : ['practicing', 'collecting_evidence', 'verifying'].includes(next.session.state)
              ? 'evidence'
              : 'text'
    );
  };

  useEffect(() => {
    if (active && initialSessionId)
      void run('resume', () => loadStep(initialSessionId, initialReviewStepId));
    // Resume is an explicit navigation request, not an automatic hardware action.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [initialSessionId, initialReviewStepId]);

  const saveDraft = async () => {
    if (!step || !journey) return;
    await invokeCommand('learning_save_draft', {
      sessionId: step.session.sessionId,
      stepId: step.session.currentStep,
      attempt: journey.attempt,
      answer,
      reflection,
    });
    setDraftStatus('草稿已保存到本地');
  };

  useEffect(() => {
    if (!active || !step || !journey) return;
    let cancelled = false;
    setDraftStatus('正在保存草稿…');
    const timer = setTimeout(() => {
      void invokeCommand('learning_save_draft', {
        sessionId: step.session.sessionId,
        stepId: step.session.currentStep,
        attempt: journey.attempt,
        answer,
        reflection,
      })
        .then(() => {
          if (!cancelled) setDraftStatus('草稿已保存到本地');
        })
        .catch((reason) => {
          if (!cancelled) setDraftStatus(`保存失败：${formatErrorMessage(reason)}`);
        });
    }, 400);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [active, step, journey, answer, reflection]);

  const importCourse = () =>
    run('import', async () => {
      const path = await open({
        multiple: false,
        directory: false,
        filters: [{ name: 'LYNX 本地课程包', extensions: ['lynx-course'] }],
      });
      if (typeof path !== 'string') return;
      const installed = await invokeCommand('learning_import_local_course', { path });
      setSelectedCourse(`${installed.courseId}@${installed.version}`);
      await refresh();
    });

  const startSession = () =>
    run('start', async () => {
      const split = selectedCourse.lastIndexOf('@');
      if (split < 1 || (entryRequiresProject && !selectedProject))
        throw new Error('请选择课程版本和 Workbench SDK 工程');
      const session = await invokeCommand('learning_start_session', {
        courseId: selectedCourse.slice(0, split),
        version: selectedCourse.slice(split + 1),
        projectId: entryRequiresProject ? Number(selectedProject) : 0,
      });
      await loadStep(session.sessionId);
      await refresh();
    });

  const transition = (nextState: LocalLearningState) =>
    run(`state-${nextState}`, async () => {
      if (!step) return;
      await saveDraft();
      const session = await invokeCommand('learning_transition_step', {
        sessionId: step.session.sessionId,
        nextState,
        ...(answer.trim() ? { answer: answer.trim() } : {}),
        ...(reflection.trim() ? { reflection: reflection.trim() } : {}),
      });
      await loadStep(session.sessionId);
      await refresh();
    });

  const skipOptional = () =>
    run('skip-optional', async () => {
      if (!step || !journey || step.lab.required !== false) return;
      const request = {
        sessionId: step.session.sessionId,
        expectedStepId: step.session.currentStep,
        expectedAttempt: journey.attempt,
        expectedState: step.session.state,
      };
      await saveDraft();
      const session = await invokeCommand('learning_skip_optional_activity', { request });
      await loadStep(session.sessionId);
      await refresh();
    });

  const revisitActivity = (targetStepId: string) =>
    run('revisit', async () => {
      if (!active || !step || !journey) return;
      const target = learnLatestAttempt(journey.attempts, targetStepId);
      const access = journey.activityAccess?.find((a) => a.stepId === targetStepId);
      if ((!target && !access?.available) || access?.available === false) return;
      const origin = navigationContext.current;
      const epoch = navigationEpoch.current;
      const isCurrent = () =>
        mounted.current &&
        navigationContext.current === origin &&
        navigationEpoch.current === epoch;
      const request = {
        sessionId: step.session.sessionId,
        expectedStepId: step.session.currentStep,
        expectedAttempt: journey.attempt,
        expectedState: step.session.state,
        targetStepId,
        expectedTargetAttempt: target?.attempt ?? 0,
      };
      await saveDraft();
      if (!isCurrent()) return;
      const selected = await invokeCommand('learning_revisit_activity', { request });
      if (!isCurrent()) return;
      await loadStep(selected.sessionId, undefined, isCurrent);
    });

  const executeAction = () =>
    run('action', async () => {
      if (!step) return;
      const result = await invokeCommand('learning_run_step_action', {
        sessionId: step.session.sessionId,
        credentials: {},
      });
      setAction(result);
      const next = await invokeCommand('learning_get_local_step', {
        sessionId: step.session.sessionId,
      });
      setStep(next);
      setJourney(
        await invokeCommand('learning_get_journey', { sessionId: next.session.sessionId })
      );
    });

  const askTutor = () =>
    run('tutor', async () => {
      if (!step || !question.trim()) return;
      const diagnostic = selectedDiagnostic;
      if (
        diagnostic &&
        (diagnostic.sessionId !== step.session.sessionId ||
          diagnostic.stepId !== step.session.currentStep ||
          diagnostic.attempt !== journey?.attempt)
      ) {
        setSelectedDiagnostic(null);
        throw new Error('诊断所属尝试已变化，请重新选择当前记录。');
      }
      await saveDraft();
      setTutor(
        await invokeCommand('learning_ask_local_tutor', {
          request: {
            sessionId: step.session.sessionId,
            question: question.trim(),
            hintLevel,
            expectedStepId: step.session.currentStep,
            expectedAttempt: journey?.attempt,
            ...(diagnostic ? { diagnosticDigest: diagnostic.digest } : {}),
          },
        })
      );
      if (diagnostic) setSelectedDiagnostic((current) => (current === diagnostic ? null : current));
    });

  const prepareHil = () =>
    run('hil-prepare', async () => {
      if (!step) return;
      const prepared = await invokeCommand('learning_prepare_hil_action', {
        sessionId: step.session.sessionId,
      });
      setHilPreparation(prepared);
      setHilEvidenceKind(prepared.requiredEvidenceKinds[0] ?? '');
      setHilApproval(null);
      setHilConfirmation('');
    });

  const confirmHil = () =>
    run('hil-confirm', async () => {
      if (!step || !hilPreparation) return;
      setHilApproval(
        await invokeCommand('learning_confirm_hil_action', {
          request: {
            sessionId: step.session.sessionId,
            approvalToken: hilPreparation.approvalToken,
            confirmation: hilConfirmation,
          },
        })
      );
      setStep(
        await invokeCommand('learning_get_local_step', { sessionId: step.session.sessionId })
      );
      setJourney(
        await invokeCommand('learning_get_journey', { sessionId: step.session.sessionId })
      );
    });

  const attachHilEvidence = () =>
    run('hil-evidence', async () => {
      if (!step || !journey) return;
      await invokeCommand('learning_attach_hil_evidence', {
        request: {
          sessionId: step.session.sessionId,
          expectedStepId: step.session.currentStep,
          expectedAttempt: journey.attempt,
          evidenceKind: hilEvidenceKind,
          status: hilEvidenceStatus,
          summary: hilSummary.trim(),
          observation: hilObservation.trim(),
          ...(hilArtifactSha256.trim() ? { artifactSha256: hilArtifactSha256.trim() } : {}),
          ...(hilDurationSeconds ? { durationSeconds: Number(hilDurationSeconds) } : {}),
        },
      });
      setHilSummary('');
      setHilObservation('');
      setHilArtifactSha256('');
      setHilDurationSeconds('');
      setStep(
        await invokeCommand('learning_get_local_step', { sessionId: step.session.sessionId })
      );
      setJourney(
        await invokeCommand('learning_get_journey', { sessionId: step.session.sessionId })
      );
    });

  const finalizeHil = () =>
    run('hil-finalize', async () => {
      if (!step) return;
      const result = await invokeCommand('learning_finalize_hil_step', {
        sessionId: step.session.sessionId,
      });
      setAction(result);
      setStep(
        await invokeCommand('learning_get_local_step', { sessionId: step.session.sessionId })
      );
      setJourney(
        await invokeCommand('learning_get_journey', { sessionId: step.session.sessionId })
      );
      await refresh();
    });

  const makeReport = () =>
    run('report', async () => {
      if (!active || !step) return;
      const request = ++reportSequence.current;
      const sessionId = step.session.sessionId;
      setReport(null);
      try {
        const result = await invokeCommand('learning_generate_local_report', { sessionId });
        if (request === reportSequence.current && result.session.sessionId === sessionId)
          setReport(result);
      } catch (reason) {
        if (request === reportSequence.current) throw reason;
      }
    });

  const currentCourse = useMemo(
    () => courses.find((item) => `${item.courseId}@${item.version}` === selectedCourse),
    [courses, selectedCourse]
  );
  const entryRequiresProject = currentCourse?.entryRequiresProject !== false;
  const reflectionActivity = step?.lab.activityKind === 'reflection';
  const stages = reflectionActivity
    ? [
        { title: '理解目标', detail: '阅读教材并回答问题', view: 'text' as const },
        { title: '整理理解与复盘', detail: '记录自述，不自动评分', view: 'assessment' as const },
        { title: '学习档案', detail: '回顾本地记录', view: 'records' as const },
      ]
    : journeyStages;
  const softwareReady =
    journey?.evidence.find(
      (item) =>
        item.stepId === step?.session.currentStep &&
        item.attempt === journey.attempt &&
        item.kind === 'command' &&
        item.source === 'software'
    )?.status === 'PASS';

  const boundProject = projects.find((project) => project.id === step?.session.projectId);
  const reviewLab = journey?.labs.find((lab) => lab.stepId === reviewLabId);
  const currentAccess = journey?.activityAccess?.find(
    (a) => a.stepId === step?.session.currentStep
  );
  const prerequisiteBlocked = currentAccess?.available === false;
  const graphNext =
    journey?.activityAccess?.length && step
      ? [...journey.labs.slice(step.stepIndex + 1), ...journey.labs.slice(0, step.stepIndex)].find(
          (lab) =>
            journey.activityAccess?.find((a) => a.stepId === lab.stepId)?.available &&
            !['completed', 'skipped'].includes(
              learnLatestAttempt(journey.attempts, lab.stepId)?.state || ''
            )
        )
      : undefined;

  return (
    <div
      className="learning-page local-course-workspace learn-journey learn-workspace-v2"
      data-view={tab}
      data-session={Boolean(step)}
      data-testid="local-course-workspace"
    >
      <header className="local-course-header">
        <button
          className="learning-secondary"
          onClick={() =>
            void run('leave', async () => {
              await saveDraft();
              onBack();
            })
          }
          disabled={Boolean(busy)}
        >
          <LynxIcon name="chevron-left" size={16} /> 返回
        </button>
        <div className="local-course-heading">
          <div className="local-course-hero-mark" aria-hidden="true">
            <span>LEARN</span>
            <small>AI LAB</small>
          </div>
          <div className="local-course-heading-copy">
            <span className="learning-section-eyebrow">离线模式 · 本地签名课程包</span>
            <h1>{journey?.title || currentCourse?.title || '离线课程包学习'}</h1>
            <p>此处仅用于本地签名课程包。在线课程请返回学习指南，从官网同步后进入。</p>
          </div>
          <div className="local-course-hero-facts" aria-label="课程能力摘要">
            <span>
              <strong>{currentCourse?.labCount ?? '—'}</strong> 课时
            </span>
            <span>
              <strong>Ed25519</strong> 本地验签
            </span>
            <span>
              <strong>1:1</strong>{' '}
              {reflectionActivity || !entryRequiresProject ? '教材追溯' : '源码追溯'}
            </span>
          </div>
        </div>
        <button onClick={importCourse} disabled={Boolean(busy)} data-testid="local-course-import">
          <LynxIcon name="download" size={16} /> {busy === 'import' ? '验签中…' : '导入课程包'}
        </button>
      </header>

      {error && <div className="local-course-error">{error}</div>}

      {!step ? (
        <div className="local-course-picker" data-testid="learn-session-setup">
          <section>
            <div className="local-course-card-heading">
              <span>01</span>
              <div>
                <small>COURSE PACKAGE</small>
                <h2>选择已验签课程</h2>
              </div>
            </div>
            {courses.length === 0 ? (
              <div className="local-course-empty">
                <LynxIcon name="document" size={24} />
                <strong>等待本地课程包</strong>
                <p>选择 `.lynx-course` 文件后，LYNX 会先验签、再逐文件核对 provenance。</p>
              </div>
            ) : (
              <select
                value={selectedCourse}
                onChange={(event) => setSelectedCourse(event.target.value)}
              >
                {courses.map((item) => (
                  <option
                    key={`${item.courseId}@${item.version}`}
                    value={`${item.courseId}@${item.version}`}
                  >
                    {item.title} · {item.version} · {item.labCount} 课
                  </option>
                ))}
              </select>
            )}
            {currentCourse && (
              <dl>
                <div>
                  <dt>板卡</dt>
                  <dd>{currentCourse.boardProfile}</dd>
                </div>
                <div>
                  <dt>包 SHA256</dt>
                  <dd>{currentCourse.packageSha256.slice(0, 20)}…</dd>
                </div>
                <div>
                  <dt>签名指纹</dt>
                  <dd>{currentCourse.signerFingerprint.slice(0, 20)}…</dd>
                </div>
              </dl>
            )}
          </section>
          <section>
            <div className="local-course-card-heading">
              <span>02</span>
              <div>
                <small>SDK WORKSPACE</small>
                <h2>{entryRequiresProject ? '绑定 Workbench 工程' : '直接开始阅读'}</h2>
              </div>
            </div>
            {entryRequiresProject ? (
              <>
                <select
                  value={selectedProject}
                  onChange={(event) => setSelectedProject(event.target.value)}
                >
                  <option value="">请选择工程</option>
                  {projects
                    .filter((item) => item.id !== null)
                    .map((item) => (
                      <option key={item.id!} value={item.id!}>
                        {item.name} · {item.sdkDir}
                      </option>
                    ))}
                </select>
                <p>
                  实际命令仅在课程声明的工程子目录执行；旧版 KVM 包沿用
                  tutorial-examples/web-kvm。认证信息沿用系统密钥环。
                </p>
              </>
            ) : (
              <p>
                可以先阅读课程；选修活动也可明确跳过。开始实践时再按本活动要求绑定工程；理解回答与复盘不自动评分。
              </p>
            )}
            <button
              disabled={
                !selectedCourse || (entryRequiresProject && !selectedProject) || Boolean(busy)
              }
              onClick={startSession}
            >
              创建学习会话
            </button>
          </section>
          <section>
            <div className="local-course-card-heading">
              <span>03</span>
              <div>
                <small>LEARNING RECORD</small>
                <h2>继续已有会话</h2>
              </div>
            </div>
            <div className="local-course-session-list">
              {sessions.length === 0 && <p>还没有本地学习记录。</p>}
              {sessions.map((session) => (
                <button
                  key={session.sessionId}
                  onClick={() => void run('resume', () => loadStep(session.sessionId))}
                >
                  <strong>{session.currentStep}</strong>
                  <span>
                    {stateLabels[session.state]} · {session.classification}
                    {session.recordedClassification &&
                      session.recordedClassification !== session.classification && (
                        <> · 原分类 {session.recordedClassification}（实机未验证）</>
                      )}
                  </span>
                </button>
              ))}
            </div>
          </section>
        </div>
      ) : (
        <div className="local-course-session-shell">
          <div className="learn-workspace-topbar">
            <button
              className="learning-secondary"
              disabled={Boolean(busy)}
              onClick={() =>
                void run('leave', async () => {
                  await saveDraft();
                  onBack();
                })
              }
            >
              返回实验中心
            </button>
            <div>
              <LynxIcon name="book" size={20} />
              <strong>{journey?.title || step.session.courseId}</strong>
              <span>/</span>
              <strong>{step.lab.title}</strong>
              <span>
                第 {step.stepIndex + 1} / {step.totalSteps} 课
              </span>
            </div>
            <details>
              <summary>课程目录 ▾</summary>
              <div className="learn-directory">
                {journey?.labs.map((lab) => (
                  <button
                    className="learning-secondary"
                    key={lab.stepId}
                    onClick={() => {
                      setReviewLabId(lab.stepId === step.session.currentStep ? '' : lab.stepId);
                      setTab('text');
                    }}
                  >
                    {lab.number}. {lab.title}{' '}
                    {lab.stepId === step.session.currentStep ? '· 当前' : ''}
                    {journey?.activityAccess?.find((a) => a.stepId === lab.stepId)?.available ===
                    false
                      ? ' · 前置未满足'
                      : ''}
                  </button>
                ))}
              </div>
            </details>
            {reflectionActivity ? (
              <span className="learn-notice">本活动无需工程或开发板 · 自述未评分</span>
            ) : (
              <>
                <button className="learning-secondary" onClick={() => setTab('engineering')}>
                  <i className="learn-status-dot" />
                  Ubuntu · {boundProject ? '已配置 · 待检查' : '未配置'}
                </button>
                <button className="learning-secondary" onClick={() => setTab('evidence')}>
                  <i className="learn-status-dot" />
                  开发板 · {step.session.boardUid ? '已绑定 · 待核验' : '未绑定'}
                </button>
              </>
            )}
          </div>
          <nav className="learn-course-chapters" aria-label="相邻课程章节">
            {journey?.labs
              .slice(Math.max(0, step.stepIndex - 1), step.stepIndex + 2)
              .map((chapter) => (
                <button
                  key={chapter.stepId}
                  className={chapter.stepId === step.session.currentStep ? 'active' : ''}
                  aria-current={chapter.stepId === step.session.currentStep ? 'page' : undefined}
                  onClick={() => {
                    setReviewLabId(
                      chapter.stepId === step.session.currentStep ? '' : chapter.stepId
                    );
                    setTab('text');
                  }}
                >
                  <span>{String(chapter.number).padStart(2, '0')}</span>
                  {chapter.title}
                </button>
              ))}
          </nav>
          <nav className="local-course-stage-rail" aria-label="本课学习阶段">
            <h3>{reflectionActivity ? '学习步骤' : '实验步骤'}</h3>
            {stages.map((stage, index) => {
              const derivedStage = reflectionActivity
                ? tab === 'records'
                  ? 2
                  : tab === 'assessment'
                    ? 1
                    : 0
                : tab === 'engineering'
                  ? 1
                  : tab === 'assessment'
                    ? 5
                    : ['collecting_evidence', 'verifying'].includes(step.session.state)
                      ? 4
                      : step.session.state === 'practicing'
                        ? 3
                        : step.session.state === 'reflecting'
                          ? 5
                          : 0;
              const current =
                stageSelection?.sessionId === step.session.sessionId && stageSelection.view === tab
                  ? stageSelection.index
                  : derivedStage;
              return (
                <button
                  key={stage.title}
                  className={index === current ? 'active' : ''}
                  aria-current={index === current ? 'step' : undefined}
                  onClick={() => {
                    setReviewLabId('');
                    setTab(stage.view);
                    setStageSelection({
                      sessionId: step.session.sessionId,
                      index,
                      view: stage.view,
                    });
                  }}
                >
                  <i>{index + 1}</i>
                  <span>
                    <strong>{stage.title}</strong>
                    <small>{stage.detail}</small>
                  </span>
                </button>
              );
            })}
          </nav>
          <div className="local-course-grid">
            <section className="local-course-main" aria-busy={Boolean(busy)}>
              {prerequisiteBlocked && tab !== 'text' && (
                <div data-testid="local-current-prerequisites">
                  <LearnPrerequisites
                    access={currentAccess}
                    onReview={(id) => {
                      setReviewLabId(id);
                      setTab('text');
                    }}
                  />
                </div>
              )}
              <nav className="learn-workspace-tabs" aria-label="学习内容视图">
                {(
                  [
                    ['text', '教材'],
                    ['engineering', '工程与版本'],
                    ['evidence', '运行与证据'],
                  ] as const
                )
                  .filter(([key]) => !reflectionActivity || key !== 'engineering')
                  .map(([key, label]) => (
                    <button
                      key={key}
                      className={tab === key ? 'active' : ''}
                      aria-pressed={tab === key}
                      onClick={() => {
                        setStageSelection(null);
                        setTab(key);
                      }}
                    >
                      <LynxIcon
                        name={
                          key === 'text' ? 'book' : key === 'engineering' ? 'folder' : 'terminal'
                        }
                        size={24}
                      />
                      {reflectionActivity && key === 'evidence' ? '学习记录' : label}
                    </button>
                  ))}
              </nav>
              {tab === 'engineering' &&
                !reflectionActivity &&
                (step.session.projectId > 0 ? (
                  <LearnEngineering
                    active={active}
                    attempt={journey?.attempt}
                    key={step.session.sessionId}
                    step={step}
                    project={boundProject}
                  />
                ) : (
                  <section className="learn-panel" data-testid="local-deferred-project">
                    <h2>为本次实践绑定工程</h2>
                    <p>前面的阅读记录已保留。选择已保存的 Workbench 工程后，才能继续实践。</p>
                    <select
                      aria-label="实践工程"
                      value={selectedProject}
                      onChange={(e) => setSelectedProject(e.target.value)}
                    >
                      <option value="">请选择工程</option>
                      {projects
                        .filter((p) => p.id !== null)
                        .map((p) => (
                          <option key={p.id!} value={p.id!}>
                            {p.name} · {p.sdkDir}
                          </option>
                        ))}
                    </select>
                    <button
                      disabled={!selectedProject || Boolean(busy)}
                      onClick={() =>
                        void run('bind-project', async () => {
                          await saveDraft();
                          const session = await invokeCommand('learning_bind_local_project', {
                            sessionId: step.session.sessionId,
                            projectId: Number(selectedProject),
                          });
                          await loadStep(session.sessionId);
                          await refresh();
                        })
                      }
                    >
                      绑定此工程
                    </button>
                  </section>
                ))}
              {tab === 'records' && (
                <LearnRecords
                  active={active}
                  initialSessionId={step.session.sessionId}
                  onResume={(_id, target) => {
                    if (target) {
                      setReviewLabId(target);
                      setTab('text');
                    }
                  }}
                />
              )}
              {tab === 'assessment' && (
                <section className="learn-assessment">
                  <div className="learn-assessment-heading">
                    <button className="learning-text-button" onClick={() => setTab('text')}>
                      ‹ 返回课程
                    </button>
                    <span>{journey?.title}</span>
                  </div>
                  <h1>这一课，你掌握了什么？</h1>
                  <p>
                    <LearnBadge tone={softwareReady ? 'green' : 'orange'}>
                      {reflectionActivity
                        ? '理论自述 · 未评分'
                        : softwareReady
                          ? '软件实践已验证'
                          : '实践待验证'}
                    </LearnBadge>{' '}
                    · 理解与实机结果分别记录
                  </p>
                  <div className="learn-assessment-columns">
                    <aside className="learn-assessment-goals">
                      <h2>
                        <LynxIcon name="check-circle" size={24} /> 本课目标
                      </h2>
                      <p>完成本课学习后，你应能够：</p>
                      <div className="learn-notice">{step.lab.learningObjective}</div>
                      <h3>观察与解释</h3>
                      <p>{step.lab.expectedObservation}</p>
                      <button
                        className="learning-secondary"
                        onClick={() => {
                          setReviewLabId('');
                          setTab('text');
                        }}
                      >
                        返回本课补学
                      </button>
                    </aside>
                    <div>
                      <section className="learn-panel">
                        <h2>
                          <LynxIcon name="chart" size={24} />{' '}
                          {reflectionActivity ? '学习记录' : '实践结果'}
                        </h2>
                        <p>根据本次尝试的实际记录进行核对。</p>
                        <table className="learn-table">
                          <thead>
                            <tr>
                              <th>检查项</th>
                              <th>结果</th>
                              <th>依据</th>
                            </tr>
                          </thead>
                          <tbody>
                            <tr>
                              <td>软件校验</td>
                              <td>
                                <LearnBadge tone={softwareReady ? 'green' : 'orange'}>
                                  {reflectionActivity
                                    ? '本活动不要求'
                                    : softwareReady
                                      ? '通过'
                                      : '待验证'}
                                </LearnBadge>
                              </td>
                              <td>
                                {reflectionActivity ? '本活动无需软件验证' : '本次软件执行证据'}
                              </td>
                            </tr>
                            <tr>
                              <td>实机验证</td>
                              <td>
                                {step.lab.hilVerification ? '尚未验证（需设备证据）' : '本课未要求'}
                              </td>
                              <td>
                                {reflectionActivity ? '本活动无需实机验证' : '课程规则与板端证据'}
                              </td>
                            </tr>
                            <tr>
                              <td>理解回答</td>
                              <td>{answer.trim() ? '已记录' : '待回答'}</td>
                              <td>原文记录 · 非 AI 评分</td>
                            </tr>
                          </tbody>
                        </table>
                        <button className="learning-text-button" onClick={() => setTab('evidence')}>
                          查看本次证据 ›
                        </button>
                      </section>
                      <section className="learn-panel learn-assessment-answer">
                        <h2>理解检查</h2>
                        <label>{step.lab.comprehensionCheck}</label>
                        <textarea
                          value={answer}
                          onChange={(event) => setAnswer(event.target.value)}
                          maxLength={4000}
                          placeholder="用自己的话解释本课结果"
                          disabled={Boolean(busy) || step.session.state !== 'reflecting'}
                        />
                        <small>{answer.length} / 4000 · 回答保留原文，不自动等同理解达标</small>
                      </section>
                    </div>
                  </div>
                </section>
              )}
              {tab === 'text' && reviewLab && (
                <section className="learn-panel">
                  <span className="learn-tag">目录预览 · 不改变当前学习位置</span>
                  <h1>{reviewLab.title}</h1>
                  <LearnText
                    active={active}
                    sessionId={step.session.sessionId}
                    stepId={reviewLab.stepId}
                    title={reviewLab.title}
                  />
                  <p>{reviewLab.learningObjective}</p>
                  <h3>实践任务</h3>
                  <p>{reviewLab.studentAction}</p>
                  <h3>预期现象</h3>
                  <p>{reviewLab.expectedObservation}</p>
                  <button onClick={() => setReviewLabId('')}>返回当前课时</button>
                </section>
              )}
              {tab === 'text' && (
                <LearnRevisit
                  lab={reviewLab || step.lab}
                  attempt={
                    journey
                      ? learnLatestAttempt(journey.attempts, (reviewLab || step.lab).stepId)
                      : undefined
                  }
                  current={(reviewLab || step.lab).stepId === step.session.currentStep}
                  access={journey?.activityAccess?.find(
                    (a) => a.stepId === (reviewLab || step.lab).stepId
                  )}
                  onReview={(id) => {
                    setReviewLabId(id === step.session.currentStep ? '' : id);
                    setTab('text');
                  }}
                  busy={Boolean(busy)}
                  onContinue={() => revisitActivity((reviewLab || step.lab).stepId)}
                />
              )}
              <div hidden={tab !== 'text' || Boolean(reviewLab)}>
                <LearnText
                  title={step.lab.title}
                  active={active && tab === 'text' && !reviewLab}
                  sessionId={step.session.sessionId}
                  stepId={step.session.currentStep}
                />
                <div className="learn-objective-cards">
                  <div className="local-course-callout objective">
                    <strong>
                      <span>01</span> 学习目标
                    </strong>
                    <p>{step.lab.learningObjective}</p>
                  </div>
                  <div className="local-course-callout action">
                    <strong>
                      <span>02</span> 学生动作
                    </strong>
                    <p>{step.lab.studentAction}</p>
                  </div>
                  <div className="local-course-callout observation">
                    <strong>
                      <span>03</span> 期望观察
                    </strong>
                    <p>{step.lab.expectedObservation}</p>
                  </div>
                </div>
                <details className="learn-rule-details">
                  <summary>本课验证规则</summary>
                  <div className="local-course-rule">
                    <span>受控校验命令</span>
                    <code>{step.lab.verificationRules[0]?.command}</code>
                    <small>命令来自已签名 manifest，页面不能提交任意 shell。</small>
                  </div>
                </details>
              </div>
              <div hidden={tab !== 'evidence'}>
                <LearnEvidence
                  key={`${step.session.sessionId}-${step.session.currentStep}`}
                  active={active && tab === 'evidence'}
                  step={step}
                  journey={journey}
                  onSelectDiagnostic={(value) => {
                    setSelectedDiagnostic(value);
                    setQuestion(
                      (current) =>
                        current || '请根据这条诊断，解释失败原因并给出下一步检查建议，不执行命令。'
                    );
                  }}
                  onSaved={async () =>
                    setJourney(
                      await invokeCommand('learning_get_journey', {
                        sessionId: step.session.sessionId,
                      })
                    )
                  }
                />
                {step.lab.hilVerification && (
                  <section className="local-course-hil" data-testid="local-course-hil-panel">
                    <div className="local-course-hil-heading">
                      <div>
                        <span>HIL EVIDENCE GATE</span>
                        <h2>实机证据门禁</h2>
                      </div>
                      <strong>{step.session.boardUid || '未绑定板卡'}</strong>
                    </div>
                    <p className="learn-notice">
                      当前本地课程仅保存人工观察，尚未接入可信设备执行与采集回执。手填结果、镜像摘要或持续时间不能证明实机通过。
                    </p>
                    <p>{step.lab.hilVerification.instructions}</p>
                    <ul>
                      {step.lab.hilVerification.expectedObservations.map((item) => (
                        <li key={item}>{item}</li>
                      ))}
                    </ul>
                    {step.lab.hilVerification.image && (
                      <dl className="local-course-hil-image">
                        <div>
                          <dt>镜像</dt>
                          <dd>{step.lab.hilVerification.image.path}</dd>
                        </div>
                        <div>
                          <dt>MD5</dt>
                          <dd>{step.lab.hilVerification.image.md5}</dd>
                        </div>
                        <div>
                          <dt>SHA256</dt>
                          <dd>{step.lab.hilVerification.image.sha256}</dd>
                        </div>
                      </dl>
                    )}
                    {!softwareReady && step.session.state === 'practicing' && (
                      <div className="local-course-hil-pending">
                        先执行并通过本课的软件校验，硬件确认入口才会开放。
                      </div>
                    )}
                    {softwareReady && step.session.state === 'practicing' && !hilPreparation && (
                      <button className="learning-secondary" onClick={prepareHil}>
                        核对观察目标并生成确认
                      </button>
                    )}
                    {hilPreparation && !hilApproval && (
                      <div className="local-course-hil-confirm">
                        <label>
                          输入确认短语
                          <code>{hilPreparation.confirmationPhrase}</code>
                        </label>
                        <input
                          value={hilConfirmation}
                          onChange={(event) => setHilConfirmation(event.target.value)}
                          placeholder="逐字输入上方确认短语"
                        />
                        <button
                          disabled={hilConfirmation !== hilPreparation.confirmationPhrase}
                          onClick={confirmHil}
                        >
                          确认本次观察范围
                        </button>
                      </div>
                    )}
                    {hilApproval && step.session.state === 'practicing' && (
                      <div className="local-course-hil-evidence-form">
                        <div className="local-course-hil-approved">
                          已确认观察范围 {hilApproval.actionId} · 目标 {hilApproval.boardUid}
                          ；未执行硬件操作
                        </div>
                        <select
                          value={hilEvidenceKind}
                          onChange={(event) => setHilEvidenceKind(event.target.value)}
                        >
                          {step.lab.hilVerification.requiredEvidenceKinds.map((kind) => (
                            <option value={kind} key={kind}>
                              {kind}
                            </option>
                          ))}
                        </select>
                        <select
                          value={hilEvidenceStatus}
                          onChange={(event) =>
                            setHilEvidenceStatus(event.target.value as 'PASS' | 'FAIL')
                          }
                        >
                          <option value="PASS">自述：观测符合（未验证）</option>
                          <option value="FAIL">自述：观测不符合（未验证）</option>
                        </select>
                        <input
                          value={hilSummary}
                          onChange={(event) => setHilSummary(event.target.value)}
                          placeholder="人工观察摘要"
                        />
                        <textarea
                          value={hilObservation}
                          onChange={(event) => setHilObservation(event.target.value)}
                          placeholder="记录人工观察或粘贴日志；均按自述保存，不能证明设备来源"
                        />
                        {hilEvidenceKind === 'flash-verification' && (
                          <input
                            value={hilArtifactSha256}
                            onChange={(event) => setHilArtifactSha256(event.target.value)}
                            placeholder="自述镜像 SHA256（不代表已烧录）"
                          />
                        )}
                        {hilEvidenceKind === 'soak-observation' && (
                          <input
                            type="number"
                            min={step.lab.hilVerification.minimumDurationSeconds ?? 0}
                            value={hilDurationSeconds}
                            onChange={(event) => setHilDurationSeconds(event.target.value)}
                            placeholder="自述运行秒数（不代表已实测）"
                          />
                        )}
                        <div className="local-course-actions">
                          <button
                            disabled={!hilSummary.trim() || !hilObservation.trim()}
                            onClick={attachHilEvidence}
                          >
                            保存人工观察（未验证）
                          </button>
                          <button className="learning-secondary" onClick={finalizeHil}>
                            检查实机证据缺口
                          </button>
                        </div>
                      </div>
                    )}
                    {step.evidence.filter((item) => item.source !== 'software').length > 0 && (
                      <div className="local-course-hil-evidence-list">
                        <p>观察与核验历史（自述、旧规则结论均不代表设备通过）</p>
                        {step.evidence
                          .filter((item) => item.source !== 'software')
                          .map((item) => (
                            <div key={`${item.digest}-${item.createdAt}`}>
                              <strong>{item.status}</strong>
                              <span>{item.kind}</span>
                              <small>{item.summary}</small>
                              <code>{item.digest.slice(0, 20)}…</code>
                            </div>
                          ))}
                      </div>
                    )}
                  </section>
                )}
              </div>
              <fieldset
                className="learn-action-area"
                disabled={Boolean(busy) || prerequisiteBlocked}
                hidden={
                  Boolean(reviewLab) ||
                  tab === 'engineering' ||
                  tab === 'records' ||
                  (tab === 'text' &&
                    !['reading', 'checking_understanding'].includes(step.session.state)) ||
                  (tab === 'assessment' && step.session.state !== 'reflecting') ||
                  (tab === 'evidence' &&
                    !['practicing', 'collecting_evidence', 'verifying'].includes(
                      step.session.state
                    ))
                }
              >
                {step.session.state === 'reading' && (
                  <button onClick={() => transition('checking_understanding')}>
                    已阅读，进入理解检查
                  </button>
                )}
                {step.session.state === 'checking_understanding' && (
                  <div className="local-course-input">
                    <label>{step.lab.comprehensionCheck}</label>
                    <textarea
                      value={answer}
                      onChange={(event) => setAnswer(event.target.value)}
                      maxLength={4000}
                      placeholder="用自己的话回答"
                    />
                    {!reflectionActivity && step.session.projectId <= 0 && (
                      <p className="learn-notice">
                        请先在“工程与版本”绑定实践工程；回答可以先保存。
                      </p>
                    )}
                    <button
                      disabled={
                        !answer.trim() || (!reflectionActivity && step.session.projectId <= 0)
                      }
                      onClick={() => transition(reflectionActivity ? 'reflecting' : 'practicing')}
                    >
                      {reflectionActivity ? '保存理解回答并复盘' : '提交回答并开始实践'}
                    </button>
                  </div>
                )}
                {step.session.state === 'practicing' && (
                  <button
                    disabled={Boolean(busy)}
                    onClick={executeAction}
                    data-testid="local-course-run-action"
                  >
                    <LynxIcon name="terminal" size={16} />{' '}
                    {busy === 'action' ? '执行中…' : '执行本课确定性校验'}
                  </button>
                )}
                {action && (
                  <div className={`local-course-result ${action.status.toLowerCase()}`}>
                    <strong>{action.status}</strong>
                    <span>{action.summary}</span>
                    <code>{action.outputDigest}</code>
                    {action.source === 'software' && (
                      <button className="learning-secondary" onClick={() => setTab('evidence')}>
                        查看本次执行记录与诊断
                      </button>
                    )}
                  </div>
                )}
                {!reflectionActivity &&
                  ['collecting_evidence', 'verifying', 'reflecting'].includes(
                    step.session.state
                  ) && (
                    <p className="learn-notice">
                      继续前会只读复核课程工程文件。工程变化后请恢复原输入或重新实践；历史记录仍保留。此检查不代表外部工具链或开发板状态已验证。
                    </p>
                  )}
                {step.session.state === 'collecting_evidence' && (
                  <button onClick={() => transition('verifying')}>证据已记录，核对规则</button>
                )}
                {step.session.state === 'verifying' && (
                  <div className="local-course-actions">
                    <button onClick={() => transition('reflecting')}>规则符合，进入反思</button>
                    <button className="learning-secondary" onClick={() => transition('practicing')}>
                      重新实践
                    </button>
                  </div>
                )}
                {!reflectionActivity &&
                  ['collecting_evidence', 'reflecting'].includes(step.session.state) && (
                    <button className="learning-secondary" onClick={() => transition('practicing')}>
                      重新实践
                    </button>
                  )}
                {step.session.state === 'reflecting' && (
                  <div className="local-course-input">
                    <label>{step.lab.reflectionPrompt}</label>
                    <textarea
                      value={reflection}
                      onChange={(event) => setReflection(event.target.value)}
                      maxLength={4000}
                      placeholder="记录结论、差异和下一步"
                    />
                    <button disabled={!reflection.trim()} onClick={() => transition('completed')}>
                      完成本课并进入下一课
                    </button>
                  </div>
                )}
              </fieldset>
              {step.session.state === 'completed' && (
                <button onClick={makeReport} disabled={Boolean(busy)}>
                  生成最终学习报告
                </button>
              )}
              {step.lab.required === false && step.session.state !== 'completed' && (
                <div className="learn-notice" data-testid="optional-activity">
                  <strong>本活动为选修</strong>
                  <p>可以跳过后继续。已有记录保留；跳过不计为完成或验证通过。</p>
                  <button disabled={Boolean(busy) || !journey} onClick={skipOptional}>
                    跳过本选修活动
                  </button>
                </div>
              )}
              <div className="learn-workspace-secondary">
                <button className="learning-text-button" onClick={() => setTab('assessment')}>
                  章节评测
                </button>
                <button className="learning-text-button" onClick={() => setTab('records')}>
                  学习档案
                </button>
              </div>
              <details className="learn-session-meta">
                <summary>会话与保存状态</summary>
                <p className="learn-muted" role="status">
                  {draftStatus} · 当前尝试 {journey?.attempt ?? '—'} · 切换视图不会完成步骤
                </p>
                <button
                  className="learning-secondary"
                  disabled={Boolean(busy)}
                  onClick={() =>
                    void run('leave', async () => {
                      await saveDraft();
                      setStep(null);
                      setJourney(null);
                    })
                  }
                >
                  退出到会话列表
                </button>
              </details>
              {report && (
                <div className="local-course-report">
                  <h2>验收报告</h2>
                  {journey?.labs.some((lab) => lab.activityKind === 'reflection') && (
                    <p>理论活动：自述完成，未评分；不替代软件或实机验证。</p>
                  )}
                  <p>
                    {localReportProgress(report)} · 证据 {report.evidenceCount} 条
                  </p>
                  <p>
                    当前软件：{localReportSoftwareResult(report)} · 当前 HIL：{report.hilResult} ·
                    provenance：
                    {report.provenanceResult}
                  </p>
                  <p className="learn-notice" data-testid="local-report-currency">
                    历史软件：{report.recordedSoftwareResult || report.softwareResult}。{' '}
                    {localReportCurrencyText(report)}
                  </p>
                  <p>
                    HIL 步骤：{report.hilPassedSteps}/{report.hilRequiredSteps}
                    {report.recordedHilResult && report.recordedHilResult !== report.hilResult && (
                      <> · 旧规则记录：{report.recordedHilResult}（不作为实机通过）</>
                    )}
                  </p>
                </div>
              )}
            </section>

            <aside className="local-course-tutor">
              <section className="learn-assistant-card">
                <h2>
                  <LynxIcon name="agent" size={24} />
                  {tab === 'engineering'
                    ? 'AI 调试指导'
                    : tab === 'evidence'
                      ? 'AI 分析与建议'
                      : tab === 'assessment'
                        ? 'AI 本课小结'
                        : 'AI 学习助手'}
                </h2>
                {agentPlan && (
                  <section className="learn-agent-plan" data-testid="learn-agent-plan">
                    <strong>{agentPlan.title}</strong>
                    <p>{agentPlan.guidance}</p>
                    <small>
                      依据：{agentPlan.stepId} · 第 {agentPlan.attempt} 次尝试 · 规则指引
                    </small>
                    <button
                      className="learning-secondary"
                      onClick={() => {
                        setTab(agentPlan.destination);
                        setStageSelection(null);
                      }}
                    >
                      前往当前任务 →
                    </button>
                  </section>
                )}
                <div className="learn-notice">
                  {tab === 'engineering'
                    ? '先核对工程与版本，再进入开发。检查不会覆盖已有修改。'
                    : tab === 'evidence'
                      ? '观察照片与日志应结合课程规则确认，单张照片不代表实验通过。'
                      : tab === 'assessment'
                        ? reflectionActivity
                          ? '整理自己的理解与未解决问题；回答和复盘不自动评分。'
                          : '回顾本课实践结果，解释依据与仍未解决的问题。'
                        : '先分清开发主机与运行目标，再开始操作。'}
                </div>
                <button
                  className="learning-secondary"
                  onClick={() => setQuestion('请解释当前课的关键概念，并指出教材依据。')}
                >
                  <LynxIcon name="brain" size={20} />
                  解释当前概念 →
                </button>
                <button
                  className="learning-secondary"
                  onClick={() => setQuestion('根据当前学习阶段，给我下一步检查提示，不直接代做。')}
                >
                  <LynxIcon name="play" size={20} />
                  给我下一步提示 →
                </button>
                <details className="learn-tutor-compose" open={Boolean(question || tutor)}>
                  <summary>针对本课提问</summary>
                  <select
                    value={hintLevel}
                    onChange={(event) => setHintLevel(event.target.value as typeof hintLevel)}
                  >
                    <option value="L1">L1 · 只给方向</option>
                    <option value="L2">L2 · 给检查点</option>
                    <option value="L3">L3 · 给步骤</option>
                    <option value="L4">L4 · 解释完整思路</option>
                  </select>
                  <textarea
                    value={question}
                    onChange={(event) => setQuestion(event.target.value)}
                    placeholder="针对当前步骤提问，不要粘贴密码或密钥"
                  />
                  <button disabled={!question.trim() || Boolean(busy)} onClick={askTutor}>
                    {busy === 'tutor' ? '思考中…' : '向 AI 助教提问'}
                  </button>
                  {selectedDiagnostic && (
                    <div className="learn-notice" data-testid="local-tutor-diagnostic-selection">
                      <strong>本次提问将附带 1 条已查看的命令诊断</strong>
                      <p>发送到当前默认 AI Provider。仅在点击提问时发送；请先检查内容。</p>
                      <details>
                        <summary>查看将发送的诊断</summary>
                        <p>
                          {selectedDiagnostic.stepId} · 尝试 {selectedDiagnostic.attempt} ·{' '}
                          {selectedDiagnostic.status}
                        </p>
                        <pre className="learn-log-output">{selectedDiagnostic.text}</pre>
                        {selectedDiagnostic.truncated && <p>仅保存了脱敏输出末尾，内容已截断。</p>}
                      </details>
                      <button
                        className="learning-secondary"
                        onClick={() => setSelectedDiagnostic(null)}
                      >
                        不附带诊断
                      </button>
                    </div>
                  )}
                  {tutor && (
                    <div className="local-course-tutor-answer">
                      <SafeMarkdown source={tutor.answer} />
                      <small>上下文：{tutor.contextScope.join('、')} · 执行动作：否</small>
                    </div>
                  )}
                  <small>
                    默认使用本课目标、当前尝试的回答和最近三条证据摘要。命令诊断仅在你选择后附到本次提问；AI
                    不能执行命令或判定实机通过。
                  </small>
                </details>
              </section>
              {reflectionActivity ? (
                <section className="learn-assistant-card">
                  <h2>本活动学习说明</h2>
                  <p>
                    阅读教材，写下自己的理解，再完成复盘。记录只表示已提交，不代表答案正确、软件或实机通过。
                  </p>
                  <button className="learning-secondary" onClick={() => setTab('records')}>
                    查看学习档案
                  </button>
                </section>
              ) : tab === 'assessment' ? (
                <>
                  <section className="learn-assistant-card">
                    <h2>
                      <LynxIcon name="document" size={24} /> 本课问题记录
                    </h2>
                    <div className="learn-assessment-issues">
                      {(journey?.evidence || [])
                        .filter((item) => item.stepId === step.session.currentStep)
                        .slice(-3)
                        .map((item, index) => (
                          <button
                            key={index}
                            className="learning-secondary"
                            onClick={() => setTab('evidence')}
                          >
                            <LynxIcon
                              name={item.status === 'PASS' ? 'check-circle' : 'circle'}
                              size={24}
                            />
                            <span>
                              {item.summary}
                              <small>
                                {item.status} · 第 {item.attempt} 次尝试 · {item.source}
                              </small>
                            </span>
                            <span>›</span>
                          </button>
                        ))}
                      {!journey?.evidence.some(
                        (item) => item.stepId === step.session.currentStep
                      ) && (
                        <p className="learn-notice">
                          尚无本课执行记录。返回实践后可查看失败现象和验证依据。
                        </p>
                      )}
                    </div>
                    <button className="learning-text-button" onClick={() => setTab('engineering')}>
                      定位到工程 →
                    </button>
                  </section>
                  <section className="learn-assistant-card learn-next-chapter">
                    <h2>
                      <LynxIcon name="play" size={24} />{' '}
                      {journey?.activityAccess?.length
                        ? graphNext
                          ? '下一可用课时'
                          : '课程总结'
                        : step.stepIndex + 1 < step.totalSteps
                          ? '下一课'
                          : '课程总结'}
                    </h2>
                    <p>
                      {journey?.activityAccess?.length
                        ? graphNext?.title || '查看课程卡点与总结'
                        : journey?.labs[step.stepIndex + 1]?.title || '整理学习过程与后续建议'}
                    </p>
                    {step.session.state === 'completed' ? (
                      <button onClick={() => setTab('records')}>查看学习总结 →</button>
                    ) : (
                      <button
                        disabled={
                          Boolean(busy) ||
                          prerequisiteBlocked ||
                          step.session.state !== 'reflecting' ||
                          !reflection.trim()
                        }
                        onClick={() => transition('completed')}
                      >
                        提交复盘并继续 →
                      </button>
                    )}
                    <small>完成本课实践和复盘后继续；通过条件由课程规则核对。</small>
                  </section>
                </>
              ) : (
                <>
                  <section className="learn-assistant-card">
                    <h2>
                      <LynxIcon name="agent" size={24} />
                      当前环境状态
                    </h2>
                    <button
                      className="learn-environment-row learning-secondary"
                      onClick={() => setTab('engineering')}
                    >
                      <i className="learn-status-dot" />
                      Ubuntu 工程 · {boundProject ? '已配置' : '未配置'} ›
                    </button>
                    <button
                      className="learn-environment-row learning-secondary"
                      onClick={() => setTab('evidence')}
                    >
                      <i className="learn-status-dot" />
                      开发板 · {step.session.boardUid ? '已绑定' : '未绑定'} ›
                    </button>
                    <p className="learn-notice">
                      可以先阅读，执行前再核对连接。已配置不代表当前在线。
                    </p>
                  </section>
                  <section className="learn-assistant-card">
                    <h2>
                      <LynxIcon name="folder" size={24} />
                      关联工程
                    </h2>
                    <strong>{boundProject?.name || '工程待配置'}</strong>
                    <code>{boundProject?.sdkDir || '请在工作台绑定工程'}</code>
                    <button className="learning-secondary" onClick={() => setTab('engineering')}>
                      定位到工程 →
                    </button>
                  </section>
                  <section className="learn-assistant-card">
                    <h2>
                      <LynxIcon name="brain" size={24} />
                      学习提示
                    </h2>
                    <ul>
                      <li>当前阶段：{stateLabels[step.session.state]}</li>
                      <li>按预期现象检查，失败后保留日志与本次尝试。</li>
                      <li>AI 提示用于辅助理解，不能替代实际验证。</li>
                    </ul>
                  </section>
                </>
              )}
            </aside>
          </div>
        </div>
      )}
      <LearnFooter>{draftStatus || '固定课程版本 · 学习记录保存在本地'}</LearnFooter>
    </div>
  );
};
