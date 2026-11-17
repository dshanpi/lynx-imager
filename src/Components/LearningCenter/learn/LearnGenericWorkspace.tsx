import { useEffect, useId, useRef, useState, type ReactNode } from 'react';
import { invokeCommand } from '../../../Platform/IPC/Client';
import type {
  LearnVerifiedCourse,
  LearnUnitContent,
  LearnVerifiedPreview,
} from '../../../Platform/IPC/Commands';
import type { LearningCourse } from '../types';
import { LearnBadge, LearnBreadcrumb, LearnFooter } from './LearnChrome';
import { SafeMarkdown } from './LearnMarkdown';
import { LearnDocument } from './LearnDocument';
import { learnCoursePin } from './genericModel';
import { LearnGenericReview } from './LearnGenericReview';
import { LearnGenericEnvironment } from './LearnGenericEnvironment';
import { LearnGenericActions } from './LearnGenericActions';
import { LearnPracticeSync } from './LearnPracticeSync';
import { LearnGenericBoard } from './LearnGenericBoard';
import { LearnGenericProgress } from './LearnGenericProgress';
import { LearnGenericTutor } from './LearnGenericTutor';
import { learnServiceError } from './serviceStatus';
import {
  reflectionStorageKey,
  readReflectionDrafts,
  writeReflectionDrafts,
  type ReflectionDrafts,
} from './reflectionStorage';
import './LearnHub.css';
import './LearnGenericWorkspace.css';

