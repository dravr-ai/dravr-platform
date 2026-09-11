// ABOUTME: The one button: four variants, 44 tall, radius 8, no shadow (Boreal v2.2 D4)
// ABOUTME: primary fills with the one green; secondary and ghost are hairline and bare; danger fills with error

import React from 'react';
import {
  TouchableOpacity,
  Text,
  ActivityIndicator,
  type ViewStyle,
  type TextStyle,
} from 'react-native';
import { useThemeColors } from '../../constants/theme';

type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger';

interface ButtonProps {
  title: string;
  onPress: () => void;
  variant?: ButtonVariant;
  disabled?: boolean;
  loading?: boolean;
  fullWidth?: boolean;
  style?: ViewStyle;
  textStyle?: TextStyle;
  testID?: string;
}

// 44 is the thumb's floor and the only height; radius 8 is the ladder's step
// for buttons and fields. A resting button casts no shadow — hairlines lift,
// shadows float (DESIGN.md §4).
const baseClasses = 'flex-row items-center justify-center rounded-lg h-11 px-6';

const variantClasses: Record<ButtonVariant, string> = {
  primary: 'bg-primary',
  secondary: 'bg-transparent border border-border-strong',
  ghost: 'bg-transparent',
  danger: 'bg-error',
};

const textVariantClasses: Record<ButtonVariant, string> = {
  primary: 'text-on-primary',
  secondary: 'text-text-primary',
  ghost: 'text-primary',
  danger: 'text-on-error',
};

/** The variants whose label sits on a filled ground. */
const FILLED: ReadonlySet<ButtonVariant> = new Set(['primary', 'danger']);

export function Button({
  title,
  onPress,
  variant = 'primary',
  disabled = false,
  loading = false,
  fullWidth = false,
  style,
  textStyle,
  testID,
}: ButtonProps) {
  const colors = useThemeColors();
  const isDisabled = disabled || loading;

  const buttonClassName = [
    baseClasses,
    variantClasses[variant],
    fullWidth ? 'w-full' : '',
    isDisabled ? 'opacity-50' : '',
  ].filter(Boolean).join(' ');

  const textClassName = [
    'text-base font-semibold',
    textVariantClasses[variant],
    isDisabled ? 'opacity-70' : '',
  ].filter(Boolean).join(' ');

  // The spinner takes the label's ink: `onPrimary` on a filled ground (the
  // same token whether the ground is the green or the error), the green
  // itself on a bare one — never a frozen hex, which was the wrong green in
  // dark.
  const spinnerColor = FILLED.has(variant)
    ? variant === 'danger'
      ? colors.tokens.onError
      : colors.tokens.onPrimary
    : colors.tokens.primary;

  return (
    <TouchableOpacity
      className={buttonClassName}
      style={style}
      onPress={onPress}
      disabled={isDisabled}
      activeOpacity={0.7}
      testID={testID}
    >
      {loading ? (
        <ActivityIndicator color={spinnerColor} size="small" />
      ) : (
        <Text className={textClassName} style={textStyle}>{title}</Text>
      )}
    </TouchableOpacity>
  );
}
