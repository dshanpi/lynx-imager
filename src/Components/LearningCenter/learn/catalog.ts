import type { LearnPublishedCourse } from '../../../Platform/IPC/Commands';
import type { LearningCourse } from '../types';

export function publishedLearnCatalog(courses: LearnPublishedCourse[]): LearningCourse[] {
  return courses.map((c) => ({
    id: `published:${c.courseId}`,
    courseId: c.courseId,
    slug: c.courseSlug,
    releaseId: c.releaseId,
    title: c.title,
    version: c.version,
    summary: '嵌入式 AI 辅助学习整课 · 领取课程后按固定版本学习',
    track: c.track,
    source: 'cloud',
    cloudProtocol: 'v2',
    catalogOnly: true,
    progress: 0,
    minLynxVersion: '',
    entitlementRequired: c.accessMode !== 'free',
    hasEntitlement: false,
    chapters: c.chapters.map((ch) => ({
      id: ch.chapterId,
      key: ch.slug,
      title: ch.title,
      kind: 'document',
    })),
    labs: [],
  }));
}

/** Pinned enrollments take precedence over the latest advertised release; never repin here. */
export function mergeLearnCatalog(
  published: LearningCourse[],
  enrolled: LearningCourse[],
  legacyEnrolled: LearningCourse[],
  local: LearningCourse[]
): LearningCourse[] {
  const owned = [...enrolled, ...legacyEnrolled];
  return [
    ...owned,
    ...published.filter(
      (c) => !owned.some((e) => (c.courseId && e.courseId === c.courseId) || e.slug === c.slug)
    ),
    ...local,
  ];
}
