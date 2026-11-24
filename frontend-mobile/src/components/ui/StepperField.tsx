// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A short typed figure in the mono face, centred on a bottom stroke, with an optional − and + beside it — a step's m:ss on the day editor
// ABOUTME: The buttons are for a thumb that would rather not type; the field keeps whatever keyboard the caller names

import React, { forwardRef } from 'react';
import { TextInput, type TextInputProps, View } from 'react-native';
import { useThemeColors } from '../../constants/theme';
import { SquareIconButton } from './SquareIconButton';

/** One of the buttons beside the field. */
export interface StepperFieldButton {
  label: string;
  disabled?: boolean;
  onPress: () => void;
  testID?: string;
}

export interface StepperFieldProps extends Omit<TextInputProps, 'style' | 'className'> {
  /** The field's name: no label is drawn beside it, the caller draws its own above. */
  accessibilityLabel: string;
  /** The − before the field; left out, the field stands alone. */
  decrement?: StepperFieldButton;
  /** The + after the field. */
  increment?: StepperFieldButton;
}

/**
 * The figure sits on the same single bottom stroke as every field
 * (DESIGN.md "Inputs / Fields"), 44 tall, so a time typed in m:ss and the
 * buttons that move it read as one control.
 */
export const StepperField = forwardRef<TextInput, StepperFieldProps>(function StepperField(
  { decrement, increment, ...props },
  ref,
) {
  const colors = useThemeColors();
  return (
    <View className="flex-row items-center gap-2">
      {decrement ? <SquareIconButton icon="minus" {...decrement} /> : null}
      <TextInput
        ref={ref}
        className="h-11 min-w-20 border-b border-border px-2 text-center font-mono text-lg text-text-primary"
        placeholderTextColor={colors.text.tertiary}
        selectionColor={colors.tokens.primary}
        {...props}
      />
      {increment ? <SquareIconButton icon="plus" {...increment} /> : null}
    </View>
  );
});
