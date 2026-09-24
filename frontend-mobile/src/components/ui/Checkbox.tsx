// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The one checkbox: a 20pt box beside its label, filled with the green and ticked in on-primary when checked
// ABOUTME: The whole row is the press target and carries the checkbox role, its checked state and the label as its name

import React from 'react';
import { Pressable, Text, View } from 'react-native';
import { Check } from 'lucide-react-native';
import { useThemeColors } from '../../constants/theme';

export interface CheckboxProps {
  checked: boolean;
  /** Called with the state a press asks for — the opposite of `checked`. */
  onChange: (checked: boolean) => void;
  /** The visible label, and the control's accessible name. */
  label: string;
  /**
   * The label's ink class. Body ink by default; on a tinted notice, the ink
   * that tint's hue binds (`text-on-warning-container` on a warning tint).
   */
  labelClassName?: string;
  testID?: string;
}

/**
 * A checked box is the one green (`primary`) with the tick in `onPrimary`, the
 * same pairing as a filled button, so it reads in both schemes: 7.44:1 light,
 * 10.02:1 dark.
 *
 * An unchecked box is drawn in `outline`, not the `border-strong` hairline. The
 * empty box is the only thing that shows the control is there and unticked,
 * so its edge answers to the 3:1 non-text floor (WCAG 1.4.11), and the
 * hairline does not reach it: 1.55:1 on the light canvas, 2.30:1 on the dark
 * one. `outline` measures 6.58:1 and 5.89:1 there, and 5.94:1 and 4.92:1 on a
 * warning notice's tint.
 */
export function Checkbox({
  checked,
  onChange,
  label,
  labelClassName = 'text-text-primary',
  testID,
}: CheckboxProps) {
  const colors = useThemeColors();

  return (
    <Pressable
      className="flex-row items-start min-h-11 py-1"
      onPress={() => onChange(!checked)}
      accessibilityRole="checkbox"
      accessibilityState={{ checked }}
      accessibilityLabel={label}
      testID={testID}
    >
      <View
        className="w-5 h-5 rounded border-2 items-center justify-center mr-2"
        style={{
          backgroundColor: checked ? colors.tokens.primary : 'transparent',
          borderColor: checked ? colors.tokens.primary : colors.tokens.outline,
        }}
      >
        {checked ? <Check size={14} strokeWidth={3} color={colors.tokens.onPrimary} /> : null}
      </View>
      <Text className={`flex-1 text-sm font-medium ${labelClassName}`}>{label}</Text>
    </Pressable>
  );
}
