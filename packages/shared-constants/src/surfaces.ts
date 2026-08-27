// ABOUTME: Single registry of the user-facing surfaces Dravr offers, per platform
// ABOUTME: Each client asserts it implements this list — the registry is the source, not a mirror

import { SURFACE_CAPABILITIES, type ReplyBlockKind } from './surface-capabilities.generated';

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
  /**
   * Sidebar label a regular user clicks to reach this surface on web, or null
   * when it is not a top-level web destination — reached through the settings
   * gear, or operator-only.
   *
   * Not decoration: the design sweep walks exactly these, and it used to walk a
   * third hand-written list of its own that nothing kept in step with either
   * this registry or the sidebar.
   */
  webNav: string | null;
  /**
   * Reply-block kinds this surface renders, from the generated server
   * catalogue.
   *
   * Empty for every surface that carries no coach reply — a settings screen
   * renders no turn envelope. Only the chat surface has a non-empty column,
   * and it is read out of the catalogue rather than typed here, so a surface
   * cannot claim an affordance the server never sends it.
   */
  blocks: readonly ReplyBlockKind[];
  /** Required when either side is null: why that platform does not have it. */
  why?: string;
}

/**
 * What the in-app chat renders, straight from the generated catalogue.
 *
 * One column for both clients because both resolve the same server-side
 * capabilities. That is proven rather than assumed: `SurfaceParity.test.ts` in
 * each client asserts the catalogue's `web_chat` and `mobile_chat` rows list
 * the same blocks, so the day they diverge the assertion fails instead of this
 * constant quietly describing only one of them.
 */
const CHAT_BLOCKS: readonly ReplyBlockKind[] = SURFACE_CAPABILITIES.web_chat.blocks;

/** A surface that carries no coach reply renders no reply blocks. */
const NO_BLOCKS: readonly ReplyBlockKind[] = [];

export const USER_SURFACES: readonly UserSurface[] = [
  // ---- primary destinations ----
  {
    id: 'chat',
    label: 'Chat',
    web: 'chat',
    mobile: '/(app)/(tabs)/(chat)',
    webNav: 'Chat',
    blocks: CHAT_BLOCKS,
  },
  {
    id: 'discover',
    label: 'Discover',
    web: 'discover',
    mobile: '/(app)/(tabs)/(discover)',
    webNav: 'Discover',
    blocks: NO_BLOCKS,
  },
  {
    id: 'notifications',
    label: 'Notifications',
    web: 'notifications',
    mobile: '/(app)/notifications',
    webNav: 'Notifications',
    blocks: NO_BLOCKS,
  },

  // ---- account & settings ----
  {
    id: 'profile',
    label: 'Profile',
    web: 'settings',
    mobile: '/(app)/(tabs)/(settings)/profile',
    webNav: null,
    blocks: NO_BLOCKS,
  },
  {
    id: 'data-providers',
    label: 'Data Providers',
    web: 'data-providers',
    mobile: '/(app)/(tabs)/(settings)/connections',
    webNav: 'Data Providers',
    blocks: NO_BLOCKS,
  },
  {
    id: 'coaching-style',
    label: 'Coaching Style',
    web: 'settings',
    mobile: '/(app)/(tabs)/(settings)/coaching-style',
    webNav: null,
    blocks: NO_BLOCKS,
  },
  {
    id: 'messaging',
    label: 'Messaging',
    web: 'settings',
    mobile: '/(app)/(tabs)/(settings)/messaging',
    webNav: null,
    blocks: NO_BLOCKS,
  },
  {
    id: 'ai-provider',
    label: 'AI Provider',
    web: 'settings',
    mobile: '/(app)/(tabs)/(settings)/ai-provider',
    webNav: null,
    blocks: NO_BLOCKS,
  },
  {
    id: 'connected-apps',
    label: 'Connected Apps',
    web: 'settings',
    mobile: '/(app)/(tabs)/(settings)/connected-apps',
    webNav: null,
    blocks: NO_BLOCKS,
  },
  {
    id: 'privacy',
    label: 'Privacy',
    web: 'settings',
    mobile: '/(app)/(tabs)/(settings)/privacy',
    webNav: null,
    blocks: NO_BLOCKS,
  },
  {
    id: 'memory',
    label: 'Memory',
    web: 'settings',
    mobile: '/(app)/memory',
    webNav: null,
    blocks: NO_BLOCKS,
  },

  // ---- deliberately asymmetric ----
  {
    id: 'billing',
    label: 'Billing',
    web: 'usage',
    mobile: '/(app)/billing',
    webNav: 'Usage',
    blocks: NO_BLOCKS,
    why: 'Both gate on BILLING_ENABLED, which ships false for the first release.',
  },
  {
    id: 'admin-console',
    label: 'Admin console',
    web: 'users',
    mobile: null,
    webNav: null,
    blocks: NO_BLOCKS,
    why: 'Operator surface. Deliberately web-only — there is no mobile operator workflow. The design sweep walks it with the other admin surfaces, not the athlete ones.',
  },
] as const;

/** Surfaces a platform is expected to implement (i.e. not deliberately absent). */
export function surfacesFor(platform: 'web' | 'mobile'): UserSurface[] {
  return USER_SURFACES.filter((s) => s[platform] !== null);
}

/**
 * The web sidebar labels a regular user can click, in registry order.
 *
 * The design sweep walks these. It used to carry its own copy of the list —
 * a third surface declaration beside this registry and the sidebar itself —
 * which is how it kept screenshotting eight surfaces while the product had
 * grown past them.
 */
export function webNavLabels(): string[] {
  return USER_SURFACES.map((s) => s.webNav).filter((label): label is string => label !== null);
}
