/** Namespaced v2 read contracts; these never grant execution or synthesize progress. */
export interface LearnV2Enrollment {
  courseSlug?: string | null;
  enrollmentId: string;
  courseId: string;
  releaseId: string;
  manifestSha256: string;
  title: string;
  version: string;
  summary: string;
  track: string;
  currentUnitId: string | null;
}
export interface LearnV2Home {
  availability: 'available' | 'not_deployed';
  enrolled: LearnV2Enrollment[];
}
export interface LearnCoursePin {
  enrollmentId: string;
  releaseId: string;
  manifestSha256: string;
}
export interface LearnCourseResource {
  resourceId: string;
  kind: 'document' | 'slides' | 'video' | 'source' | 'sdk' | 'image';
  title: string;
  sourceUrl: string;
  sha256: string | null;
  size: number | null;
  artifactRef: string | null;
  commit: string | null;
}
export interface LearnActivity {
  activityId: string;
  kind: 'read' | 'understand' | 'code' | 'build' | 'deploy' | 'verify' | 'reflect';
  title: string;
  instruction: string;
  expectedObservation: string;
  recoveryHint: string;
  required: boolean;
  resourceRefs: string[];
  environmentRef: string;
  actionRefs: string[];
  ruleRefs: string[];
  completionMode: 'user_confirmed' | 'quiz' | 'validator';
}
export interface LearnCourseUnit {
  unitId: string;
  title: string;
  objective: string;
  activities: LearnActivity[];
}
export interface LearnCourseManifest {
  format: '100ask.learn-course';
  schemaVersion: 1 | 2;
  courseId: string;
  releaseId: string;
  version: string;
  title: string;
  track: 'kvm' | 'multimedia' | 'bsp' | 'linux_app' | 'theory' | 'custom';
  requiredClientCapabilities: string[];
  resources: LearnCourseResource[];
  environments: Array<{
    profileId: string;
    target: 'none' | 'ubuntu' | 'board';
    requiredCapabilities: string[];
    versionChecks: Array<{ component: string; operator: string; expected: string }>;
  }>;
  actions: Array<{
    actionId: string;
    capabilityId?: string;
    catalogVersion: string;
    target: 'ubuntu' | 'board';
    risk: string;
    parameters: Record<string, unknown>;
    evidenceOutputs: string[];
  }>;
  rules: Array<{
    ruleId: string;
    validatorId: string;
    validatorVersion: string;
    parameters: Record<string, unknown>;
    requiredEvidenceKinds: string[];
    minimumTrust: string;
  }>;
  chapters: Array<{
    chapterId: string;
    title: string;
    units: LearnCourseUnit[];
    summaryPrompt: string;
  }>;
}
export interface LearnVerifiedCourse {
  enrollmentId: string;
  manifestSha256: string;
  signingKeyId: string;
  manifest: LearnCourseManifest;
  missingCapabilities: string[];
  executionEnabled: boolean;
  agentPolicy: {
    enabled: boolean;
    hintLevels: string[];
    contextResourceRefs: string[];
    maxContextBytes: number;
    allowedToolIds: string[];
    allowImageAnalysis: boolean;
  };
}
export interface LearnUnitContent {
  enrollmentId: string;
  releaseId: string;
  unitId: string;
  documents: Array<{ resourceId: string; sha256: string; markdown: string }>;
}

