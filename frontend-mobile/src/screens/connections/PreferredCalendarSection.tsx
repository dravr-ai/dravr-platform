// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The calendar choice on the Connections screen — which connected calendar Dravr's plan and workouts go on
// ABOUTME: Shown with several calendars or a recorded one; the agent asks when none is chosen (carnet#1393)

import React from 'react';
import { Text } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { useTranslation } from '@pierre/i18n';
import { Row, Section } from '../../components/ui';
import { useThemeColors } from '../../constants/theme';
import { usePreferredCalendar } from '../../hooks/usePreferredCalendar';

interface PreferredCalendarSectionProps {
  /**
   * The connected providers as the rows above show them, in any stable form:
   * when it changes, a connection was made or removed here, and the calendars
   * to choose from are read again.
   */
  connected: string;
}

/** One row per connected calendar plus "none chosen", in which case the agent asks. */
export function PreferredCalendarSection({ connected }: PreferredCalendarSectionProps): React.JSX.Element | null {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const preferred = usePreferredCalendar(connected);

  if (!preferred.visible || preferred.calendar === null) return null;
  const { chosen, chosen_connected: chosenConnected, calendars } = preferred.calendar;
  const options: { value: string | null; label: string }[] = [
    { value: null, label: t('settings.preferredCalendarNone') },
    ...calendars.map((calendar) => ({ value: calendar.provider, label: calendar.name })),
  ];
  const disabled = preferred.isUpdating;

  return (
    <Section
      title={t('settings.preferredCalendar')}
      description={t('settings.preferredCalendarDescription')}
      testID="preferred-calendar-section"
    >
      {options.map((option, index) => {
        const isSelected = preferred.selected === option.value;
        const id = option.value ?? 'none';
        return (
          <Row
            key={id}
            title={option.label}
            onPress={disabled ? undefined : () => preferred.choose(option.value)}
            showChevron={false}
            last={index === options.length - 1}
            accessibilityRole="radio"
            accessibilityState={{ selected: isSelected, disabled }}
            trailing={isSelected ? <Feather name="check" size={18} color={colors.tokens.primary} /> : undefined}
            testID={`preferred-calendar-option-${id}`}
          />
        );
      })}
      {chosen !== null && !chosenConnected ? (
        <Text className="mt-2 px-4 text-sm text-text-secondary" testID="preferred-calendar-disconnected">
          {t('settings.preferredCalendarDisconnected', { name: chosen.name })}
        </Text>
      ) : null}
      {preferred.saveFailed ? (
        <Text className="mt-2 px-4 text-sm text-error" accessibilityRole="alert">
          {t('settings.preferredCalendarSaveFailed')}
        </Text>
      ) : null}
    </Section>
  );
}
