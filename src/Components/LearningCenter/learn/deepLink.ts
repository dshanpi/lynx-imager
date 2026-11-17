import type { LearningDeepLinkTarget } from '../types';

// A URI is an intent, never an entitlement. Leave anonymous intents in the parent
// until login or explicit cancellation; account-keyed children may be remounted.
export function learnDeepLinkDisposition(
  target: LearningDeepLinkTarget | null,
  active: boolean,
  authenticated: boolean,
  loaded: boolean,
  loading: boolean
): 'wait' | 'login' | 'preview' | 'course' {
  if (!active || !target) return 'wait';
  if (!authenticated) return 'login';
  if (target.previewId) return 'preview';
  return loaded && !loading ? 'course' : 'wait';
}
