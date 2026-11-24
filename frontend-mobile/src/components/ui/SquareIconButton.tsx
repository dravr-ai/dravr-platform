// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A 44pt square icon button with the control-outline hairline — the − and + of a stepper (a set's repetitions, a step's time)
// ABOUTME: Named by its label alone, since the glyph says nothing to a screen reader; dimmed and inert where it would change nothing

import React from 'react';
import { Pressable } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { useThemeColors } from '../../constants/theme';

export interface SquareIconButtonProps {
  icon: React.ComponentProps<typeof Feather>['name'];
  /** The accessible name: what a press does ("One more repetition"). */
  label: string;
  disabled?: boolean;
  onPress: () => void;
  testID?: string;
}

/** Radius 8 like a button, on the page's own ground, its glyph in the primary ink of text. */
export function SquareIconButton({ icon, label, disabled = false, onPress, testID }: SquareIconButtonProps) {
  const colors = useThemeColors();
  return (
    <Pressable
      className={`h-11 w-11 items-center justify-center rounded-lg border border-border-strong bg-surface ${
        disabled ? 'opacity-40' : ''
      }`}
      onPress={onPress}
      disabled={disabled}
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled }}
      testID={testID}
    >
      <Feather name={icon} size={18} color={colors.text.primary} />
    </Pressable>
  );
}