export function LearnGenericWorkspace({
  active,
  accountId,
  course,
  onBack,
  preview,
  onRefresh,
  children,
  initialChapterId,
  backLabel = '返回我的课程',
}: {
  active: boolean;
  accountId?: number;
  initialChapterId?: string;
  backLabel?: string;
  course?: LearningCourse;
  preview?: LearnVerifiedPreview;
  onRefresh?: () => void;
  children?: ReactNode;
  onBack: () => void;
}) {
  const [verified, setVerified] = useState<LearnVerifiedCourse | null>(null);
  const [content, setContent] = useState<LearnUnitContent | null>(null);
  const [unitId, setUnitId] = useState('');
  const [activityId, setActivityId] = useState('');
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [reading, setReading] = useState(false);
  const [reload, setReload] = useState(0);
  const [guidanceOpen, setGuidanceOpen] = useState(true);
  const guidanceId = useId();
  // UI pin is checked again against the authenticated cloud enrollment in Rust.
  const pin = course ? learnCoursePin(course) : null;
  const enrollmentId = pin?.enrollmentId;
  const releaseId = pin?.releaseId;
  const manifestSha256 = pin?.manifestSha256;
  // Restored only after Rust verifies the enrollment, with an explicit account boundary.
  const reflectionScope = JSON.stringify([accountId, enrollmentId, releaseId, manifestSha256]);
  const [reflectionDrafts, setReflectionDrafts] = useState<{
    scope: string;
    units: ReflectionDrafts;
  }>({ scope: reflectionScope, units: {} });
  const reflectionCache = useRef(reflectionDrafts);
  const restoredReflection = useRef('');
  const [reflectionReadFailed, setReflectionReadFailed] = useState(false);
  const [reflectionStorageError, setReflectionStorageError] = useState('');
  const reflectionVerified = Boolean(
    verified &&
    verified.enrollmentId === enrollmentId &&
    verified.manifest.releaseId === releaseId &&
    verified.manifestSha256 === manifestSha256
  );
  function persistReflectionDrafts(units: ReflectionDrafts) {
    if (!accountId || !pin || !reflectionVerified || preview || reflectionReadFailed) return;
    try {
      writeReflectionDrafts(localStorage, reflectionStorageKey(accountId, pin), units);
      setReflectionStorageError('');
    } catch {
      setReflectionStorageError('本机草稿保存失败，当前输入仍保留。请重试，或保存复盘后再离开。');
    }
  }
  function updateReflectionDraft(activityId: string, text: string | null) {
    const previous = reflectionCache.current;
    const units = previous.scope === reflectionScope ? previous.units : {};
    const drafts = { ...units[unitId] };
    if (text === null) delete drafts[activityId];
    else drafts[activityId] = text;
    const next = { scope: reflectionScope, units: { ...units, [unitId]: drafts } };
    reflectionCache.current = next;
    setReflectionDrafts(next);
    persistReflectionDrafts(next.units);
  }
  useEffect(() => {
    if (
      !active ||
      !reflectionVerified ||
      preview ||
      !accountId ||
      !enrollmentId ||
      !releaseId ||
      !manifestSha256 ||
      restoredReflection.current === reflectionScope
    )
      return;
    restoredReflection.current = reflectionScope;
    setReflectionReadFailed(false);
    try {
      const units = readReflectionDrafts(
        localStorage,
        reflectionStorageKey(accountId, {
          enrollmentId,
          releaseId,
          manifestSha256,
        })
      );
      const next = { scope: reflectionScope, units };
      reflectionCache.current = next;
      setReflectionDrafts(next);
      setReflectionStorageError('');
    } catch {
      setReflectionReadFailed(true);
      setReflectionStorageError(
        '本机草稿读取失败，原始数据未被覆盖。请保留当前输入，修复本机存储后重新进入。'
      );
    }
  }, [
    active,
    reflectionVerified,
    preview,
    accountId,
    enrollmentId,
    releaseId,
    manifestSha256,
    reflectionScope,
  ]);
  useEffect(() => {
    let cancelled = false;
    setVerified(null);
    setContent(null);
    setUnitId('');
    setActivityId('');
    setError('');
    setBusy(false);
    if (active && preview) {
      setVerified(preview.course);
      setUnitId(
        preview.currentUnitId && preview.binding.allowedUnitIds.includes(preview.currentUnitId)
          ? preview.currentUnitId
          : preview.binding.allowedUnitIds[0] || ''
      );
      return;
    }
    if (!active || !enrollmentId || !releaseId || !manifestSha256) return;
    setBusy(true);
    void invokeCommand('learn_get_manifest', { enrollmentId, releaseId, manifestSha256 })
      .then((result) => {
        if (cancelled) return;
        setVerified(result);
        const targetChapter = initialChapterId
          ? result.manifest.chapters.find((c) => c.chapterId === initialChapterId)
          : undefined;
        if (initialChapterId && !targetChapter) {
          setError('官网指定章节不在当前学习版本中，请核对课程版本或从左侧目录选择。');
          return;
        }
        const units = targetChapter?.units || result.manifest.chapters.flatMap((c) => c.units);
        setUnitId(
          units.find((u) => u.unitId === course?.currentUnitId)?.unitId || units[0]?.unitId || ''
        );
      })
      .catch((e) => {
        if (!cancelled) setError(learnServiceError(e));
      })
      .finally(() => {
        if (!cancelled) setBusy(false);
      });
    return () => {
      cancelled = true;
    };
  }, [
    active,
    enrollmentId,
    releaseId,
    manifestSha256,
    course?.currentUnitId,
    reload,
    preview,
    initialChapterId,
  ]);
  useEffect(() => {
    let cancelled = false;
    setContent(null);
    setActivityId('');
    setReading(false);
    if (!active || !verified || !unitId) return;
    if (!preview && (!enrollmentId || !releaseId || !manifestSha256)) return;
    setReading(true);
    setError('');
    const request = preview
      ? invokeCommand('learn_get_preview_unit', {
          previewId: preview.binding.previewId,
          contentDigest: preview.binding.contentDigest,
          unitId,
        })
      : invokeCommand('learn_get_unit', {
          enrollmentId: enrollmentId!,
          releaseId: releaseId!,
          manifestSha256: manifestSha256!,
          unitId,
        });
    void request
      .then((result) => {
        if (!cancelled) setContent(result);
      })
      .catch((e) => {
        if (!cancelled) setError(learnServiceError(e));
      })
      .finally(() => {
        if (!cancelled) setReading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [active, verified, unitId, enrollmentId, releaseId, manifestSha256, preview]);

  const manifest = verified?.manifest;
  const chapter = manifest?.chapters.find((c) => c.units.some((u) => u.unitId === unitId));
  const unit = chapter?.units.find((u) => u.unitId === unitId);
  const activity = unit?.activities.find((a) => a.activityId === activityId) || unit?.activities[0];
  const environment = manifest?.environments.find((e) => e.profileId === activity?.environmentRef);
  const resources =
    manifest?.resources.filter((r) => activity?.resourceRefs.includes(r.resourceId)) || [];
  const documents =
    content?.unitId === unitId
      ? content.documents.filter((d) => activity?.resourceRefs.includes(d.resourceId))
      : [];
  const rules = manifest?.rules.filter((r) => activity?.ruleRefs.includes(r.ruleId)) || [];
  const activityIndex =
    unit?.activities.findIndex((a) => a.activityId === activity?.activityId) ?? -1;
  const readingPanel = useRef<HTMLElement>(null);
  useEffect(() => {
    if (readingPanel.current) readingPanel.current.scrollTop = 0;
  }, [unitId, activity?.activityId]);
  return (
    <div
      className="learn-journey learn-v2-page learn-cloud-workspace learn-generic-workspace"
      data-testid="learn-generic-workspace"
    >
      <LearnBreadcrumb items={['学习中心', manifest?.title || course?.title || '官网课程试学']}>
        <button className="learning-secondary" onClick={onBack}>
          {backLabel}
        </button>
      </LearnBreadcrumb>
      {children}
      <div className="learn-v2-body">
        <div className="learn-workspace-topbar">
          <strong>课程目录</strong>
          <LearnBadge>
            {preview
              ? `草稿 r${preview.binding.draftRevision} · 仅预览`
              : `v${manifest?.version || course?.version}`}
          </LearnBadge>
          {manifest && (
            <span>
              {!environment || environment.target === 'none'
                ? '本步无需开发环境'
                : environment?.target === 'board'
                  ? '开发板 · 待核验'
                  : 'Ubuntu · 待核验'}
            </span>
          )}
          <button
            className="learning-text-button"
            disabled={busy || reading}
            onClick={() => (onRefresh ? onRefresh() : setReload((n) => n + 1))}
          >
            重新同步
          </button>
          <button
            type="button"
            className="learning-secondary"
            aria-expanded={guidanceOpen}
            aria-controls={guidanceId}
            onClick={() => setGuidanceOpen(!guidanceOpen)}
          >
            {guidanceOpen ? '收起指引' : '展开指引'}
          </button>
        </div>
        {error && (
          <p className="learn-notice" role="alert">
            {error}
          </p>
        )}
        {!manifest ? (
          <section className="learn-panel learn-hub-section">
            <h2>{busy ? '正在核验课程版本与签名…' : '课程尚未打开'}</h2>
            <p>课程目录和教材须通过官方签名与当前账号权益核验。</p>
          </section>
        ) : (
          <>
            <nav className="learn-course-chapters" aria-label="课程目录">
              {manifest.chapters.flatMap((c) =>
                c.units.map((u) => (
                  <button
                    key={u.unitId}
                    className={u.unitId === unitId ? 'active' : ''}
                    aria-current={u.unitId === unitId ? 'page' : undefined}
                    onClick={() => {
                      if (u.unitId !== unitId) {
                        setContent(null);
                        setUnitId(u.unitId);
                      }
                    }}
                  >
                    {c.title} · {u.title}
                  </button>
                ))
              )}
            </nav>
            <div className={`learn-cloud-grid${guidanceOpen ? '' : ' learn-guidance-collapsed'}`}>
              <nav className="learn-panel learn-cloud-steps" aria-label="实验学习步骤">
                <h3>本课学习步骤</h3>
                {unit?.activities.map((a, i) => (
                  <button
                    key={a.activityId}
                    className={
                      a.activityId === activity?.activityId ? 'active' : 'learning-secondary'
                    }
                    aria-current={a.activityId === activity?.activityId ? 'step' : undefined}
                    onClick={() => setActivityId(a.activityId)}
                  >
                    <span>{i + 1}</span>
                    {a.title}
                  </button>
                ))}
                <p>切换步骤不会标记学习完成。</p>
              </nav>
              <article
                className="learn-panel learn-cloud-content"
                ref={readingPanel}
                tabIndex={0}
                aria-label="教材与实践内容"
              >
                <h1>{unit?.title}</h1>
                <p>{unit?.objective}</p>
                <h2>{activity?.title}</h2>
                <SafeMarkdown source={activity?.instruction || ''} />
                {reading ? (
                  <p role="status">正在核验教材快照…</p>
                ) : !content ? (
                  <p>本节教材尚未载入，请重新同步。</p>
                ) : (
                  documents.map((d) => (
                    <LearnDocument
                      key={`${unitId}:${d.resourceId}:${d.sha256}`}
                      source={d.markdown}
                      sourceUrl={resources.find((r) => r.resourceId === d.resourceId)?.sourceUrl}
                    />
                  ))
                )}
                {resources
                  .filter((r) => r.kind !== 'document')
                  .map((r) => (
                    <section key={r.resourceId}>
                      <h3>{r.title}</h3>
                      <dl>
                        <dt>来源</dt>
                        <dd>{r.sourceUrl}</dd>
                        {r.commit && (
                          <>
                            <dt>固定源码提交</dt>
                            <dd>{r.commit}</dd>
                          </>
                        )}
                        {r.sha256 && (
                          <>
                            <dt>制品 SHA-256</dt>
                            <dd>{r.sha256}</dd>
                          </>
                        )}
                      </dl>
                    </section>
                  ))}
                {!preview && pin && environment && environment.target !== 'none' && (
                  <LearnGenericEnvironment
                    key={`${pin.enrollmentId}:${pin.manifestSha256}:${environment.profileId}`}
                    active={active}
                    pin={pin}
                    profileId={environment.profileId}
                  />
                )}
                {environment && (
                  <section>
                    <h3>环境要求</h3>
                    <p>
                      {environment.target === 'none'
                        ? '本步骤无需绑定 Ubuntu、工程或开发板。'
                        : '以下为课程要求，实际环境尚未核验。'}
                    </p>
                    <ul>
                      {environment.requiredCapabilities.map((c) => (
                        <li key={c}>{c}</li>
                      ))}
                      {environment.versionChecks.map((v, i) => (
                        <li key={i}>
                          {v.component}：{v.expected}
                        </li>
                      ))}
                    </ul>
                  </section>
                )}
                {activity?.actionRefs.map((actionId) => {
                  const action = manifest.actions.find((a) => a.actionId === actionId);
                  if (!preview && pin && unit && action?.capabilityId === 'learn.board.observe') {
                    return (
                      <LearnGenericBoard
                        key={`${accountId}:${pin.enrollmentId}:${pin.manifestSha256}:${activity.activityId}:${actionId}`}
                        active={active}
                        request={{
                          ...pin,
                          profileId: activity.environmentRef,
                          unitId: unit.unitId,
                          activityId: activity.activityId,
                          actionId,
                        }}
                      />
                    );
                  }
                  return !preview &&
                    pin &&
                    unit &&
                    action?.capabilityId === 'learn.project.task' ? (
                    <LearnGenericActions
                      key={`${accountId}:${pin.enrollmentId}:${pin.manifestSha256}:${activity.activityId}:${actionId}`}
                      accountId={accountId}
                      active={active}
                      request={{
                        ...pin,
                        profileId: activity.environmentRef,
                        unitId: unit.unitId,
                        activityId: activity.activityId,
                        actionId,
                      }}
                    />
                  ) : (
                    <p key={actionId} className="learn-notice">
                      {preview
                        ? '预览模式不能执行工程任务。'
                        : `当前动作 ${actionId} 所需的执行适配器尚未开放。`}
                    </p>
                  );
                })}
                {!preview && pin && unit && verified && activity && (
                  <LearnPracticeSync
                    key={`${accountId}:${pin.enrollmentId}:${pin.manifestSha256}:${unit.unitId}:${activity.environmentRef}`}
                    active={active}
                    accountId={accountId ?? 0}
                    pin={pin}
                    unit={unit}
                    profileId={activity.environmentRef}
                    actions={verified.manifest.actions}
                  />
                )}
                {!preview && pin && unit && chapter && content && (
                  <LearnGenericReview
                    key={`${accountId}:${pin.enrollmentId}:${pin.manifestSha256}:${unit.unitId}`}
                    accountId={accountId ?? 0}
                    active={active}
                    pin={pin}
                    chapterId={chapter.chapterId}
                    unit={unit}
                  />
                )}
                <div className="learn-hub-actions">
                  <button
                    className="learning-secondary"
                    disabled={!unit || activityIndex >= unit.activities.length - 1}
                    onClick={() => {
                      if (unit) setActivityId(unit.activities[activityIndex + 1].activityId);
                    }}
                  >
                    查看下一步
                  </button>
                </div>
              </article>
              <aside
                className="learn-panel learn-cloud-agent"
                id={guidanceId}
                hidden={!guidanceOpen}
              >
                <h2>本步指引</h2>
                {!preview && pin && unit && verified && (
                  <LearnGenericTutor
                    key={`${accountId}:${pin.enrollmentId}:${unit.unitId}:${activity?.activityId}:${verified.manifestSha256}`}
                    active={active}
                    pin={pin}
                    unitId={unit.unitId}
                    policy={verified.agentPolicy}
                    engineeringActivityId={
                      activity &&
                      verified.manifest.environments.some(
                        (environment) =>
                          environment.profileId === activity.environmentRef &&
                          environment.target === 'ubuntu'
                      ) &&
                      verified.manifest.resources.filter(
                        (resource) =>
                          resource.kind === 'source' &&
                          Boolean(resource.commit) &&
                          activity.resourceRefs.includes(resource.resourceId) &&
                          verified.agentPolicy.contextResourceRefs.includes(resource.resourceId)
                      ).length === 1
                        ? activity.activityId
                        : undefined
                    }
                  />
                )}
                <h3>预期现象</h3>
                <p>{activity?.expectedObservation}</p>
                <h3>遇到问题</h3>
                <p>{activity?.recoveryHint}</p>
                <h3>需要哪些证据</h3>
                {rules.length ? (
                  rules.map((r) => (
                    <p key={r.ruleId}>{r.requiredEvidenceKinds.join('、')} · 尚未验证</p>
                  ))
                ) : (
                  <p>请先理解教材与本步目标。</p>
                )}
                {!preview && pin && unit && chapter && content && (
                  <LearnGenericProgress
                    key={`${accountId}:${pin.enrollmentId}:${pin.manifestSha256}:${unit.unitId}`}
                    accountId={accountId ?? 0}
                    active={active}
                    pin={pin}
                    unitId={unit.unitId}
                    chapterId={chapter.chapterId}
                    drafts={
                      reflectionDrafts.scope === reflectionScope
                        ? reflectionDrafts.units[unit.unitId] || {}
                        : {}
                    }
                    locallySaved={Boolean(accountId && !reflectionStorageError)}
                    storageError={reflectionStorageError}
                    onRetryStorage={
                      reflectionReadFailed
                        ? undefined
                        : () => persistReflectionDrafts(reflectionCache.current.units)
                    }
                    onDraftChange={updateReflectionDraft}
                  />
                )}
                <h3>章节复盘</h3>
                <p>{chapter?.summaryPrompt}</p>
                <small>已核验课程签名 · 阅读与复盘不代表实验通过</small>
              </aside>
            </div>
          </>
        )}
      </div>
      <LearnFooter />
    </div>
  );
}
