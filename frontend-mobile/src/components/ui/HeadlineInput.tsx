// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A headline set as a field — the display face on a dashed rule, for a title that is also the screen's own head (the day editor's session)
// ABOUTME: No label is drawn, so it is named by its accessibility label; the dashed rule is what says it can be typed into

import React, { forwardRef } from 'react';
import { TextInput, type TextInputProps, View } from 'react-native';
import { useThemeColors } from '../../constants/theme';

export interface HeadlineInputProps extends Omit<TextInputProps, 'style' | 'className'> {
  /** The field's name, since nothing on screen labels it ("Title"). */
  accessibilityLabel: string;
}

/**
 * Schibsted Grotesk at the title step over a 2pt dashed rule in
 * `outline-variant`. The rule is a box with a dashed edge rather than a
 * one-sided dashed border, which iOS draws solid.
 */
export const HeadlineInput = forwardRef<TextInput, HeadlineInputProps>(function HeadlineInput(props, ref) {
  const colors = useThemeColors();
  return (
    <View>
      <TextInput
        ref={ref}
        className="min-h-11 py-1 font-display text-2xl text-text-primary"
        placeholderTextColor={colors.text.tertiary}
        selectionColor={colors.tokens.primary}
        {...props}
      />
      <View className="h-0.5 rounded-sm border border-dashed border-outline-variant" />
    </View>
  );
});
