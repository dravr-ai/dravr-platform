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

// Server-state hooks both clients bind to their own API instance. Each client
// keeps a thin `hooks/<name>` module that calls the factory once; only what is
// genuinely platform-specific (the API instance, a freshness the two clients
// set differently, how a key event is read) stays there.
export { type GroupFreshness, createGroupHooks } from './groupHooks';
export { type UnreadCountPolling, createNotificationHooks } from './notificationHooks';
export { type UseFeatureFlagsResult, createFeatureFlagsHook } from './featureFlagsHook';
export { type UseHomePreferencesResult, createHomePreferencesHook } from './homePreferencesHook';
// The athlete's units: the Settings hook, and the context every distance a surface prints reads (carnet#835).
export {
  type UnitsAutomaticHint,
  type UseUnitPreferencesResult,
  UNIT_PREFERENCE_OPTIONS,
  UnitsContext,
  createUnitPreferencesHook,
  unitsAutomaticHint,
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

// Home route reads: followed through the server's `pending` answers.
export { readActivityRoute } from './activityRoute';

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