export interface LearnVerifiedPreview {
  currentUnitId?: string | null;
  binding: {
    format: '100ask.learn-preview-binding';
    schemaVersion: 1;
    previewId: string;
    userId: number;
    draftId: string;
    draftRevision: number;
    contentDigest: string;
    courseId: string;
    releaseId: string;
    manifestSha256: string;
    policySha256: string;
    allowedUnitIds: string[];
    expiresAt: string;
    scope: 'draft_readonly';
  };
  course: LearnVerifiedCourse;
}
export interface LearnPreviewSummary {
  previewId: string;
  title: string;
  expiresAt: string;
  draftRevision: number;
  chapterCount: number;
  unitCount: number;
  currentUnitId: string | null;
}
export interface LearnPreviewList {
  items: LearnPreviewSummary[];
  discoveryAvailable: boolean;
  warnings: string[];
}
export interface LearnPreviewReadReceipt {
  format: '100ask.learn-preview-read-receipt';
  schemaVersion: 1;
  receiptId: string;
  previewId: string;
  userId: number;
  draftRevision: number;
  contentDigest: string;
  manifestSha256: string;
  scope: 'read_integration';
  status: 'server_resources_verified';
  clientObservationTrust: 'client_reported';
  publicationEligible: false;
  clientVersion: string;
  platform: string;
  verifiedAt: string;
  documents: Array<{ unitId: string; resourceId: string; sha256: string }>;
}

/** Explicit learner assertions; never a practice grade. */
export interface LearnRecordEntry {
  chapterId: string;
  unitId: string;
  activityId: string;
  title: string;
  selfConfirmable: boolean;
  confirmed: boolean;
  reflection: string;
  trustLevel: 'user_asserted';
  updatedAt: string;
}
export interface LearnLearningRecord extends LearnCoursePin {
  revision: number;
  entries: LearnRecordEntry[];
}
export interface LearnRecordRequest extends LearnCoursePin {
  expectedRevision: number;
  unitId: string;
  activityId: string;
  kind: 'confirm' | 'reflection';
  text: string;
  requestId: string;
}

export interface LearnEnvironmentRequest extends LearnCoursePin {
  profileId: string;
}

export interface LearnPracticeSyncSelection extends LearnEnvironmentRequest {
  batchId: string;
  unitId: string;
  runs: Array<{ runId: string; snapshotSha256: string }>;
}
export interface LearnPracticeSyncBatch extends LearnEnvironmentRequest {
  batchId: string;
  unitId: string;
  runIds: string[];
  evidenceCount: number;
  verifiedCount: number;
  bytes: number;
  status: 'pending' | 'conflict' | 'accepted';
  stage: 'session' | 'events' | 'evidence' | 'grant' | 'upload' | 'completion' | 'done';
  revision: number;
  attemptId: string | null;
  errorCode: string | null;
  retryable: boolean;
}
export interface LearnEnvironmentBinding {
  projectId: number;
  projectName: string;
  hostName: string;
  workspacePath: string;
  boardUid: string;
  boardName: string;
  sshAlias: string;
  configDigest: string;
  changed: boolean;
}
export interface LearnEnvironmentView {
  profileId: string;
  target: string;
  binding: LearnEnvironmentBinding | null;
  projects: Array<{
    projectId: number;
    name: string;
    hostName: string;
    workspacePath: string;
    hasBoard: boolean;
  }>;
}
export interface LearnEnvironmentInspection {
  commit: string;
  branch: string;
  dirty: boolean;
  untracked: boolean;
  diffSha256: string;
  os: string;
  architecture: string;
  compiler: string;
  checkedAt: string;
  fingerprint: string;
  checks: Array<{ component: string; expected: string; observed: string; status: string }>;
}

