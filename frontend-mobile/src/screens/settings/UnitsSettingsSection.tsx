// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The units choice in Settings — Metric or Imperial; every athlete reads metric until they choose otherwise
// ABOUTME: Writes the server-side preference the phone, the web and the agent's replies all read (carnet#835, carnet#1044)

import React from 'react';
import { Text } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { useTranslation } from '@pierre/i18n';
import { UNIT_SYSTEM_OPTIONS } from '@pierre/ui-logic';
import { Row, Section } from '../../components/ui';
import { useThemeColors } from '../../constants/theme';
import { useUnitPreferences } from '../../hooks/useUnits';

/** The athlete's units, one row per choice: metric unless they choose imperial here. */
export function UnitsSettingsSection(): React.JSX.Element {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const units = useUnitPreferences();
  const disabled = units.units === null || units.isUpdating;

  return (
    <Section title={t('settings.units')} description={t('settings.unitsDescription')} testID="units-settings">
      {UNIT_SYSTEM_OPTIONS.map((option, index) => {
        const isSelected = units.units === option.value;
        return (
          <Row
            key={option.value}
            title={t(option.labelKey)}
            onPress={disabled ? undefined : () => units.update(option.value)}
            showChevron={false}
            last={index === UNIT_SYSTEM_OPTIONS.length - 1}
            accessibilityRole="radio"
            accessibilityState={{ selected: isSelected, disabled }}
            trailing={isSelected ? <Feather name="check" size={18} color={colors.tokens.primary} /> : undefined}
            testID={`units-option-${option.value}`}
          />
        );
      })}
      {units.saveFailed ? (
        <Text className="mt-2 px-4 text-sm text-error" accessibilityRole="alert">
          {t('settings.unitsSaveFailed')}
        </Text>
      ) : null}
    </Section>
  );
}
