import type { IpcCommandSpec } from '../Types';

export interface RecordCenterEvent {
  eventId: string;
  occurredAtMs: number;
  actorType: string;
  actorId: string;
  category: string;
  action: string;
  scopeType: string;
  scopeId: string;
  boardUid: string;
  sessionId: string | null;
  correlationId: string;
  causationId: string | null;
  result: string;
  errorCode: string | null;
  summary: string;
  metadata: Record<string, unknown>;
}

export interface RecordCenterSession {
  sessionId: string;
  kind: string;
  title: string;
  scopeType: string;
  scopeId: string;
  boardUid: string;
  startedAtMs: number;
  endedAtMs: number | null;
  eventCount: number;
  integrity: string;
}

export interface RecordCenterFeedbackDraft {
  id: string;
  title: string;
  problemType: string;
  page: string;
  expected: string;
  actual: string;
  reproducible: string;
  status: string;
  eventIds: string[];
  includeSensitiveBody: boolean;
  createdAtMs: number;
  updatedAtMs: number;
}

export interface RecordCenterBackup {
  id: string;
  path: string;
  sha256: string;
  eventCount: number;
  sessionCount: number;
  feedbackCount: number;
  createdAtMs: number;
}

export interface RecordCenterRestorePreview {
  path: string;
  schemaVersion: number;
  newEvents: number;
  conflictingEvents: number;
  newSessions: number;
  conflictingSessions: number;
  newFeedback: number;
  conflictingFeedback: number;
  contentSha256: string;
}

export interface RecordCenterRestoreResult {
  importedEvents: number;
  importedSessions: number;
  importedFeedback: number;
  skippedConflicts: number;
}

export interface RecordCenterCommandMap {
  record_center_append_event: IpcCommandSpec<
    {
      request: {
        actorType: string;
        actorId: string;
        category: string;
        action: string;
        scopeType?: string;
        scopeId?: string;
        sessionId?: string | null;
        sessionKind?: string | null;
        sessionTitle?: string | null;
        endSession?: boolean;
        correlationId?: string | null;
        causationId?: string | null;
        result: string;
        errorCode?: string | null;
        summary: string;
        metadata?: Record<string, unknown>;
      };
    },
    RecordCenterEvent
  >;
  record_center_list_events: IpcCommandSpec<
    {
      filter?: {
        query?: string;
        actorType?: string;
        scopeId?: string;
        result?: string;
        errorCode?: string;
        sessionId?: string;
        limit?: number;
      };
    },
    RecordCenterEvent[]
  >;
  record_center_overview: IpcCommandSpec<
    undefined,
    {
      todayEventCount: number;
      activeSerialSessions: number;
      aiSessionCount: number;
      deviceErrorCount: number;
      pendingFeedbackCount: number;
      gapCount: number;
      latestBackupAtMs: number | null;
      lanServiceEnabled: boolean;
    }
  >;
  record_center_list_sessions: IpcCommandSpec<{ limit?: number }, RecordCenterSession[]>;
  record_center_save_feedback_draft: IpcCommandSpec<
    {
      actorId?: string;
      request: {
        id?: string;
        title: string;
        problemType: string;
        page: string;
        expected: string;
        actual: string;
        reproducible: string;
        eventIds?: string[];
        includeSensitiveBody?: boolean;
      };
    },
    RecordCenterFeedbackDraft
  >;
  record_center_list_feedback_drafts: IpcCommandSpec<undefined, RecordCenterFeedbackDraft[]>;
  record_center_create_backup: IpcCommandSpec<
    { targetDir: string; actorId?: string },
    RecordCenterBackup
  >;
  record_center_list_backups: IpcCommandSpec<undefined, RecordCenterBackup[]>;
  record_center_preview_restore: IpcCommandSpec<
    { sourcePath: string; actorId?: string },
    RecordCenterRestorePreview
  >;
  record_center_restore_backup: IpcCommandSpec<
    { sourcePath: string; actorId?: string },
    RecordCenterRestoreResult
  >;
}
