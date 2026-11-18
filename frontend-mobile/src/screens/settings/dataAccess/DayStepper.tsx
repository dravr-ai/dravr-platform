// ABOUTME: A calendar day chosen a day at a time — the day written out between two chevrons, bounded by a first and a last day
// ABOUTME: One adjustable element to a screen reader (swipe up or down), since the app ships no native date picker

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React from 'react';
import { Pressable, Text, View } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { useTranslation } from '@pierre/i18n';
import { addCivilDays, formatCivilDate } from '@pierre/ui-logic';
import { useThemeColors } from '../../../constants/theme';

const LONG_DATE: Intl.DateTimeFormatOptions = { weekday: 'short', day: 'numeric', month: 'long', year: 'numeric' };

interface DayStepperProps {
  label: string;
  hint?: string;
  /** The day shown, `YYYY-MM-DD`. */
  value: string;
  min?: string;
  max?: string;
  onChange: (day: string) => void;
  testID?: string;
}

export function DayStepper({ label, hint, value, min, max, onChange, testID }: DayStepperProps): React.JSX.Element {
  const { i18n } = useTranslation();
  const colors = useThemeColors();
  const earlier = addCivilDays(value, -1);
  const later = addCivilDays(value, 1);
  const canEarlier = earlier !== null && (min === undefined || earlier >= min);
  const canLater = later !== null && (max === undefined || later <= max);
  const written = formatCivilDate(value, i18n.language, LONG_DATE);

  const step = (next: string | null, allowed: boolean) => {
    if (next !== null && allowed) onChange(next);
  };

  const chevron = (name: 'chevron-left' | 'chevron-right', enabled: boolean, onPress: () => void, id: string) => (
    <Pressable
      onPress={onPress}
      disabled={!enabled}
      className="h-11 w-11 items-center justify-center"
      importantForAccessibility="no-hide-descendants"
      accessibilityElementsHidden
      testID={`${testID}-${id}`}
    >
      <Feather name={name} size={20} color={enabled ? colors.tokens.primary : colors.text.tertiary} />
    </Pressable>
  );

  return (
    <View className="mb-4">
      <Text className="mb-2 text-sm font-medium text-text-secondary">{label}</Text>
      <View
        className="flex-row items-center border-b border-border"
        accessible
        accessibilityRole="adjustable"
        accessibilityLabel={label}
        accessibilityValue={{ text: written }}
        accessibilityHint={hint}
        accessibilityActions={[{ name: 'increment' }, { name: 'decrement' }]}
        onAccessibilityAction={(event) =>
          event.nativeEvent.actionName === 'increment' ? step(later, canLater) : step(earlier, canEarlier)
        }
        testID={testID}
      >
        {chevron('chevron-left', canEarlier, () => step(earlier, canEarlier), 'earlier')}
        <Text className="flex-1 text-center text-base text-text-primary" testID={`${testID}-value`}>
          {written}
        </Text>
        {chevron('chevron-right', canLater, () => step(later, canLater), 'later')}
      </View>
      {hint !== undefined && <Text className="mt-1 text-xs text-text-tertiary">{hint}</Text>}
    </View>
  );
}
