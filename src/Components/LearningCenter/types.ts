export type LearningSection = 'home' | 'hardware' | 'courses' | 'labs' | 'history';

export type LearningView = LearningSection | 'cloud-course' | 'precheck' | 'local-course';

export type { LearningDeepLinkTarget } from '../../Platform/IPC/Commands';

export interface LearningBoardProfile {
  profileId: string;
  displayName: string;
  vendor?: string;
  soc?: string;
}

export interface LearningLab {
  id: string;
  key: string;
  title: string;
  summary: string;
  durationMinutes: number;
  riskLevel: 'L0' | 'L1' | 'L2' | 'L3' | 'L4';
  packageState: 'bundled-preview' | 'ready' | 'downloaded' | 'not-downloaded';
  packageVersion: string;
  entitlementRequired: boolean;
  hasEntitlement: boolean;
  boardProfiles: LearningBoardProfile[];
  sdkRequirement: string;
  minLynxVersion: string;
}

export interface LearningChapter {
  id: string;
  key: string;
  title: string;
  kind: 'document' | 'video' | 'lab';
  labId?: string;
  sourceUrl?: string;
  completed?: boolean;
  locked?: boolean;
}

export interface LearningCourse {
  courseId?: string;
  catalogOnly?: boolean;
  id: string;
  slug: string;
  title: string;
  version: string;
  summary: string;
  cover?: string;
  progress: number;
  minLynxVersion: string;
  source: 'bundled-preview' | 'cloud' | 'local';
  localCourseId?: string;
  resumeSessionId?: string;
  enrollmentId?: string;
  cloudProtocol?: 'v1' | 'v2';
  releaseId?: string;
  manifestSha256?: string;
  currentUnitId?: string;
  track?: string;
  level?: string;
  entitlementRequired?: boolean;
  hasEntitlement?: boolean;
  lynxReady?: boolean;
  chapters: LearningChapter[];
  labs: LearningLab[];
}

export interface LearningProgress {
  resourceType: string;
  resourceId: string;
  state: string;
  percent: number;
  sessionId: string;
  revision: number;
  updatedAt: number;
}

export interface LearningPrecheckItem {
  id: 'lynx' | 'account' | 'package' | 'sdk-host' | 'sdk' | 'board' | 'serial';
  label: string;
  value: string;
  status: 'pass' | 'warning' | 'needs-config' | 'blocked';
  detail: string;
  actionLabel?: string;
}

export interface LearningInventoryTarget {
  id: number;
  name: string;
}

export interface LearningInventory {
  devices: LearningInventoryTarget[];
  hosts: LearningInventoryTarget[];
  linkedProjectCount: number;
}
