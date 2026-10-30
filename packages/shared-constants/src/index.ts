// ABOUTME: Main entry point for @pierre/shared-constants package
// ABOUTME: Re-exports all shared constants for convenient importing

// Onboarding step registry (pure decision logic shared by web + mobile)
export * from './onboarding';
export * from './brands';

// Agent tuning bounds (mirrors the server-side range checks)
export {
  MIN_MAX_TOOL_ITERATIONS,
  MAX_MAX_TOOL_ITERATIONS,
  DEFAULT_MAX_TOOL_ITERATIONS,
  COACH_CATEGORY_LABEL_KEY,
  coachCategoryLabelKey,
  COACH_TOOL_TAG,
  isCoachFacing,
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
  BORDER_INK,
  ghostBorder,
  PRIMARY_HOVER,
  SEMANTIC_COLORS,
  SEMANTIC_COLORS_DARK,
  CONTAINER_INKS,
  CONTAINER_INKS_DARK,
  PROVIDER_COLORS,
  PROVIDER_GLYPH_INK,
  providerGlyphInk,
  GRADIENT_COLORS,
  AMBIENT_SHADOW,
  AI_GLOW,
  BUTTON_GLOW,
  TYPOGRAPHY,
  BRAND_TRACKING,
  SPACING,
  BORDER_RADIUS,
  FONT_WEIGHT,
  DESIGN_SYSTEM,
} from './design-system';

export type { DesignSystem, BorealTokens, ColorScheme, ProviderGlyphInk } from './design-system';

// Notification constants (category metadata, time formatting)
export {
  NOTIFICATION_CATEGORY_COLORS,
  NOTIFICATION_CATEGORY_META,
  NOTIFICATION_CATEGORIES,
  NOTIFICATION_CATEGORY_BLURB_KEYS,
  NOTIFICATION_MAX_PER_DAY_CHOICES,
  NOTIFICATION_QUIET_HOUR_VALUES,
  defaultNotificationPreference,
  localTimezone,
  notificationCapLabel,
  mergeNotificationPreferences,
  notificationPreferenceUpdate,
  formatNotificationTime,
  formatCollapsedCount,
} from './notifications';

export type { NotificationCategoryColors, NotificationCategoryMeta } from './notifications';

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
export { USER_SURFACES, surfaceById, surfacesFor, webNavLabels } from './surfaces';
export type { DestinationRoutes, UserSurface } from './surfaces';

// Settings menu grouping (which panes, in what order, holding what)
export {
  SETTINGS_PANES,
  ADMIN_HIDDEN_PANES,
  settingsPanesFor,
  settingsPane,
  settingsPaneSections,
  settingsPaneWebRoute,
  settingsSectionScreen,
  HELP_URL,
  LEGAL_URL,
  SITE_URL,
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
  MOBILE_THREAD_INFO_GROUP,
  MOBILE_THREAD_INFO_PARAM,
  MOBILE_THREAD_PATHNAME,
  WEB_THREAD_INFO_SUBVIEW,
  resolveNotificationDestination,
  webNotificationRoute,
  mobileNotificationTarget,
} from './notification-routing';
export type { NotificationDestination, NotificationNavTarget } from './notification-routing';

// Memory fact kinds (the server's FactKind wire values and their label keys)
export { MEMORY_KIND_LABEL_KEY, memoryFactCountLabel } from './memory';

// Billing vocabulary both plan pages print (tier names, plan rows, quota
// counters and provider statuses as corpus keys, and the subscription statuses
// that ask the athlete to fix their payment)
export {
  PLAN_TIER_LABEL_KEY,
  planTierLabelKey,
  PLAN_LIMIT_ROWS,
  PLAN_INCLUDED_USAGE_LABEL_KEY,
  PLAN_PER_MONTH_KEY,
  PLAN_UNLIMITED_KEY,
  PLAN_INCLUDED_CUSTOM_KEY,
  quotaCounterLabelKey,
  subscriptionStatusLabelKey,
  invoiceStatusLabelKey,
  billingLabel,
  PAYMENT_PROBLEM_STATUSES,
  hasPaymentProblem,
} from './billing';