export interface LearnUnitOutcome {
  chapterId: string;
  unitId: string;
  title: string;
  objective: string;
  required: boolean;
  passed: boolean;
  state: string;
  currentAttemptId: string;
}
export interface LearnLearningSummary extends LearnCoursePin {
  revision: number;
  allRequiredPassed: boolean;
  units: LearnUnitOutcome[];
}
export interface LearnAssessmentResult {
  status: string;
  passedRules: string[];
  failedRules: string[];
  missingEvidence: string[];
  unverified: string[];
  verificationLevel: string;
}
export interface LearnReviewResult {
  attemptId: string;
  assessment: LearnAssessmentResult;
  summary: LearnLearningSummary;
}
export interface LearnReviewRequest extends LearnCoursePin {
  expectedRevision: number;
  unitId: string;
  attemptId: string;
  answers: Record<string, string>;
  reflection: string;
  requestId: string;
}
export interface LearnReportRequest extends LearnCoursePin {
  scope: 'chapter' | 'course';
  chapterId: string;
  inputRevision: number;
  requestId: string;
}
export interface LearnCourseReport {
  reportId: string;
  enrollmentId: string;
  releaseId: string;
  inputRevision: number;
  inputDigest: string;
  scope: 'chapter' | 'course';
  chapterId: string;
  status: 'completed';
  allRequiredPassed: boolean;
  units: LearnUnitOutcome[];
  unverified: string[];
  reflections: { unitId: string; activityId: string; text: string; occurredAt: string }[];
}
export interface LearnReportJob {
  jobId: string;
  status: string;
  report: LearnCourseReport | null;
}
export interface LearnActionRequest extends LearnEnvironmentRequest {
  unitId: string;
  activityId: string;
  actionId: string;
}
export interface LearnActionRun {
  runId: string;
  request: LearnActionRequest;
  projectId: number;
  projectName: string;
  hostName: string;
  state: string;
  updatedAt: string;
  syncStatus: string;
  result: {
    confirmation?: string;
    task?: {
      workspace: string;
      commit: string;
      argv: string[];
      cwd: string;
      timeoutSeconds: number;
      sourceFingerprint: string;
    };
    startedAt?: string;
    finishedAt?: string;
    log?: string;
    logBytes?: number;
    logSha256?: string;
    exitCode?: number;
    reason?: string;
    artifacts?: { path: string; sha256: string; size: number }[];
  };
}

/** Public summaries from the dedicated published Learn catalog, not ordinary website lessons. */
export interface LearnPublishedCourse {
  courseId: string;
  courseSlug: string;
  releaseId: string;
  title: string;
  version: string;
  track: string;
  accessMode: 'free' | 'paid' | 'mixed';
  chapters: Array<{
    chapterId: string;
    slug: string;
    title: string;
    units: Array<{ unitId: string; title: string }>;
  }>;
}
/** A local observation, never an assessment or cloud acceptance receipt. */
export interface LearnBoardObservation {
  observationId: string;
  request: LearnActionRequest;
  projectId: number;
  projectName: string;
  deviceId: number;
  boardUid: string;
  boardName: string;
  bindingDigest: string;
  state: 'unknown' | 'observed' | 'failed' | 'stale';
  startedAt: string;
  finishedAt: string | null;
  snapshot: {
    transport: string;
    observedAtMs: number;
    osName: string;
    kernel: string;
    architecture: string;
    cpuModel: string;
    cpuCores: number | null;
    memoryTotalBytes: number | null;
    uptimeSeconds: number | null;
  } | null;
  evidenceSha256: string | null;
  errorCode: string | null;
  syncStatus: 'local_only';
  trust: 'client_observation';
}

/** Locally collected complete content; no upload or assessment receipt. */
export interface LearnActionEvidencePreview {
  snapshotSha256: string;
  runId: string;
  request: LearnActionRequest;
  bindingDigest: string;
  collectedAt: string;
  trust: 'client_observation';
  syncStatus: 'local_only';
  executionContext?: {
    collectorVersion: string;
    sourceCommit: string;
    sourceFingerprint: string;
    definitionSha256: string;
    facts: { os: string; architecture: string; pythonVersion: string };
    fingerprint: string;
    observedAt: string;
  };
  items: Array<{
    kind: 'stdout' | 'build_log' | 'exit_status';
    state: string;
    startedAt: string;
    finishedAt: string;
    contentType: string;
    sha256: string;
    size: number;
    rawSha256: string;
    rawBytes: number;
    redacted: boolean;
    outputTruncated: boolean;
    collectorVersion: string;
    text: string;
  }>;
}
