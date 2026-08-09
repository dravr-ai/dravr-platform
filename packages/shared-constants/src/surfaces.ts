// ABOUTME: Single registry of the user-facing surfaces Dravr offers, per platform
// ABOUTME: Each client asserts it implements this list — the registry is the source, not a mirror

/**
 * The user-facing surfaces of the product, and where each is reachable.
 *
 * This exists because web and mobile drifted silently: mobile shipped rows for
 * Profile, Privacy and Personal Information whose destinations were never
 * built, and screens for Memory and Billing that existed but had nothing
 * navigating to them. Nothing failed — the gap was only visible by driving both
 * apps side by side.
 *
 * Deliberately NOT a comparison between two lists. A test that diffs web's tabs
 * against mobile's would be an invariant policing drift between two systems,
 * which just makes the duplication tolerable. Instead this is the single
 * declaration of intent, and each platform has one test asserting it delivers
 * what is declared here. Adding a surface means editing this file first; the
 * platform tests then tell you which client is missing it.
 *
 * `web` and `mobile` hold the route each platform serves the surface at, or
 * `null` when the surface is deliberately absent there — with `why` recording
 * the reason, so a null is a decision rather than an oversight.
 */
export interface UserSurface {
  /** Stable id for the surface, independent of either platform's routing. */
  id: string;
  /** What the athlete calls it. */
  label: string;
  /** Web route: the dashboard hash, or null when deliberately web-absent. */
  web: string | null;
  /** Mobile route: the expo-router path, or null when deliberately absent. */
  mobile: string | null;
  /** Required when either side is null: why that platform does not have it. */
  why?: string;
}

export const USER_SURFACES: readonly UserSurface[] = [
  // ---- primary destinations ----
  { id: 'chat', label: 'Chat', web: 'chat', mobile: '/(app)/(tabs)/(chat)' },
  { id: 'coaches', label: 'Coaches', web: 'my-coaches', mobile: '/(app)/(tabs)/(coaches)' },
  { id: 'discover', label: 'Discover', web: 'discover', mobile: '/(app)/(tabs)/(discover)' },
  { id: 'groups', label: 'Groups', web: 'groups', mobile: '/(app)/(tabs)/(groups)' },
  { id: 'insights', label: 'Insights', web: 'insights', mobile: '/(app)/(tabs)/(social)' },
  { id: 'notifications', label: 'Notifications', web: 'notifications', mobile: '/(app)/notifications' },

  // ---- account & settings ----
  { id: 'profile', label: 'Profile', web: 'settings', mobile: '/(app)/(tabs)/(settings)/profile' },
  {
    id: 'data-providers',
    label: 'Data Providers',
    web: 'data-providers',
    mobile: '/(app)/(tabs)/(settings)/connections',
  },
  {
    id: 'coaching-style',
    label: 'Coaching Style',
    web: 'settings',
    mobile: '/(app)/(tabs)/(settings)/coaching-style',
  },
  {
    id: 'messaging',
    label: 'Messaging',
    web: 'settings',
    mobile: '/(app)/(tabs)/(settings)/messaging',
  },
  {
    id: 'ai-provider',
    label: 'AI Provider',
    web: 'settings',
    mobile: '/(app)/(tabs)/(settings)/ai-provider',
  },
  {
    id: 'connected-apps',
    label: 'Connected Apps',
    web: 'settings',
    mobile: '/(app)/(tabs)/(settings)/connected-apps',
  },
  {
    id: 'privacy',
    label: 'Privacy',
    web: 'settings',
    mobile: '/(app)/(tabs)/(social)/social-settings',
  },
  { id: 'memory', label: 'Memory', web: 'settings', mobile: '/(app)/memory' },

  // ---- deliberately asymmetric ----
  {
    id: 'billing',
    label: 'Billing',
    web: 'usage',
    mobile: '/(app)/billing',
    why: 'Both gate on BILLING_ENABLED, which ships false for the first release.',
  },
  {
    id: 'admin-console',
    label: 'Admin console',
    web: 'users',
    mobile: null,
    why: 'Operator surface. Deliberately web-only — there is no mobile operator workflow.',
  },
] as const;

/** Surfaces a platform is expected to implement (i.e. not deliberately absent). */
export function surfacesFor(platform: 'web' | 'mobile'): UserSurface[] {
  return USER_SURFACES.filter((s) => s[platform] !== null);
}
