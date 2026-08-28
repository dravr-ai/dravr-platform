// ABOUTME: Main entry point for @pierre/ui-logic package
// ABOUTME: Re-exports all headless UI hooks for shared component logic

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
  type ClassifiedApiError,
  classifyApiError,
  API_ERROR_KEYS,
  prefersServerDetail,
  describeApiError,
} from './apiError';
