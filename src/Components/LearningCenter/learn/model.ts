import type {
  InstalledLocalCourse,
  LearnJourney,
  LocalLearningSession,
  CloudLearningEnrollment,
  LocalLearningReport,
} from '../../../Platform/IPC/Commands';
import type { LearningCourse } from '../types';

export function learnRedeemNeedsSync(error: unknown): boolean {
  const value =
    typeof error === 'object' && error !== null
      ? (error as { machineCode?: string; message?: string })
      : null;
  const text = value?.machineCode || value?.message || (typeof error === 'string' ? error : '');
  return /HARDWARE_API_(UNAVAILABLE|FAILED|INVALID_RESPONSE|SCHEMA_MISMATCH)|ORDER_ALREADY_REDEEMED/.test(
    text
  );
}

export function learnAccess(course: LearningCourse) {
  if (course.source === 'local') return { label: '本地已安装', available: true };
  if (course.source !== 'cloud') return { label: '协议样板', available: false };
  if (course.entitlementRequired === undefined) return { label: '权益待确认', available: false };
  if (!course.entitlementRequired) return { label: '免费课程', available: true };
  return course.hasEntitlement
    ? { label: '已拥有', available: true }
    : { label: '未解锁', available: false };
}

export function filterLearnCourses(
  courses: LearningCourse[],
  query: string,
  access: string,
  track: string
) {
  const words = query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  return courses.filter((course) => {
    const text = [
      course.title,
      course.summary,
      ...course.chapters.map((item) => item.title),
      ...course.labs.map((item) => item.title),
    ]
      .join(' ')
      .toLocaleLowerCase();
    return (
      words.every((word) => text.includes(word)) &&
      (!track || course.track === track) &&
      (access !== 'owned' ||
        course.source === 'local' ||
        (course.source === 'cloud' && course.hasEntitlement)) &&
      (access !== 'free' || (course.source === 'cloud' && course.entitlementRequired === false))
    );
  });
}

/** Match the report's maximum attempt number without changing historical order. */
export function learnLatestAttempt(
  attempts: LearnJourney['attempts'],
  stepId: string
): LearnJourney['attempts'][number] | undefined {
  let latest: LearnJourney['attempts'][number] | undefined;
  for (const attempt of attempts) {
    if (attempt.stepId === stepId && (!latest || attempt.attempt > latest.attempt)) {
      latest = attempt;
    }
  }
  return latest;
}

export function learnReportTitle(report: {
  completedSteps: number;
  totalSteps: number;
  hilResult: string;
  softwareResult?: string;
  requiredSteps?: number;
  requiredCompletedSteps?: number;
}) {
  if (
    (report.requiredCompletedSteps ?? report.completedSteps) <
    (report.requiredSteps ?? report.totalSteps)
  )
    return '阶段学习档案';
  if (['REVALIDATION_REQUIRED', 'CURRENT_UNVERIFIED'].includes(report.softwareResult || ''))
    return '课程总结 · 工程待复核';
  return ['PASS', 'NOT_REQUIRED'].includes(report.hilResult)
    ? '课程综合总结'
    : '课程总结 · 实机待验证';
}

export function localReportCurrencyText(report: LocalLearningReport): string {
  const status = report.currentWorkspace?.status;
  const environments = report.currentWorkspace?.environmentChecks || [];
  const hasGraph = !!report.prerequisiteChecks?.length;
  const text =
    status === 'NOT_REQUIRED'
      ? report.optionalSteps
        ? '课程没有必修实践，无需工程核验；选修记录保留，跳过不计通过。'
        : '理论活动无需工程核验；自述完成仍不自动评分。'
      : status === 'MATCH'
        ? report.optionalSteps
          ? '本次只读核对：课程工程文件和已保存绑定与最后一次必修软件执行一致。'
          : '本次只读核对：课程工程文件和已保存绑定与最后一次软件执行一致。'
        : status === 'CHANGED'
          ? '课程工程、绑定或已声明环境已变化，当前软件结果需要重新验证。历史通过与学习记录仍保留；可恢复原输入后重新生成报告，或开始新的学习会话。'
          : status === 'UNRECORDED'
            ? '历史记录没有可核对的工程或所需环境快照，当前软件结果未核验。请重新实践并生成报告。'
            : status === 'UNAVAILABLE'
              ? '本次无法核验当前工程，历史通过不能作为当前通过。请检查连接、工程占用和读取条件，再重新生成报告。'
              : '当前工程尚未核验；以下历史记录不证明当前工程仍适用。';
  const time = report.currentWorkspace?.checkedAt;
  return (
    (hasGraph
      ? '汇总包含必修活动及其声明的前置实践；无关选修不阻断必修结果。 '
      : report.optionalSteps
        ? '软件与 HIL 汇总仅计算必修活动；选修完成、跳过和失败分别保留。 '
        : '') +
    text +
    (report.currentWorkspace?.sourceChecks?.length
      ? ` ${hasGraph ? '逐项核对' : '重学后逐项核对'} ${report.currentWorkspace.sourceChecks.length} 个${hasGraph ? '必修及其前置' : '必修'}实践的声明源码与保存绑定；旧记录缺少源码快照时需重新实践。`
      : '') +
    (environments.length
      ? ` 本次另列出 ${environments.length} 个必修实践活动的环境核对结果。`
      : '') +
    (time ? ` 核对时间：${new Date(time * 1000).toLocaleString()}。` : '') +
    (status === 'NOT_REQUIRED'
      ? ''
      : environments.length
        ? ' 环境核对仅覆盖课程声明并在执行时记录的项目，不证明未声明依赖、全部 SDK 或板上镜像。'
        : ' 核对范围仅含课程工程文件和保存的绑定，不证明外部工具链、SDK 依赖或板上镜像。')
  );
}

