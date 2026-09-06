// ABOUTME: Reusable button component with variants following Pierre/Stitch design system
// ABOUTME: Supports primary, secondary, ghost, danger, gradient, pill, and pillar variants

import React, { useMemo } from 'react';
import {
  TouchableOpacity,
  Text,
  ActivityIndicator,
  type ViewStyle,
  type TextStyle,
} from 'react-native';
import { PRIMARY_PALETTE, useThemeColors } from '../../constants/theme';

type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger' | 'gradient' | 'pill' | 'activity' | 'nutrition' | 'recovery';
type ButtonSize = 'sm' | 'md' | 'lg';

interface ButtonProps {
  title: string;
  onPress: () => void;
  variant?: ButtonVariant;
  size?: ButtonSize;
  disabled?: boolean;
  loading?: boolean;
  fullWidth?: boolean;
  style?: ViewStyle;
  textStyle?: TextStyle;
  testID?: string;
}

// Base classes for all buttons
const baseClasses = 'flex-row items-center justify-center rounded-xl';

// Variant classes
const variantClasses: Record<ButtonVariant, string> = {
  primary: 'bg-primary',
  secondary: 'bg-transparent border border-border-strong',
  ghost: 'bg-transparent',
  danger: 'bg-error',
  gradient: 'bg-primary',
  pill: 'bg-primary rounded-full',
  activity: 'bg-pierre-activity',
  nutrition: 'bg-pierre-nutrition',
  recovery: 'bg-pierre-recovery',
};

// Size classes
const sizeClasses: Record<ButtonSize, string> = {
  sm: 'py-1 px-4 min-h-[36px]',
  md: 'py-2 px-6 min-h-[44px]',
  lg: 'py-4 px-8 min-h-[52px]',
};

// Text classes
const textBaseClasses = 'font-semibold';

const textVariantClasses: Record<ButtonVariant, string> = {
  primary: 'text-on-primary',
  secondary: 'text-text-primary',
  ghost: 'text-primary-500',
  danger: 'text-on-error',
  gradient: 'text-on-primary',
  pill: 'text-on-primary',
  activity: 'text-on-primary',
  nutrition: 'text-on-primary',
  recovery: 'text-on-primary',
};

const textSizeClasses: Record<ButtonSize, string> = {
  sm: 'text-sm',
  md: 'text-base',
  lg: 'text-lg',
};

export function Button({
  title,
  onPress,
  variant = 'primary',
  size = 'md',
  disabled = false,
  loading = false,
  fullWidth = false,
  style,
  textStyle,
  testID,
}: ButtonProps) {
  const colors = useThemeColors();
  const isDisabled = disabled || loading;

  // Variant-specific styles (shadows require style objects in RN)
  const variantShadowStyles = useMemo<Partial<Record<ButtonVariant, ViewStyle>>>(() => ({
    gradient: {
      shadowColor: colors.pierre.violet,
      shadowOffset: { width: 0, height: 0 },
      shadowOpacity: 0.4,
      shadowRadius: 20,
      elevation: 12,
    },
    pill: {
      shadowColor: colors.pierre.violet,
      shadowOffset: { width: 0, height: 0 },
      shadowOpacity: 0.4,
      shadowRadius: 20,
      elevation: 12,
    },
    activity: {
      shadowColor: colors.pierre.activity,
      shadowOffset: { width: 0, height: 4 },
      shadowOpacity: 0.25,
      shadowRadius: 14,
      elevation: 8,
    },
    nutrition: {
      shadowColor: colors.pierre.nutrition,
      shadowOffset: { width: 0, height: 4 },
      shadowOpacity: 0.25,
      shadowRadius: 14,
      elevation: 8,
    },
    recovery: {
      shadowColor: colors.pierre.recovery,
      shadowOffset: { width: 0, height: 4 },
      shadowOpacity: 0.25,
      shadowRadius: 14,
      elevation: 8,
    },
  }), [colors]);

  const buttonClassName = [
    baseClasses,
    variantClasses[variant],
    sizeClasses[size],
    fullWidth ? 'w-full' : '',
    isDisabled ? 'opacity-50' : '',
  ].filter(Boolean).join(' ');

  const textClassName = [
    textBaseClasses,
    textVariantClasses[variant],
    textSizeClasses[size],
    isDisabled ? 'opacity-70' : '',
  ].filter(Boolean).join(' ');

  // Combine className styles with shadow styles (shadows need style prop in RN).
  // There used to be a `surface="light"` override here that pinned the fill and
  // the label to BOREAL_LIGHT for a screen with a fixed deep-green ground; that
  // screen now follows the athlete's appearance setting, so nothing called it.
  const combinedStyle: ViewStyle = {
    ...variantShadowStyles[variant],
    ...style,
  };

  return (
    <TouchableOpacity
      className={buttonClassName}
      style={combinedStyle}
      onPress={onPress}
      disabled={isDisabled}
      activeOpacity={0.7}
      testID={testID}
    >
      {loading ? (
        <ActivityIndicator
          color={['primary', 'gradient', 'pill', 'danger', 'activity', 'nutrition', 'recovery'].includes(variant) ? '#ffffff' : PRIMARY_PALETTE[500]}
          size="small"
        />
      ) : (
        <Text className={textClassName} style={textStyle}>{title}</Text>
      )}
    </TouchableOpacity>
  );
}
