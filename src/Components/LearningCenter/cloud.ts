import { invokeCommand } from '../../Platform/IPC/Client';
import type { CloudLearningCourse, CloudLearningProgress } from '../../Platform/IPC/Commands';
import type { LearningCourse, LearningProgress } from './types';

export async function readCloudLearningCatalog(
  enrolled: { bundleKey: string; version: string }[]
): Promise<{
  courses: LearningCourse[];
  progress: LearningProgress[];
  progressWarning: string;
}> {
  if (!enrolled.length) return { courses: [], progress: [], progressWarning: '' };
  const [coursesResult, progressResult] = await Promise.allSettled([
    Promise.all(
      enrolled.map(async (entry) => {
        const course = await invokeCommand('learning_get_course', { identifier: entry.bundleKey });
        if (course.slug !== entry.bundleKey || course.version !== entry.version)
          throw new Error('已报名课程详情与固定版本不一致，请从原学习路径继续。');
        return course;
      })
    ),
    invokeCommand('learning_list_progress'),
  ]);
  if (coursesResult.status === 'rejected') throw coursesResult.reason;
  const courses = coursesResult.value;
  const progress = progressResult.status === 'fulfilled' ? progressResult.value : [];
  const mappedProgress = progress
    .filter((item) =>
      courses.some(
        (course) =>
          (item.resourceType === 'course' && item.resourceId === String(course.id)) ||
          (item.resourceType === 'lab' &&
            course.labs.some((lab) => item.resourceId === String(lab.id)))
      )
    )
    .map(mapCloudLearningProgress);
  return {
    courses: courses.map((course) => mapCloudLearningCourse(course, mappedProgress)),
    progress: mappedProgress,
    progressWarning: progressResult.status === 'rejected' ? '课程已同步，学习进度暂不可用。' : '',
  };
}

function mapCloudLearningProgress(progress: CloudLearningProgress): LearningProgress {
  return {
    resourceType: progress.resourceType,
    resourceId: progress.resourceId,
    state: progress.state,
    percent: progress.percent,
    sessionId: progress.sessionId,
    revision: progress.revision,
    updatedAt: progress.updateTime,
  };
}

function mapCloudLearningCourse(
  course: CloudLearningCourse,
  progress: LearningProgress[]
): LearningCourse {
  const courseProgress = progress.find(
    (item) => item.resourceType === 'course' && item.resourceId === String(course.id)
  );
  const labProgress = progress.filter(
    (item) =>
      item.resourceType === 'lab' && course.labs.some((lab) => String(lab.id) === item.resourceId)
  );
  const derivedProgress =
    labProgress.length > 0
      ? Math.round(labProgress.reduce((sum, item) => sum + item.percent, 0) / course.labs.length)
      : 0;

  return {
    id: String(course.id),
    slug: course.slug,
    title: course.title,
    version: course.version,
    summary: course.summary || '该课程暂未提供简介。',
    cover: course.cover,
    progress: courseProgress?.percent ?? derivedProgress,
    minLynxVersion: course.labs.find((lab) => lab.minLynxVersion)?.minLynxVersion || '0.6.0',
    source: 'cloud',
    track: course.track,
    level: course.level,
    entitlementRequired: course.entitlementRequired,
    hasEntitlement: course.hasEntitlement,
    lynxReady: course.lynxReady,
    chapters: course.chapters.map((chapter) => ({
      id: String(chapter.id),
      key: chapter.chapterKey,
      title: chapter.title,
      kind:
        chapter.contentType === 'video'
          ? 'video'
          : chapter.contentType === 'lab'
            ? 'lab'
            : 'document',
      sourceUrl: chapter.sourceUrl,
    })),
    labs: course.labs.map((lab) => ({
      id: String(lab.id),
      key: lab.labKey,
      title: lab.title,
      summary: lab.summary,
      durationMinutes: lab.durationMinutes,
      riskLevel: normalizeRisk(lab.riskLevel),
      packageState: lab.packageReady ? 'ready' : 'not-downloaded',
      packageVersion: lab.currentPackageVersion,
      entitlementRequired: lab.entitlementRequired,
      hasEntitlement: lab.hasEntitlement,
      boardProfiles: lab.boardProfiles.map((profile) => ({
        profileId: profile.profileId,
        displayName: profile.displayName,
        vendor: profile.vendor,
        soc: profile.soc,
      })),
      sdkRequirement: [lab.sdkType, lab.sdkVersions].filter(Boolean).join(' ') || '按实验包要求',
      minLynxVersion: lab.minLynxVersion || '0.6.0',
    })),
  };
}

function normalizeRisk(value: string): 'L0' | 'L1' | 'L2' | 'L3' | 'L4' {
  return ['L0', 'L1', 'L2', 'L3', 'L4'].includes(value)
    ? (value as 'L0' | 'L1' | 'L2' | 'L3' | 'L4')
    : 'L0';
}
