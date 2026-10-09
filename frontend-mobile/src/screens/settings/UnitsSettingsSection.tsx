// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The units choice in Settings — Automatic, Metric or Imperial — and what Automatic resolved to
// ABOUTME: Writes the server-side preference the phone, the web and the agent's replies all read (carnet#835)

import React from 'react';
import { Text } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { useTranslation } from '@pierre/i18n';
import { UNIT_PREFERENCE_OPTIONS, unitsAutomaticHint } from '@pierre/ui-logic';
import { Row, Section } from '../../components/ui';
import { useThemeColors } from '../../constants/theme';
import { useUnitPreferences } from '../../hooks/useUnits';

/**
 * The athlete's units, one row per choice: their explicit choice always
 * wins; Automatic follows the connected provider's own setting, else the
 * phone's region.
 */
export function UnitsSettingsSection(): React.JSX.Element {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const units = useUnitPreferences();
  const prefs = units.preferences;
  const hint = prefs === null ? null : unitsAutomaticHint(prefs);

  return (
    <Section title={t('settings.units')} description={t('settings.unitsDescription')} testID="units-settings">
      {UNIT_PREFERENCE_OPTIONS.map((option, index) => {
        const isSelected = prefs?.preference === option.value;
        return (
          <Row
            key={option.value}
            title={t(option.labelKey)}
            onPress={prefs === null || units.isUpdating ? undefined : () => units.update(option.value)}
            showChevron={false}
            last={index === UNIT_PREFERENCE_OPTIONS.length - 1}
            accessibilityRole="radio"
            accessibilityState={{ selected: isSelected, disabled: prefs === null || units.isUpdating }}
            trailing={isSelected ? <Feather name="check" size={18} color={colors.tokens.primary} /> : undefined}
            testID={`units-option-${option.value}`}
          />
        );
      })}
      {hint !== null ? (
        <Text className="mt-2 px-4 text-xs text-text-secondary" testID="units-settings-hint">
          {t(hint.key, { provider: hint.provider, system: t(hint.systemKey) })}
        </Text>
      ) : null}
      {units.saveFailed ? (
        <Text className="mt-2 px-4 text-sm text-error" accessibilityRole="alert">
          {t('settings.unitsSaveFailed')}
        </Text>
      ) : null}
    </Section>
  );
}
