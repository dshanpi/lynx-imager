export type TerminalWorkspaceProfileId =
  'five-pane' | 'four-pane' | 'three-pane' | 'two-pane' | 'single-pane';

export interface TerminalWorkspaceProfile {
  id: TerminalWorkspaceProfileId;
  visiblePaneCount: number;
  areas: string[];
  columns: string;
  rows: string;
  slots: string[];
}

const PROFILES: TerminalWorkspaceProfile[] = [
  {
    id: 'five-pane',
    visiblePaneCount: 5,
    areas: ['auxiliary-a main auxiliary-c', 'auxiliary-b main auxiliary-d'],
    columns: 'minmax(260px, 0.82fr) minmax(560px, 1.72fr) minmax(260px, 0.82fr)',
    rows: 'minmax(0, 1fr) minmax(0, 1fr)',
    slots: ['auxiliary-a', 'auxiliary-b', 'main', 'auxiliary-c', 'auxiliary-d'],
  },
  {
    id: 'four-pane',
    visiblePaneCount: 4,
    areas: ['auxiliary-a main auxiliary-c', 'auxiliary-b main auxiliary-c'],
    columns: 'minmax(250px, 0.8fr) minmax(520px, 1.65fr) minmax(250px, 0.8fr)',
    rows: 'minmax(0, 1fr) minmax(0, 1fr)',
    slots: ['auxiliary-a', 'auxiliary-b', 'main', 'auxiliary-c'],
  },
  {
    id: 'three-pane',
    visiblePaneCount: 3,
    areas: ['main main', 'auxiliary-a auxiliary-b'],
    columns: 'minmax(0, 1fr) minmax(0, 1fr)',
    rows: 'minmax(300px, 1.35fr) minmax(220px, 0.65fr)',
    slots: ['auxiliary-a', 'auxiliary-b', 'main'],
  },
  {
    id: 'two-pane',
    visiblePaneCount: 2,
    areas: ['main', 'auxiliary-a'],
    columns: 'minmax(0, 1fr)',
    rows: 'minmax(300px, 1fr) minmax(220px, 0.7fr)',
    slots: ['auxiliary-a', 'main'],
  },
  {
    id: 'single-pane',
    visiblePaneCount: 1,
    areas: ['main'],
    columns: 'minmax(0, 1fr)',
    rows: 'minmax(0, 1fr)',
    slots: ['main'],
  },
];

export function selectTerminalWorkspaceProfile(
  width: number,
  height: number,
  paneCount: number
): TerminalWorkspaceProfile {
  const available = Math.max(1, Math.min(5, Math.floor(paneCount)));
  let target = 1;
  if (width >= 1500 && height >= 820) target = 5;
  else if (width >= 1180 && height >= 700) target = 4;
  else if (width >= 900 && height >= 600) target = 3;
  else if (width >= 760 && height >= 540) target = 2;
  target = Math.min(target, available);
  return PROFILES.find((profile) => profile.visiblePaneCount === target) ?? PROFILES[4];
}

export const DEFAULT_TERMINAL_WORKSPACE_ORDER = [
  'serial',
  'primary-ssh',
  'device-status',
  'task-log',
] as const;