export const localEnvironmentStatus = {
  MATCH: '与执行时一致',
  CHANGED: '已变化，需重新验证',
  UNAVAILABLE: '当前无法核验',
  UNRECORDED: '旧执行未记录环境',
};

export const localSourceStatus = { ...localEnvironmentStatus, UNRECORDED: '旧执行未记录源码身份' };

export const localPrerequisiteStatus = {
  completed: '已完成（记录）',
  not_started: '尚未开始',
  incomplete: '最新尝试未完成',
  skipped: '已跳过，未满足前置',
  verification_required: '缺少有效验证证据',
  prerequisite_blocked: '该课的前置尚未满足',
};

export function localReportProgress(report: LocalLearningReport): string {
  if (!report.optionalSteps) return `已完成 ${report.completedSteps} / ${report.totalSteps} 课时`;
  return `必修完成 ${report.requiredCompletedSteps ?? 0} / ${report.requiredSteps ?? 0} · 选修完成 ${report.optionalCompletedSteps ?? 0} / ${report.optionalSteps} · 选修跳过 ${report.optionalSkippedSteps ?? 0}`;
}

export function localReportSoftwareResult(report: LocalLearningReport): string {
  if (report.softwareResult !== 'PASS' || report.currentWorkspace?.status === 'MATCH')
    return report.softwareResult;
  return report.currentWorkspace?.status === 'CHANGED'
    ? 'REVALIDATION_REQUIRED'
    : 'CURRENT_UNVERIFIED';
}

export function learnLocalCatalog(
  courses: InstalledLocalCourse[],
  sessions: LocalLearningSession[]
): LearningCourse[] {
  return courses.map((course) => {
    const resume = sessions
      .filter((s) => s.courseId === course.courseId && s.courseVersion === course.version)
      .sort((a, b) => b.updatedAt - a.updatedAt)[0];
    const chapters = course.chapters || [];
    const position = resume ? chapters.findIndex((c) => c.stepId === resume.currentStep) : -1;
    return {
      id: `local:${encodeURIComponent(course.courseId)}@${encodeURIComponent(course.version)}`,
      localCourseId: course.courseId,
      resumeSessionId: resume?.sessionId,
      slug: course.courseId,
      title: course.title,
      version: course.version,
      summary: `${course.labCount} 个学习章节 · ${course.boardProfile}`,
      source: 'local',
      minLynxVersion: '',
      progress:
        resume?.state === 'completed'
          ? 100
          : position > 0
            ? Math.floor((position / course.labCount) * 100)
            : 0,
      chapters: chapters.map((c) => ({ id: c.stepId, key: c.stepId, title: c.title, kind: 'lab' })),
      labs: [],
    };
  });
}

function learnUnitComplete(unit: CloudLearningEnrollment['units'][number]): boolean {
  try {
    const policy: unknown = JSON.parse(unit.completionPolicy);
    const required: unknown = Array.isArray(policy)
      ? policy
      : policy && typeof policy === 'object' && 'required' in policy
        ? policy.required
        : null;
    return (
      Array.isArray(required) &&
      required.length > 0 &&
      required.every(
        (step) =>
          typeof step === 'string' &&
          ['document', 'video', 'workspace', 'lab'].includes(step) &&
          unit.stepProgress.some(
            (progress) => progress.step === step && progress.state === 'completed'
          )
      )
    );
  } catch {
    return false;
  }
}

export function learnEnrollmentCatalog(enrollments: CloudLearningEnrollment[]): LearningCourse[] {
  return enrollments.map((e) => ({
    id: `enrollment:${e.enrollmentId}`,
    enrollmentId: e.enrollmentId,
    slug: e.bundleKey,
    title: e.title,
    version: e.version,
    summary: e.summary,
    source: 'cloud',
    track: e.track,
    minLynxVersion: e.minLynxVersion,
    entitlementRequired: true,
    hasEntitlement: true,
    // Only server-confirmed unit completion contributes, never the selected tab.
    progress: e.units.length
      ? Math.floor((e.units.filter(learnUnitComplete).length / e.units.length) * 100)
      : 0,
    chapters: e.units.map((u) => ({
      id: u.unitKey,
      key: u.unitKey,
      title: u.title,
      kind: 'document',
    })),
    labs: [],
  }));
}
