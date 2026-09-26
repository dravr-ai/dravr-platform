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
  describeLoginFailure,
  refusalReason,
} from './apiError';

// Server-state hooks both clients bind to their own API instance. Each client
// keeps a thin `hooks/<name>` module that calls the factory once; only what is
// genuinely platform-specific (the API instance, a freshness the two clients
// set differently, how a key event is read) stays there.
export { type GroupFreshness, createGroupHooks } from './groupHooks';
export { type UnreadCountPolling, createNotificationHooks } from './notificationHooks';
export { type UseFeatureFlagsResult, createFeatureFlagsHook } from './featureFlagsHook';

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
