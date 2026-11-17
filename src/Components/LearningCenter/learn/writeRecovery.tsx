import { useCallback, useMemo, useState } from 'react';
import type {
  LearnCoursePin,
  LearnRecordRequest,
  LearnReviewRequest,
} from '../../../Platform/IPC/Commands';

type Kind = 'record' | 'review';
type Request = LearnRecordRequest | LearnReviewRequest;
export interface WriteScope {
  accountId: number;
  pin: LearnCoursePin;
  unitId: string;
  kind: Kind;
}
const fail = (machineCode: string) => ({ machineCode });
const validId = (v: unknown): v is string =>
  typeof v === 'string' && /^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$/.test(v);
const text = (v: unknown, empty = true): v is string =>
  typeof v === 'string' &&
  !v.includes('\0') &&
  new TextEncoder().encode(v).length <= 4096 &&
  (empty || !!v.trim());
export function writeRecoveryKey(scope: WriteScope) {
  return `learn-write-v1:${JSON.stringify([scope.accountId, scope.pin.enrollmentId, scope.pin.releaseId, scope.pin.manifestSha256, scope.unitId, scope.kind])}`;
}
function decode(raw: string, scope: WriteScope): Request {
  if (
    !Number.isSafeInteger(scope.accountId) ||
    scope.accountId < 1 ||
    new TextEncoder().encode(raw).length > 512 * 1024
  )
    throw fail('LEARN_WRITE_RECOVERY_INVALID');
  try {
    const row = JSON.parse(raw);
    const common = [
      'enrollmentId',
      'releaseId',
      'manifestSha256',
      'unitId',
      'expectedRevision',
      'requestId',
    ];
    const fields = [
      ...common,
      ...(scope.kind === 'record'
        ? ['activityId', 'kind', 'text']
        : ['attemptId', 'answers', 'reflection']),
    ];
    if (
      !row ||
      typeof row !== 'object' ||
      Array.isArray(row) ||
      Object.keys(row).sort().join('|') !== fields.sort().join('|') ||
      row.enrollmentId !== scope.pin.enrollmentId ||
      row.releaseId !== scope.pin.releaseId ||
      row.manifestSha256 !== scope.pin.manifestSha256 ||
      row.unitId !== scope.unitId ||
      ![row.enrollmentId, row.releaseId, row.unitId, row.requestId].every(validId) ||
      !/^[a-f0-9]{64}$/.test(row.manifestSha256) ||
      !Number.isSafeInteger(row.expectedRevision) ||
      row.expectedRevision < 1
    )
      throw new Error();
    if (scope.kind === 'record') {
      if (
        !validId(row.activityId) ||
        !['confirm', 'reflection'].includes(row.kind) ||
        !text(row.text, row.kind === 'confirm') ||
        (row.kind === 'confirm' && row.text !== '')
      )
        throw new Error();
    } else {
      if (
        !(row.attemptId === '' || validId(row.attemptId)) ||
        !text(row.reflection) ||
        !row.answers ||
        typeof row.answers !== 'object' ||
        Array.isArray(row.answers)
      )
        throw new Error();
      const entries = Object.entries(row.answers);
      if (
        entries.length > 100 ||
        entries.some(([id, value]) => !validId(id) || !text(value, false))
      )
        throw new Error();
    }
    return row as Request;
  } catch {
    throw fail('LEARN_WRITE_RECOVERY_INVALID');
  }
}
export function readPendingWrite(
  storage: Pick<Storage, 'getItem'>,
  scope: WriteScope
): Request | null {
  if (!Number.isSafeInteger(scope.accountId) || scope.accountId < 1)
    throw fail('LEARN_WRITE_RECOVERY_INVALID');
  let raw: string | null;
  try {
    raw = storage.getItem(writeRecoveryKey(scope));
  } catch {
    throw fail('LEARN_WRITE_RECOVERY_UNAVAILABLE');
  }
  return raw === null ? null : decode(raw, scope);
}
export function savePendingWrite(
  storage: Pick<Storage, 'getItem' | 'setItem'>,
  scope: WriteScope,
  request: Request
) {
  const encoded = JSON.stringify(request);
  decode(encoded, scope);
  const existing = readPendingWrite(storage, scope);
  if (existing && JSON.stringify(existing) !== encoded) throw fail('LEARN_WRITE_PENDING');
  try {
    storage.setItem(writeRecoveryKey(scope), encoded);
  } catch {
    throw fail('LEARN_WRITE_RECOVERY_UNAVAILABLE');
  }
}
export function clearPendingWrite(
  storage: Pick<Storage, 'getItem' | 'removeItem'>,
  scope: WriteScope,
  request: Request
) {
  const existing = readPendingWrite(storage, scope);
  if (!existing || JSON.stringify(existing) !== JSON.stringify(request))
    throw fail('LEARN_WRITE_PENDING');
  try {
    storage.removeItem(writeRecoveryKey(scope));
  } catch {
    throw fail('LEARN_WRITE_RECOVERY_UNAVAILABLE');
  }
}
/** Called only after a successful authenticated GET for this enrollment. No network or auto replay. */
export function useWriteRecovery<T extends Request>(scope: WriteScope) {
  const {
    accountId,
    unitId,
    kind,
    pin: { enrollmentId, releaseId, manifestSha256 },
  } = scope;
  const stableScope = useMemo(
    () => ({ accountId, unitId, kind, pin: { enrollmentId, releaseId, manifestSha256 } }),
    [accountId, unitId, kind, enrollmentId, releaseId, manifestSha256]
  );
  const key = writeRecoveryKey(stableScope);
  const [state, setState] = useState<{ key: string; ready: boolean; pending: T | null }>({
    key,
    ready: false,
    pending: null,
  });
  const reset = useCallback(() => setState({ key, ready: false, pending: null }), [key]);
  const load = useCallback(() => {
    const pending = readPendingWrite(localStorage, stableScope) as T | null;
    setState({ key, ready: true, pending });
  }, [key, stableScope]);
  return {
    ready: state.key === key && state.ready,
    pending: state.key === key ? state.pending : null,
    reset,
    load,
    begin: (request: T) => {
      savePendingWrite(localStorage, scope, request);
      setState({ key, ready: true, pending: request });
    },
    complete: (request: T) => {
      clearPendingWrite(localStorage, scope, request);
      setState({ key, ready: true, pending: null });
    },
  };
}
export function PendingWriteNotice({
  request,
  disabled,
  retry,
}: {
  request: Request | null;
  disabled: boolean;
  retry: () => void;
}) {
  if (!request) return null;
  return (
    <div className="learn-notice" role="status">
      <p>有一次提交尚未确认。不会自动发送，也不会用新的输入替换原请求。</p>
      <details>
        <summary>查看待确认的原输入</summary>
        {'text' in request ? (
          <p>{request.kind === 'confirm' ? '我已阅读并理解（本人自述）' : request.text}</p>
        ) : (
          <>
            <p>{request.reflection}</p>
            {Object.values(request.answers).map((answer, index) => (
              <p key={index}>{answer}</p>
            ))}
          </>
        )}
      </details>
      <button className="learning-secondary" disabled={disabled} onClick={retry}>
        重试原提交
      </button>
    </div>
  );
}
