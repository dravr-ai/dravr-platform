// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The one radio group: a form's single choice, as equal segments for a few short words or a stacked list of named options
// ABOUTME: Carries the radiogroup and radio roles with each option's checked state — a form choice, never the text-tab filter language

import React from 'react';
import { Pressable, StyleSheet, Text, View } from 'react-native';
import { useThemeColors } from '../../constants/theme';
import { BADGE_TONE } from './Badge';

export interface RadioOption {
  key: string;
  /** The option's visible name, and its accessible name. */
  label: string;
  /** One quiet line under the name, in the list layout: what the option is for. */
  description?: string;
}

export interface RadioGroupProps {
  options: RadioOption[];
  /** The `key` of the checked option; `null` checks none. */
  value: string | null;
  onChange: (key: string) => void;
  /**
   * The group's visible name, read as the radiogroup's name. A group the
   * screen's own heading already names (a mode switch under a sheet title)
   * goes without one.
   */
  label?: string;
  /**
   * `segmented`: one row of equal segments, for two to four short words (a
   * mode, a sport). `list`: one row per option with its radio, for names that
   * run long or a set that grows (a family, a format).
   */
  layout?: 'segmented' | 'list';
  /** Lands on the group; each option gets `${testID}-${option.key}`. */
  testID?: string;
}

/**
 * A segment is 44 tall; the checked one takes the primary tint and ink from
 * `BADGE_TONE` with a `primary` edge, so the state reads by fill, edge and
 * weight rather than by colour alone, and its edge clears the 3:1 non-text
 * floor on the sheet's ground. A list row is 44 at least, its 20pt radio
 * edged in `outline` (the empty radio's only mark, WCAG 1.4.11) and filled
 * with a `primary` dot when checked — the phone's `Checkbox` pairing.
 */
export function RadioGroup({ options, value, onChange, label, layout = 'list', testID }: RadioGroupProps) {
  const colors = useThemeColors();
  const segmented = layout === 'segmented';
  return (
    <View testID={testID}>
      {label !== undefined ? (
        <Text className="mb-2 text-sm font-medium text-text-secondary">{label}</Text>
      ) : null}
      <View
        accessibilityRole="radiogroup"
        accessibilityLabel={label}
        className={segmented ? 'flex-row rounded-lg border border-border p-0.5' : undefined}
        style={segmented ? { borderWidth: StyleSheet.hairlineWidth } : undefined}
      >
        {options.map((option) => {
          const checked = option.key === value;
          const optionTestID = testID ? `${testID}-${option.key}` : undefined;
          if (segmented) {
            return (
              <Pressable
                key={option.key}
                className={`min-h-11 flex-1 items-center justify-center rounded-md border px-2 ${
                  checked ? `${BADGE_TONE.primary.fill} border-primary` : 'border-transparent'
                }`}
                onPress={() => onChange(option.key)}
                accessibilityRole="radio"
                accessibilityState={{ checked }}
                accessibilityLabel={option.label}
                testID={optionTestID}
              >
                <Text
                  className={`text-center text-sm ${
                    checked ? `font-semibold ${BADGE_TONE.primary.ink}` : 'font-medium text-text-secondary'
                  }`}
                  numberOfLines={2}
                >
                  {option.label}
                </Text>
              </Pressable>
            );
          }
          return (
            <Pressable
              key={option.key}
              className="min-h-11 flex-row items-start py-2.5"
              onPress={() => onChange(option.key)}
              accessibilityRole="radio"
              accessibilityState={{ checked }}
              accessibilityLabel={option.label}
              accessibilityHint={option.description}
              testID={optionTestID}
            >
              <View
                className="mr-3 mt-px h-5 w-5 items-center justify-center rounded-full border-2"
                style={{ borderColor: checked ? colors.tokens.primary : colors.tokens.outline }}
              >
                {checked ? (
                  <View className="h-2.5 w-2.5 rounded-full" style={{ backgroundColor: colors.tokens.primary }} />
                ) : null}
              </View>
              <View className="flex-1">
                <Text className={`text-base text-text-primary ${checked ? 'font-semibold' : ''}`}>{option.label}</Text>
                {option.description ? (
                  <Text className="mt-0.5 text-sm text-text-secondary">{option.description}</Text>
                ) : null}
              </View>
            </Pressable>
          );
        })}
      </View>
    </View>
  );
}
