// ABOUTME: Main entry point for @pierre/ui-logic package
// ABOUTME: Re-exports the headless UI hooks and the server-state hooks both clients bind

// Button hook
export {
  type ButtonVariant,
  type ButtonSize,
  type UseButtonProps,
  type UseButtonReturn,
  useButton,
} from './useButton';

// Async action hook
export {
  type AsyncActionState,
  type UseAsyncActionProps,
  type UseAsyncActionReturn,
  useAsyncAction,
} from './useAsyncAction';

// A chat turn's trail of steps and its ticking clock, held for one thread
export {
  type StreamedReplyCallbacks,
  type TurnTrailState,
  streamedReplyCallbacks,
  useTurnTrail,
  useTurnElapsedMs,
} from './useTurnTrail';

// Toast hook
export {
  type ToastType,
  type ToastItem,
  type AddToastProps,
  type UseToastProps,
  type UseToastReturn,
  useToast,
} from './useToast';

// Form field hook and validators
export {
  type ValidationResult,
  type Validator,
  type UseFormFieldProps,
  type UseFormFieldReturn,
  useFormField,
  // Common validators
  required,
  minLength,
  maxLength,
  email,
  range,
  pattern,
} from './useFormField';

// Modal hooks
export {
  type UseModalProps,
  type UseModalReturn,
  useModal,
  type UseConfirmDialogProps,
  type UseConfirmDialogReturn,
  useConfirmDialog,
} from './useModal';
export { useAbortOnClose } from './useAbortOnClose';

// API error classification — one classifier for web and mobile, so a failed
// request is named by its transport state and status, never by matching on
// the server's own prose.
export {
  type ApiErrorKind,
  type ApiErrorTranslate,
  type ClassifiedApiError,
  classifyApiError,
  API_ERROR_KEYS,
  prefersServerDetail,
  describeApiError,
  describeQuotaRefusal,
  describeSignInFailure,
  type SignInFailure,
  refusalReason,
  INVALID_SIGNUP_CODE_REASON,
  isInvalidSignupCode,
  refusalProvider,
} from './apiError';
export { describeTurnFailure, isTurnFailureRetryable } from './turnFailure';

// Self-serve account deletion: the confirmation rule and the refusal wording.
export {
  type AccountDeletionFailure,
  emailConfirms,
  describeAccountDeletionBlocker,
  describeAccountDeletionFailure,
} from './accountDeletion';
// What both clients say about an import of the athlete's own data
export type { CopyLine, ImportPhase } from './archiveImport';
export {
  AWAITING_LINK_LATE_MS,
  IMPORT_POLL_INTERVAL_MS,
  IMPORT_RESULT_STORAGE_KEY,
  importDate,
  importDeleteCopy,
  importFailureKey,
  importHistoryCopy,
  importOutcomeKey,
  importPhase,
  importSize,
  importStateCopy,
  linkRefusalKey,
  startErrorKey,
} from './archiveImport';

export type { ImportTombstoneRefusal } from './importTombstone';
export { describeRegistration } from './registration';
export {
  importTombstoneRefusal,
  importTombstoneFromReturn,
  describeProviderTombstone,
  describeEmailTombstone,
} from './importTombstone';

// Server-state hooks both clients bind to their own API instance. Each client
// keeps a thin `hooks/<name>` module that calls the factory once; only what is
// genuinely platform-specific (the API instance, a freshness the two clients
// set differently, how a key event is read) stays there.
export { type GroupFreshness, createGroupHooks } from './groupHooks';
// The coach-group onboarding sequence: group, thread, then the athlete invite, resumed on a retry.
export {
  type CoachGroupSetupApis,
  type UseCoachGroupSetupResult,
  createCoachGroupSetupHook,
} from './coachGroupSetupHook';
// A coach's plan thread for one athlete: opening it from Group info, and the draft card's Publish and Discard (carnet#1069/#1070),
// and the athlete's live plan read-only from Group info (carnet#1244).
export {
  type AthletePlanRead,
  type CoachPlanApis,
  type PlanDraftActionResult,
  type PlanDraftCardState,
  type UseAthletePlanResult,
  type UseOpenAthleteThreadResult,
  type UsePlanDraftCardResult,
  createCoachPlanHooks,
  openThreadRefusalText,
} from './coachPlanHooks';
// A coach's private lane in a one-athlete room: the Everyone / Only <agent> switch, the work
// thread's rows drawn in the room, "Adjust plan from this" and the private send (carnet#1249).
export {
  type AudienceStore,
  type PrivateLaneApis,
  type PrivateSendOptions,
  type PrivateSendOutcome,
  type UsePrivateLaneInput,
  type UsePrivateLaneResult,
  createPrivateLaneHooks,
  privateSendFailureText,
} from './privateLaneHooks';
export { type UnreadCountPolling, createNotificationHooks } from './notificationHooks';
export { type UseFeatureFlagsResult, createFeatureFlagsHook } from './featureFlagsHook';
export { type UseHomePreferencesResult, createHomePreferencesHook } from './homePreferencesHook';
// Consent to AI use of one provider's data: the privacy toggle and the connection card share it (carnet#726).
export { type AiConsentChange, createAiConsentHook } from './aiConsentHook';
// How often the coach checks in on its own: the onboarding step and Settings write it (carnet#831).
export { type UseCheckInResult, CHECK_IN_OPTIONS, createCheckInHook } from './checkInHook';
// The athlete's units, metric unless Settings chose imperial, and the context every distance a surface prints reads (carnet#835).
export {
  type UseUnitPreferencesResult,
  UNIT_SYSTEM_OPTIONS,
  UnitsContext,
  createUnitPreferencesHook,
  useDistanceUnit,
} from './unitsHook';
export {
  type ActivityUploadFailure,
  type ActivityUploadOutcome,
  type PickedActivityFile,
  type UseActivityUploadResult,
  type UseDeleteUploadedActivityResult,
  ACTIVITY_DELETE_KEYS,
  ACTIVITY_UPLOAD_KEYS,
  ACTIVITY_UPLOAD_MAX_MEGABYTES,
  activityUploadFailure,
  createActivityUploadHook,
  createDeleteUploadedActivityHook,
  describeActivityUpload,
  isDeletableActivity,
} from './activityUploadHook';

