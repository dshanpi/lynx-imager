import type {
  LearnCoursePin,
  LearnReportJob,
  LearnReportRequest,
} from '../../../Platform/IPC/Commands';

/** A resumable request index, never an assessment or a trusted report cache. */
export interface LearnReportTask extends LearnReportRequest {
  jobId: string;
}
const validId = (value: unknown): value is string =>
  typeof value === 'string' && /^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$/.test(value);
const failure = (machineCode: string) => ({ machineCode });

export function reportTaskKey(pin: LearnCoursePin, chapterId: string) {
  return `learn-report-task-v1:${JSON.stringify([pin.enrollmentId, pin.releaseId, pin.manifestSha256, chapterId])}`;
}

/** Read only after the backend has authenticated this exact enrollment pin. */
export function readReportTask(
  storage: Pick<Storage, 'getItem'>,
  key: string,
  pin: LearnCoursePin,
  chapterId: string
): LearnReportTask | null {
  let raw: string | null;
  try {
    raw = storage.getItem(key);
  } catch {
    throw failure('LEARN_REPORT_RECOVERY_UNAVAILABLE');
  }
  if (!raw) return null;
  try {
    if (raw.length > 8192) throw new Error();
    const row = JSON.parse(raw) as LearnReportTask;
    if (
      !row ||
      typeof row !== 'object' ||
      Array.isArray(row) ||
      row.enrollmentId !== pin.enrollmentId ||
      row.releaseId !== pin.releaseId ||
      row.manifestSha256 !== pin.manifestSha256 ||
      !validId(row.requestId) ||
      !Number.isSafeInteger(row.inputRevision) ||
      row.inputRevision < 1 ||
      !(row.jobId === '' || validId(row.jobId)) ||
      !(
        (row.scope === 'chapter' && row.chapterId === chapterId) ||
        (row.scope === 'course' && row.chapterId === '')
      )
    )
      throw new Error();
    // Only known request fields can be replayed, even if local storage was edited.
    return {
      ...pin,
      requestId: row.requestId,
      inputRevision: row.inputRevision,
      scope: row.scope,
      chapterId: row.chapterId,
      jobId: row.jobId,
    };
  } catch {
    throw failure('LEARN_REPORT_RECOVERY_INVALID');
  }
}

export function writeReportTask(
  storage: Pick<Storage, 'setItem'>,
  key: string,
  task: LearnReportTask
) {
  try {
    storage.setItem(key, JSON.stringify(task));
  } catch {
    throw failure('LEARN_REPORT_RECOVERY_UNAVAILABLE');
  }
}

/** Rust checks signed course/report contents; the UI also checks the exact requested snapshot. */
export function validateReportJob(task: LearnReportTask, value: LearnReportJob) {
  const report = value.report;
  if (
    value.jobId !== task.jobId ||
    !['queued', 'running', 'succeeded', 'failed', 'cancelled'].includes(value.status) ||
    (value.status !== 'succeeded' && report !== null) ||
    (value.status === 'succeeded' &&
      (!report ||
        report.status !== 'completed' ||
        report.enrollmentId !== task.enrollmentId ||
        report.releaseId !== task.releaseId ||
        report.scope !== task.scope ||
        report.chapterId !== task.chapterId ||
        report.inputRevision !== task.inputRevision))
  ) {
    throw failure('LEARN_REPORT_MISMATCH');
  }
}
