/** Unsubmitted inputs only. Call after the enrollment has been authenticated. */
export interface ReviewDraft {
  answers: Record<string, string>;
  reflection: string;
}
export function reviewDraftKey(
  enrollmentId: string,
  releaseId: string,
  digest: string,
  unitId: string
) {
  return `learn-review-draft-v1:${JSON.stringify([enrollmentId, releaseId, digest, unitId])}`;
}
export function readReviewDraft(
  storage: Pick<Storage, 'getItem'>,
  key: string
): ReviewDraft | null {
  const raw = storage.getItem(key);
  if (!raw) return null;
  if (raw.length > 256 * 1024) throw new Error('草稿超限，请先导出浏览器本地数据后整理。');
  const value: unknown = JSON.parse(raw);
  if (!value || typeof value !== 'object' || Array.isArray(value))
    throw new Error('草稿格式损坏。');
  const row = value as Record<string, unknown>;
  if (
    typeof row.reflection !== 'string' ||
    row.reflection.length > 4096 ||
    !row.answers ||
    typeof row.answers !== 'object' ||
    Array.isArray(row.answers)
  )
    throw new Error('草稿格式损坏。');
  const entries = Object.entries(row.answers);
  if (
    entries.length > 100 ||
    entries.some(
      ([id, text]) =>
        !/^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$/.test(id) ||
        typeof text !== 'string' ||
        text.length > 4096
    )
  )
    throw new Error('草稿格式损坏。');
  return {
    answers: Object.fromEntries(entries) as Record<string, string>,
    reflection: row.reflection,
  };
}
export function writeReviewDraft(
  storage: Pick<Storage, 'setItem'>,
  key: string,
  draft: ReviewDraft
) {
  const raw = JSON.stringify(draft);
  if (raw.length > 256 * 1024) throw new Error('草稿超限。');
  storage.setItem(key, raw);
}
