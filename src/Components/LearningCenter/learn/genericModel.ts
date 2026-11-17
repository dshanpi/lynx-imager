import type { LearnV2Enrollment, LearnCoursePin } from '../../../Platform/IPC/Commands';
import type { LearningCourse } from '../types';

export function learnGenericCatalog(enrolled: LearnV2Enrollment[]): LearningCourse[] {
  return enrolled.map((e) => ({
    id: `v2:${e.enrollmentId}`,
    courseId: e.courseId,
    slug: e.courseSlug || e.courseId,
    title: e.title,
    version: e.version,
    summary: e.summary,
    progress: 0,
    minLynxVersion: '',
    source: 'cloud',
    cloudProtocol: 'v2',
    enrollmentId: e.enrollmentId,
    releaseId: e.releaseId,
    manifestSha256: e.manifestSha256,
    currentUnitId: e.currentUnitId || undefined,
    track: e.track,
    entitlementRequired: true,
    hasEntitlement: true,
    chapters: [],
    labs: [],
  }));
}
export function learnCoursePin(course: LearningCourse): LearnCoursePin | null {
  if (
    course.cloudProtocol !== 'v2' ||
    !course.enrollmentId ||
    !course.releaseId ||
    !course.manifestSha256
  )
    return null;
  return {
    enrollmentId: course.enrollmentId,
    releaseId: course.releaseId,
    manifestSha256: course.manifestSha256,
  };
}
