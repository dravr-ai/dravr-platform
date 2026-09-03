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
    webNav: null,
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

/**
 * A section inside a settings pane.
 *
 * Only panes that group several things name their sections; a pane that is a
 * single destination has none. The ids are what each client tags its rendered
 * block with, so the grouping is checkable rather than described.
 */
export type SettingsSectionId =
  | 'account-status'
  | 'usage'
  | 'security'
  | 'connected-mcp-apps'
  | 'sign-out'
  | 'version'
  | 'coach-model'
  | 'help'
  | 'legal';

/** The settings panes, by id. */
export type SettingsPaneId =
  | 'profile'
  | 'connections'
  | 'tokens'
  | 'coaching'
  | 'messaging'
  | 'notifications'
  | 'memory'
  | 'privacy'
  | 'about'
  | 'account'
  | 'billing';

/** One named settings destination, and what it holds. */
export interface SettingsPane {
  id: SettingsPaneId;
  /** Corpus key of the pane's name, rendered by both clients. */
  nameKey: string;
  /** Corpus key of the one-line hint under the name. */
  hintKey: string;
  /** Web: the `activeTab` id, or null when web serves it elsewhere. */
  web: SettingsPaneId | null;
  /** Mobile: the expo-router path, or null when mobile serves it elsewhere. */
  mobile: string | null;
  /**
   * The sections the pane groups, in render order. Absent when the pane is a
   * single destination with nothing to order.
   */
  holds?: readonly SettingsSectionId[];
  /**
   * The gate the pane rides on: the `api_tokens` server feature flag, the
   * build-time billing toggle, or nothing.
   */
  flag: 'api_tokens' | 'billing' | null;
  /** Required when either side is null: why that platform lacks the pane. */
  why?: string;
}

/**
 * Every settings pane, in menu order — the one declaration of how settings are
 * grouped, read by the web tab rail and the mobile settings list alike.
 *
 * Distinct from {@link USER_SURFACES}, which answers a different question:
 * that registry says whether a destination EXISTS on a platform, this one says
 * what the settings menu LISTS, in what order, under which name, holding what.
 * They overlap on the settings destinations by design; neither derives the
 * other, because a surface can exist without being a pane — `connected-apps`
 * is a screen of its own on mobile and a section of the Account pane on web.
 *
 * It exists because the grouping drifted with nothing to catch it: usage sat
 * inside Account on web and stood alone on mobile, MCP apps likewise, and the
 * mobile app served the whole lot as one 1,200pt scroll while web served ten
 * named panes. `holds` is what pins that — the two clients render the same
 * sections, in the same pane, in the same order.
 */
export const SETTINGS_PANES: readonly SettingsPane[] = [
  {
    id: 'profile',
    nameKey: 'settingsTabs.profile',
    hintKey: 'settingsTabs.profileHint',
    web: 'profile',
    mobile: '/(app)/(tabs)/(settings)/profile',
    flag: null,
  },
  {
    id: 'connections',
    nameKey: 'settingsTabs.connections',
    hintKey: 'settingsTabs.connectionsHint',
    web: 'connections',
    mobile: '/(app)/(tabs)/(settings)/connections',
    flag: null,
  },
  {
    id: 'tokens',
    nameKey: 'settingsTabs.tokens',
    hintKey: 'settingsTabs.tokensHint',
    web: 'tokens',
    mobile: '/(app)/(tabs)/(settings)/tokens',
    flag: 'api_tokens',
  },
  {
    id: 'coaching',
    nameKey: 'settingsTabs.coaching',
    hintKey: 'settingsTabs.coachingHint',
    web: 'coaching',
    mobile: '/(app)/(tabs)/(settings)/coaching-style',
    flag: null,
  },
  {
    id: 'messaging',
    nameKey: 'settingsTabs.messaging',
    hintKey: 'settingsTabs.messagingHint',
    web: 'messaging',
    mobile: '/(app)/(tabs)/(settings)/messaging',
    flag: null,
  },
  {
    id: 'notifications',
    nameKey: 'settingsTabs.notifications',
    hintKey: 'settingsTabs.notificationsHint',
    web: 'notifications',
    mobile: '/(app)/(tabs)/(settings)/notification-preferences',
    flag: null,
  },
  {
    id: 'memory',
    nameKey: 'settingsTabs.memory',
    hintKey: 'shell.memoryTitle',
    web: 'memory',
    mobile: '/(app)/memory',
    flag: null,
  },
  {
    id: 'privacy',
    nameKey: 'settingsTabs.privacy',
    hintKey: 'settingsTabs.privacyHint',
    web: 'privacy',
    mobile: '/(app)/(tabs)/(settings)/privacy',
    flag: null,
  },
  {
    id: 'about',
    nameKey: 'settingsTabs.about',
    hintKey: 'settingsTabs.aboutHint',
    web: 'about',
    mobile: '/(app)/(tabs)/(settings)/about',
    holds: ['version', 'coach-model', 'help', 'legal'],
    flag: null,
  },
  {
    id: 'account',
    nameKey: 'settingsTabs.account',
    hintKey: 'settingsTabs.accountHint',
    web: 'account',
    mobile: '/(app)/(tabs)/(settings)/account',
    holds: ['account-status', 'usage', 'security', 'connected-mcp-apps', 'sign-out'],
    flag: null,
  },
  {
    id: 'billing',
    nameKey: 'app.billing',
    hintKey: 'app.planAndUsage',
    web: null,
    mobile: '/(app)/billing',
    flag: 'billing',
    why: 'Web serves plan and usage from the Usage destination in the sidebar, which the settings menu does not duplicate.',
  },
];

/**
 * Panes an operator does not get: provider connections, messaging and About
 * are athlete-account surfaces. Gated on `role`, as the Dashboard gates.
 */
export const ADMIN_HIDDEN_PANES: ReadonlySet<SettingsPaneId> = new Set([
  'connections',
  'about',
  'messaging',
]);

/** The panes a platform lists in its settings menu, in menu order. */
export function settingsPanesFor(platform: 'web' | 'mobile'): SettingsPane[] {
  return SETTINGS_PANES.filter((pane) => pane[platform] !== null);
}

/** The pane with this id. */
export function settingsPane(id: SettingsPaneId): SettingsPane {
  const pane = SETTINGS_PANES.find((candidate) => candidate.id === id);
  if (!pane) {
    throw new Error(`No settings pane declared for id "${id}"`);
  }
  return pane;
}

/**
 * The sections a pane groups, in render order — empty for a pane that is a
 * single destination. Both clients render from this, so a section moved on one
 * of them without moving here shows up as a missing block rather than as a
 * layout nobody compared.
 */
export function settingsPaneSections(id: SettingsPaneId): readonly SettingsSectionId[] {
  return settingsPane(id).holds ?? [];
}

/**
 * Where the About pane's help and legal rows go.
 *
 * One address for both clients. The marketing site's help, privacy and terms
 * paths each answer 404, and a per-client copy of the destination is how the
 * same dead link ships twice.
 */
export const HELP_URL = 'https://dravr.ai/docs';
export const LEGAL_URL = 'https://dravr.ai/docs';

/** The release both clients report in their About pane. */
export const APP_VERSION = '1.0.0';