// Composer palettes. Platform-free: each composer reads its own key event and
// hands the palette the key's name.
export { PALETTE_KEYS } from './paletteKeys';
export {
  type UseCommandPaletteOptions,
  type UseCommandPaletteResult,
  createCommandPaletteHook,
} from './commandPalette';
export {
  type UseMentionPaletteOptions,
  type UseMentionPaletteResult,
  createMentionPaletteHook,
} from './mentionPalette';

// Home's reads: recent activities and their stale-answer schedule, a route, the plan, status and volume.
export {
  type ActivityRouteOptions,
  type ActivityRouteState,
  type HomeReadApi,
  type HomeReadPlatform,
  type RecentActivitiesState,
  type StaleRefetchPhase,
  createHomeReadHooks,
} from './homeReadHooks';

// One activity's view: its figures, its split and lap tables, the questions it offers.
export {
  type ActivityFigure,
  type ActivityPrompt,
  type SegmentRow,
  type SegmentTable,
  type SpeedForm,
  ACTIVITY_PROMPTS,
  type AskAboutTranslate,
  activityFigures,
  activityFirstLine,
  formatClock,
  formatActivityDistance,
  formatSpeed,
  lapsTable,
  speedForm,
  splitsTable,
} from './activityView';
// Home's words and figures: calendar days, instants, sports, row figures and the plan drafts, one rule on both clients.
export {
  type PlanWindow,
  type Translate,
  DRAFT_DATE,
  ROW_DATE,
  activityNaming,
  activityRowFigures,
  calendarDayLabel,
  civilDayMonth,
  civilDayOfMonth,
  civilMonthTitle,
  civilShortDate,
  civilWeekdayNarrow,
  formatCivilDate,
  formatInstant,
  formatSyncTime,
  phaseWeekLabel,
  planDayDraft,
  planDayRouteDraft,
  planWindow,
  sessionFacts,
  sportLabel,
  volumeMetricText,
} from './homeFormat';
export { type ActivityDetailState, createActivityDetailHook } from './activityDetailHook';
export { type ActivityConversationState, createActivityConversationHook } from './activityConversation';

// An agent system prompt's estimated size, worded by one catalogue sentence on both clients.
export { PROMPT_TOKEN_ESTIMATE_KEY, estimatePromptTokens } from './promptTokens';

// Home activity list: whether it is fetching, failed or settled, said once on both clients.
export {
  type RecentActivitiesSync,
  type RecentActivitiesSyncInput,
  type RequestsInFlight,
  recentActivitiesSync,
  useRequestsInFlight,
} from './recentActivitiesSync';

// Home's volume-card sport, which the activity list beside it filters on too.
export { type VolumeSportFilter, useVolumeSportFilter } from './volumeSportFilter';

// The gear ledger: the athlete's shoes, their distance, and the pair each activity was run in.
export {
  type ActivityGearState,
  type GearChanges,
  type GearDraft,
  type GearDraftCheck,
  type GearDraftErrors,
  type GearHooks,
  checkGearDraft,
  createGearHooks,
  emptyGearDraft,
  formatGearDistance,
  gearChanges,
  gearChoices,
  gearDistanceLimit,
  gearDraftOf,
  splitGear,
} from './gearLedger';

// Coach Home: the queue, the team table's order, the season's layout and the words of an evidence line, one rule on both clients.
export {
  FORM_STRIP_STEPS,
  agentCell,
  agentCellNeedsLook,
  athletesInGroup,
  buildQueue,
  buildRosterTable,
  buildSeason,
  civilDayNumber,
  civilFromDayNumber,
  countdownBars,
  daysBetween,
  daysToARace,
  describeLine,
  describeSource,
  firstQueued,
  formStripIndex,
  localToday,
  nextPhase,
  providerLetter,
  providerName,
  queueCount,
  queueWhen,
  sourceChips,
  trainingCut,
  type CoachQueue,
  type MergedRoster,
  type QueueEntry,
  type SeasonRow,
  type SeasonScale,
} from './coachHome';
export { createCoachRosterHooks } from './coachRosterHooks';
export {
  agentWords,
  formWords,
  lastSessionWords,
  lastWhenWords,
  nonSharerStatusWords,
  planWords,
  sourceChipWords,
  threadWords,
} from './coachHomeWords';
