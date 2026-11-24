// ABOUTME: UI components barrel export
// ABOUTME: Centralized exports for all reusable UI components

export { Button } from './Button';
export { Input } from './Input';
export { Checkbox } from './Checkbox';
export type { CheckboxProps } from './Checkbox';
// A form's single choice: segments for a few short words, a list for named options (never TextTabs, the filter language).
export { RadioGroup } from './RadioGroup';
export type { RadioGroupProps, RadioOption } from './RadioGroup';
// A one-tap pick in a row of words or figures: a sport, a step's target.
export { Chip } from './Chip';
export type { ChipProps } from './Chip';
// A stepper's − and +, and the short figure field they flank (a set's repetitions, a step's m:ss).
export { SquareIconButton } from './SquareIconButton';
export type { SquareIconButtonProps } from './SquareIconButton';
export { StepperField } from './StepperField';
export type { StepperFieldButton, StepperFieldProps } from './StepperField';
// A title set as a headline field on a dashed rule.
export { HeadlineInput } from './HeadlineInput';
export type { HeadlineInputProps } from './HeadlineInput';
export { Card } from './Card';
export { VoiceButton } from './VoiceButton';
export { PromptDialog } from './PromptDialog';
export { ScrollFadeContainer } from './ScrollFadeContainer';
export { SwipeableRow } from './SwipeableRow';
export type { SwipeAction } from './SwipeableRow';
export { InitialsAvatar, avatarSlotColors } from './InitialsAvatar';
export { Sheet } from './Sheet';
export type { SheetProps } from './Sheet';
export { EmptyState } from './EmptyState';
export type { EmptyStateAction, EmptyStateProps } from './EmptyState';

export { CollapsibleSection } from './CollapsibleSection';
// The settings grammar: a group, its rows, and the text-tab filter over a list.
export { Section } from './Section';
export type { SectionProps } from './Section';
export { Row } from './Row';
export type { RowProps } from './Row';
export { StatusDot } from './StatusDot';
export type { StatusDotTone } from './StatusDot';
// The pill that names a state, and the tone table every tinted pill reads; the web's `.badge` family.
export { Badge, BADGE_TONE, badgeInkColor } from './Badge';
export type { BadgeProps, BadgeVariant } from './Badge';
export { ALERT_TONE, alertInkColor } from './alertTone';
export type { AlertTone } from './alertTone';
export { switchColors } from './switchTone';
export type { SwitchColors } from './switchTone';
export { TextTabs } from './TextTabs';
export type { TextTabItem, TextTabsProps } from './TextTabs';
// The scroll container that insets itself for the native header and tab bar.
export { PaneScrollView } from './PaneScrollView';
// The scroll container that keeps the focused field of a form above the keyboard.
export { FormScrollView, FORM_KEYBOARD_GAP } from './FormScrollView';
export { AppearanceToggleButton } from './AppearanceToggleButton';
export { OnboardingProgressBar } from './OnboardingProgressBar';
export type { OnboardingProgressBarProps } from './OnboardingProgressBar';
export { BrandLockup } from './BrandLockup';
export { HeaderActions } from './HeaderActions';
export { HeaderCloseButton } from './HeaderCloseButton';