// Provider capability scopes (the wire slugs a provider card lists, and the
// catalogue key naming each one)
export {
  AI_CONSENT_KEYS,
  PROVIDER_NOTICES,
  PROVIDER_SCOPES,
  PROVIDER_SCOPE_LABEL_KEY,
  SCIOTTE_CODE_REJECTED,
  SCIOTTE_LOGIN_CHALLENGE,
  SCIOTTE_LOGIN_FLOW_EXPIRED,
  SCIOTTE_LOGIN_PRESETS,
  aiConsentCards,
  intervalsConnectPath,
  loginIdentifierField,
  noticeRequired,
  providerScopeLabelKey,
  sciotteTargetForBackend,
  syncAuthorizationOwed,
} from './providers';
export type {
  AiConsentCard,
  IntervalsConnectCard,
  IntervalsConnectPath,
  LoginIdentifier,
  LoginIdentifierField,
  ProviderNoticeKeys,
  ProviderScope,
  SciotteLoginPreset,
} from './providers';

// React Query keys (for consistent cache key management)
export { QUERY_KEYS } from './query-keys';
export type { QueryKeys } from './query-keys';

// The focus/idle contract both clients obey — when a client may talk to the
// server, and when it must stop because nobody is driving it.
export {
  IDLE_STOP_AFTER_MS,
  PROVIDER_LINK_POLL_INTERVAL_MS,
  CHANNEL_LINK_POLL_INTERVAL_MS,
  HOME_STALE_REFETCH_DELAYS_MS,
  HOME_ROUTE_UNAVAILABLE_RECHECK_DELAYS_MS,
  HOME_ROUTE_PENDING_BACKOFF_MS,
  HOME_ROUTE_PENDING_SLACK_MS,
  QUERY_FOCUS_POLICY,
  IdleWatch,
} from './query-policy';
export type { IdleWatchOptions } from './query-policy';

// The send path's side of that contract: the abort signal an open turn rides,
// and the registered watch it holds active and asks whether the athlete left.
export {
  idleSignal,
  idleAbort,
  resetIdleAbort,
  registerIdleWatch,
  holdIdleWhileBusy,
  trackAbsence,
  whenAthleteReturns,
} from './idle-signal';

// Claim-verdict vocabulary: the status, evidence and category words both chat surfaces
// print, as corpus keys resolved with each client's own t()
export {
  VERDICT_STATUS_LABEL_KEY,
  EVIDENCE_STRENGTH_LABEL_KEY,
  VERDICT_CATEGORY_LABEL_KEY,
  VERDICT_CATEGORY_OTHER_KEY,
  verdictCategoryLabelKey,
  VERDICT_COUNT_LABEL_KEY,
  VERDICT_CHIP_SEPARATOR,
  VERDICT_SPOKEN_SEPARATOR,
  verdictChipLabel,
} from './verdicts';

// Group health flags and the weekly report: the server sends evidence and
// stats, and both clients phrase them through these keys with their own t()
export {
  HEALTH_FLAG_DETAIL_KEY,
  WEEKLY_REPORT_KEY,
  signedWhole,
  oneDecimal,
  healthFlagDetail,
  healthFlagConcern,
  freshMemberLine,
  analyticsScopeLine,
  weeklyReportSummary,
  weeklyReportRecommendations,
  weeklyVolume,
  weeklyVolumeFigure,
} from './groups';

// Coaching-platform link vocabulary: each refusal the group's link routes
// send and each consent a member gives their coach, as corpus keys resolved
// with each client's own t(), and the platform name those keys carry
export {
  coachPlatformName,
  DELEGATION_REFUSAL_KEY,
  DELEGATION_ACTION_FAILED_KEY,
  DELEGATION_CONNECTION_REFUSALS,
  isDelegationRefusal,
  delegationRefusalKey,
  DELEGATION_CONSENT_KEYS,
  DELEGATION_CONSENTS_NONE_KEY,
  WELLNESS_READ_SKIP_KEYS,
  offeredConsents,
  consentStateKeys,
  consentNoteKey,
  confirmConsents,
  DELEGATION_REVOKE_CONFIRM_KEYS,
  consentChangeConfirm,
  COACH_CALENDAR_PLATFORM,
  COACH_CONSENTS_KEYS,
} from './delegation';
export type { DelegationConsent, ConsentChangeConfirmKeys } from './delegation';

