// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The check-in choice in Settings — every morning, three a week, weekly or never — the onboarding answer, changeable
// ABOUTME: Writes the server-side cadence the coach's check-in worker reads, as the web control does (carnet#831)

import React from 'react';
import { Text } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { useTranslation } from '@pierre/i18n';
import { CHECK_IN_OPTIONS } from '@pierre/ui-logic';
import { Row, Section } from '../../components/ui';
import { useThemeColors } from '../../constants/theme';
import { useCheckIn } from '../../hooks/useCheckIn';

/**
 * How often the coach writes first, one row per choice. An athlete who never
 * chose is shown never, which is what they get until they pick one.
 */
export function CheckInSettingsSection(): React.JSX.Element {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const checkIn = useCheckIn();
  const preference = checkIn.preference;
  const locked = preference === null || checkIn.isUpdating;

  return (
    <Section title={t('settings.checkIn')} description={t('settings.checkInDescription')} testID="check-in-settings">
      {CHECK_IN_OPTIONS.map((option, index) => {
        const isSelected = preference !== null && (preference.cadence ?? 'off') === option.value;
        return (
          <Row
            key={option.value}
            title={t(option.labelKey)}
            onPress={locked ? undefined : () => checkIn.update(option.value)}
            showChevron={false}
            last={index === CHECK_IN_OPTIONS.length - 1}
            accessibilityRole="radio"
            accessibilityState={{ selected: isSelected, disabled: locked }}
            trailing={isSelected ? <Feather name="check" size={18} color={colors.tokens.primary} /> : undefined}
            testID={`check-in-option-${option.value}`}
          />
        );
      })}
      {checkIn.saveFailed ? (
        <Text className="mt-2 px-4 text-sm text-error" accessibilityRole="alert">
          {t('settings.checkInSaveFailed')}
        </Text>
      ) : null}
    </Section>
  );
}
