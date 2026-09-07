// ABOUTME: Main entry point for @pierre/shared-constants package
// ABOUTME: Re-exports all shared constants for convenient importing

// Onboarding step registry (pure decision logic shared by web + mobile)
export * from './onboarding';
export * from './brands';

// Coach tuning bounds (mirrors the server-side range checks)
export {
  MIN_MAX_TOOL_ITERATIONS,
  MAX_MAX_TOOL_ITERATIONS,
  DEFAULT_MAX_TOOL_ITERATIONS,
  COACH_CATEGORY_LABEL_KEY,
  coachCategoryLabelKey,
} from './coaches';

// Design system (Boreal Editorial colors, typography, spacing, effects)
export {
  BOREAL_LIGHT,
  BOREAL_DARK,
  BOREAL,
  PIERRE_COLORS,
  PILLAR_COLORS,
  PRIMARY_PALETTE,
  SURFACE_HIERARCHY,
  BACKGROUND_COLORS,
  TEXT_COLORS,
  BORDER_COLORS,
  SEMANTIC_COLORS,
  SEMANTIC_COLORS_DARK,
  CONTAINER_INKS,
  CONTAINER_INKS_DARK,
  PROVIDER_COLORS,
  GRADIENT_COLORS,
  AMBIENT_SHADOW,
  AI_GLOW,
  BUTTON_GLOW,
  TYPOGRAPHY,
  BRAND_TRACKING,
  SPACING,
  BORDER_RADIUS,
  FONT_SIZE,
  FONT_WEIGHT,
  DESIGN_SYSTEM,
} from './design-system';

export type { DesignSystem, BorealTokens, ColorScheme } from './design-system';

// Notification constants (category metadata, time formatting)
export {
  NOTIFICATION_CATEGORY_META,
  NOTIFICATION_CATEGORIES,
  NOTIFICATION_MAX_PER_DAY_CHOICES,
  defaultNotificationPreference,
  mergeNotificationPreferences,
  notificationPreferenceUpdate,
  formatNotificationTime,
  formatCollapsedCount,
} from './notifications';

export type { NotificationCategoryMeta } from './notifications';

// Slash-command palette matching (one rule for both composers), and the
// command drafts every client affordance hands to the composer
export {
  COMMAND_PREFIX,
  COMMAND_DRAFTS,
  SLASH_HINT_KEY,
  isCommandDraft,
  matchCommands,
  commandDraftFor,
  COMMAND_DOMAIN_LABEL_KEY,
  commandDomainLabelKey,
} from './commands';

// @handle mention grammar (one rule for both composers, mirroring the server scanner)
export {
  MENTION_PREFIX,
  mentionDraftAt,
  matchMentionCoaches,
  insertMention,
} from './mentions';
export type { MentionDraft, MentionCandidate } from './mentions';

// User-facing surface registry (what the product offers, and where per platform)
export { USER_SURFACES, surfaceById, surfacesFor, webNavLabels, webRouteFor } from './surfaces';
export type { UserSurface } from './surfaces';

// Settings menu grouping (which panes, in what order, holding what)
export {
  SETTINGS_PANES,
  ADMIN_HIDDEN_PANES,
  settingsPanesFor,
  settingsPane,
  settingsPaneSections,
  HELP_URL,
  LEGAL_URL,
  APP_VERSION,
} from './surfaces';
export type { SettingsPane, SettingsPaneId, SettingsSectionId } from './surfaces';

// Generated from the server's own capability table: what each chat surface
// renders, and the notification screen vocabulary. Never hand-edited —
// regenerate with `bun run generate` from this package.
export {
  REPLY_BLOCK_KINDS,
  SURFACE_CAPABILITY_IDS,
  SURFACE_CAPABILITIES,
  NOTIFICATION_SCREEN_SURFACES,
} from './surface-capabilities.generated';
export type {
  ReplyBlockKind,
  SurfaceCapabilities,
  SurfaceCapabilityId,
  NotificationScreen,
} from './surface-capabilities.generated';

// Notification → destination, resolved once for both platforms.
export {
  resolveNotificationDestination,
  webNotificationRoute,
  mobileNotificationTarget,
} from './notification-routing';
export type { NotificationDestination, NotificationNavTarget } from './notification-routing';

// Memory fact kinds (the server's FactKind wire values and their label keys)
export { MEMORY_KIND_LABEL_KEY } from './memory';

// Provider capability scopes (the wire slugs a provider card lists, and the
// catalogue key naming each one)
export { PROVIDER_SCOPES, PROVIDER_SCOPE_LABEL_KEY, providerScopeLabelKey } from './providers';
export type { ProviderScope } from './providers';

// React Query keys (for consistent cache key management)
export { QUERY_KEYS } from './query-keys';
export type { QueryKeys } from './query-keys';

// The focus/idle contract both clients obey — when a client may talk to the
// server, and when it must stop because nobody is driving it.
export {
  IDLE_STOP_AFTER_MS,
  PROVIDER_LINK_POLL_INTERVAL_MS,
  CHANNEL_LINK_POLL_INTERVAL_MS,
  QUERY_FOCUS_POLICY,
  IdleWatch,
} from './query-policy';
export type { IdleWatchOptions } from './query-policy';

// Claim-verdict vocabulary: the status and evidence words both chat surfaces
// print, as corpus keys resolved with each client's own t()
export {
  VERDICT_STATUS_LABEL_KEY,
  EVIDENCE_STRENGTH_LABEL_KEY,
  VERDICT_CHIP_ONE_KEY,
  VERDICT_CHIP_N_KEY,
  verdictChipLabel,
} from './verdicts';
