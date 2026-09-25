// ABOUTME: The destinations Dravr offers, per platform: top-level surfaces and the settings panes
// ABOUTME: Each client asserts it implements both lists — the registry is the source, not a mirror

import { SURFACE_CAPABILITIES, type ReplyBlockKind } from './surface-capabilities.generated';

/**
 * The top-level surfaces of the product, and where each is reachable.
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
 *
 * Settings destinations are not rows here: each one is a pane in
 * {@link SETTINGS_PANES}, which holds its route on both platforms. The two
 * lists share one id space — {@link destinationRoutes} resolves an id in
 * either — so no id appears in both.
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
   * Empty for every surface that carries no agent reply — a settings screen
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

/** A surface that carries no agent reply renders no reply blocks. */
const NO_BLOCKS: readonly ReplyBlockKind[] = [];

export const USER_SURFACES: readonly UserSurface[] = [
  // ---- primary destinations ----
  {
    // Where an athlete lands after sign-in on both platforms, and where the
    // Dravr logo leads: today's session from the plan, the week around it,
    // and the latest activities with their routes.
    id: 'home',
    label: 'Home',
    web: 'home',
    mobile: '/(app)/(tabs)/(home)',
    webNav: 'Home',
    blocks: NO_BLOCKS,
  },
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
    // The notification feed. Its id is not `notifications`: that id is the
    // settings pane for notification preferences, and the two lists share
    // one id space.
    id: 'notification-center',
    label: 'Notifications',
    web: 'notifications',
    mobile: '/(app)/notifications',
    webNav: 'Notifications',
    blocks: NO_BLOCKS,
  },

  // ---- deliberately asymmetric ----
  {
    id: 'usage',
    label: 'Usage',
    web: 'usage',
    mobile: null,
    webNav: 'Usage',
    blocks: NO_BLOCKS,
    why: 'Mobile serves plan and usage as the billing settings pane in SETTINGS_PANES. Both gate on BILLING_ENABLED, which ships false for the first release.',
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

/** The registry row for a surface id, or null when the registry has no such row. */
export function surfaceById(id: string): UserSurface | null {
  return USER_SURFACES.find((surface) => surface.id === id) ?? null;
}

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
   * Mobile: the held sections served as a screen of their own, pushed from
   * the pane's row, by section id. Web renders every section inline, so this
   * is the one place such a screen's route is declared.
   */
  mobileScreens?: Partial<Record<SettingsSectionId, string>>;
  /**
   * The gate the pane rides on: the `api_tokens` server feature flag, the
   * build-time billing toggle, or nothing.
   */
  flag: 'api_tokens' | 'billing' | null;
  /** Required when either side is null: why that platform lacks the pane. */
  why?: string;
}

/**
 * Every settings pane, in menu order — the one declaration of the settings
 * destinations: how they are grouped, under which name, holding what, and the
 * route each platform serves them at. The web tab rail and the mobile settings
 * list both read it, and so does every link into settings — the connect-a-
 * provider banners, a provider-reauth notification.
 *
 * {@link USER_SURFACES} used to repeat the settings destinations under ids of
 * its own (`data-providers` for `connections`, `coaching-style` for
 * `coaching`, a bare `settings` web route for most of the rest), and both
 * clients navigated through both. A settings destination is declared here
 * only. A section with a phone screen of its own, as connected apps has, is
 * declared on its pane through `mobileScreens`.
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
    // LIMITATION(registre#482): the `privacy` pane has no account-deletion action on either client; deletion is by request to privacy@dravr.ai, as the published policy says.
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
    mobileScreens: { 'connected-mcp-apps': '/(app)/(tabs)/(settings)/connected-apps' },
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

/** A pane's Dashboard hash on web, or null when web serves it elsewhere. */
function paneWebRoute(pane: SettingsPane): string | null {
  return pane.web === null ? null : `settings/${pane.web}`;
}

/**
 * The web route that opens a settings pane: the `settings/<pane>` hash the
 * Dashboard parses.
 *
 * Throws for a pane web serves elsewhere: the registry is static data in this
 * monorepo, so asking for one is a wrong id, not a runtime state to branch on.
 */
export function settingsPaneWebRoute(id: SettingsPaneId): string {
  const route = paneWebRoute(settingsPane(id));
  if (route === null) {
    throw new Error(`Settings pane "${id}" has no web route`);
  }
  return route;
}

/**
 * The phone screen a held section is served at, from the pane that holds it.
 * Throws when no pane declares one, for the same reason as
 * {@link settingsPaneWebRoute}.
 */
export function settingsSectionScreen(section: SettingsSectionId): string {
  for (const pane of SETTINGS_PANES) {
    const route = pane.mobileScreens?.[section];
    if (route !== undefined) {
      return route;
    }
  }
  throw new Error(`No settings pane serves section "${section}" as a screen of its own`);
}

/** Where one destination is served on each platform; null where it is not. */
export interface DestinationRoutes {
  /** The web Dashboard route. */
  web: string | null;
  /** The mobile expo-router path. */
  mobile: string | null;
}

/**
 * The routes a destination id opens, on both platforms: a top-level surface
 * from {@link USER_SURFACES}, or a settings pane from {@link SETTINGS_PANES}.
 * Null when neither declares the id.
 *
 * The two lists share one id space — no id is declared in both — so an id
 * names exactly one row. That is what lets a server-side declaration, such as
 * the screen a notification opens, name a destination without knowing which
 * list holds it.
 */
export function destinationRoutes(id: string): DestinationRoutes | null {
  const surface = surfaceById(id);
  if (surface !== null) {
    return { web: surface.web, mobile: surface.mobile };
  }
  const pane = SETTINGS_PANES.find((candidate) => candidate.id === id);
  if (pane !== undefined) {
    return { web: paneWebRoute(pane), mobile: pane.mobile };
  }
  return null;
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
 * One address each for both clients. The marketing site has no help page, so
 * help goes to the docs hub; legal goes to the privacy policy, which links the
 * terms. A per-client copy of a destination is how the same dead link ships
 * twice, so both clients read these.
 */
export const HELP_URL = 'https://dravr.ai/docs';
export const LEGAL_URL = 'https://dravr.ai/privacy';

/** The release both clients report in their About pane. */
export const APP_VERSION = '1.0.0';