// The map layers both route cards draw over, and the ink the track is drawn in
export {
  BASEMAP_STYLE,
  DEFAULT_MAP_LAYER,
  MAP_LAYERS,
  ROUTE_INK,
  localizeBasemapStyle,
  localizedLabel,
  mapLayerStyle,
} from './route-map';
export type {
  BasemapStyleDocument,
  MapLayer,
  MapLayerLabelKey,
  RasterMapLayer,
  RasterStyle,
  StyleMapLayer,
} from './route-map';

// The line Garmin's brand guidelines owe a figure derived from Garmin-sourced data
export { garminContributionLine } from './garmin-attribution';

// The Home training status: form band names, the form line, load ratio and recovery days
export {
  FORM_BAND_DETAIL_KEY,
  FORM_BAND_LABEL_KEY,
  TRAINING_STATUS_KEY,
  formLine,
  loadRatioLine,
  recoveryLine,
  trendDayLine,
  trendLabelLine,
  trendSpanDays,
} from './training-status';

// The Home weekly volume: the card's keys, the activity count, the trend's label
export {
  ALL_SPORTS,
  TRAINING_VOLUME_KEY,
  activitiesLine,
  volumeChartLabel,
  weekMinutesSeconds,
} from './training-volume';

// What every unread and notification badge prints for a count
export { badgeLabel } from './badge';

// The athlete's own API keys: the window each key's usage line counts over
export { API_KEY_USAGE_WINDOW_DAYS } from './api-keys';

// The catalogue key naming each account role (Account pane, sidebar badge)
export { ACCOUNT_ROLE_LABEL_KEY } from './roles';

// Singular or plural catalogue key for a count, per locale (no Intl.PluralRules)
export { countKey } from '@pierre/chat-utils';
export type { CountKeys } from '@pierre/chat-utils';

// The singular and plural keys of every count the clients print
export {
  A2A_TOTAL_APPS_KEYS,
  ACTIVE_TOKEN_COUNT_KEYS,
  API_KEY_REQUESTS_KEYS,
  BUDGET_MINUTES_KEYS,
  BUDGET_SECONDS_KEYS,
  CLAIM_VERDICT_SUMMARY_KEYS,
  CLAIM_VERDICT_SUMMARY_STATUS_ONLY_KEYS,
  COLLAPSED_SIMILAR_KEYS,
  GEAR_ACTIVITY_COUNT_KEYS,
  IMPORT_COMPLETED_KEYS,
  IMPORT_DELETE_CONFIRM_ACTION_KEYS,
  IMPORT_DELETE_CONFIRM_BODY_KEYS,
  IMPORT_HISTORY_LINE_KEYS,
  IMPORT_HISTORY_ON_CARD_KEYS,
  IMPORT_STOPPED_KEYS,
  INSTALL_COUNT_KEYS,
  MEMORY_FACT_COUNT_KEYS,
  NOTIFICATIONS_BELL_UNREAD_KEYS,
  NOTIFICATIONS_UNREAD_KEYS,
  ONBOARDING_WINDOW_SUMMARY_KEYS,
  OPEN_ROW_UNREAD_KEYS,
  PER_DAY_KEYS,
  PLAN_HARD_PER_WEEK_KEYS,
  PLAN_MORE_WEEKS_KEYS,
  PROVIDERS_CONNECTED_KEYS,
  UNREAD_COUNT_ARIA_KEYS,
  WEEK_ACTIVITY_COUNT_KEYS,
  WEEK_CORRECTED_KEYS,
  WEEK_FLAGGED_KEYS,
} from './count-keys';

// The chat header's fallback line when there is no group and no handle to name:
// which providers the coach can see, or that none are connected.
export { providerStatusLine } from './provider-status';
export type { ProviderStatusRow } from './provider-status';
