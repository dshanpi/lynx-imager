/** Unsubmitted learner notes, never an authoritative assessment. Read only after enrollment verification. */
export type ReflectionDrafts = Record<string, Record<string, string>>;
interface Pin {
  enrollmentId: string;
  releaseId: string;
  manifestSha256: string;
}
const validId = (value: string) =>
  /^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$/.test(value) &&
  !['__proto__', 'constructor', 'prototype'].includes(value);
const invalid = () => new Error('本机草稿格式损坏或超限，原始数据未被覆盖。');

export function reflectionStorageKey(accountId: number, pin: Pin) {
  if (
    !Number.isSafeInteger(accountId) ||
    accountId <= 0 ||
    !validId(pin.enrollmentId) ||
    !validId(pin.releaseId) ||
    !/^[a-f0-9]{64}$/i.test(pin.manifestSha256)
  )
    throw invalid();
  return `learn-reflection-draft-v1:${JSON.stringify([accountId, pin.enrollmentId, pin.releaseId, pin.manifestSha256])}`;
}

function validate(value: unknown): ReflectionDrafts {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw invalid();
  const units = Object.entries(value);
  if (units.length > 256) throw invalid();
  return Object.fromEntries(
    units.map(([unitId, raw]) => {
      if (!validId(unitId) || !raw || typeof raw !== 'object' || Array.isArray(raw))
        throw invalid();
      const entries = Object.entries(raw);
      if (
        entries.length > 100 ||
        entries.some(([id, text]) => !validId(id) || typeof text !== 'string' || text.length > 4096)
      )
        throw invalid();
      return [unitId, Object.fromEntries(entries) as Record<string, string>];
    })
  );
}

export function readReflectionDrafts(storage: Pick<Storage, 'getItem'>, key: string) {
  const raw = storage.getItem(key);
  if (raw === null) return {};
  if (raw.length > 256 * 1024) throw invalid();
  const value: unknown = JSON.parse(raw);
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw invalid();
  const row = value as Record<string, unknown>;
  if (row.version !== 1) throw invalid();
  return validate(row.units);
}

export function writeReflectionDrafts(
  storage: Pick<Storage, 'setItem' | 'removeItem'>,
  key: string,
  drafts: ReflectionDrafts
) {
  const units = Object.fromEntries(
    Object.entries(validate(drafts)).filter(([, entries]) => Object.keys(entries).length > 0)
  );
  const raw = JSON.stringify({ version: 1, units });
  if (raw.length > 256 * 1024) throw invalid();
  if (Object.keys(units).length === 0) storage.removeItem(key);
  else storage.setItem(key, raw);
}
