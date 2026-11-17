import { invokeCommand } from '../../Platform/IPC';

export type RecordEventInput = Parameters<
  typeof invokeCommand<'record_center_append_event'>
>[1] extends infer Args
  ? NonNullable<Args> extends { request: infer Request }
    ? Request
    : never
  : never;

/** Recording is diagnostic infrastructure and must never break the owning operation. */
export async function appendRecordEvent(request: RecordEventInput) {
  return invokeCommand('record_center_append_event', { request });
}
