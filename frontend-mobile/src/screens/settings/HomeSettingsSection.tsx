// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home choices in Settings — today, the switch that brings back the suggestion to build a training plan
// ABOUTME: Writes the same server-side preference Home's "Hide" sets, so web and mobile agree (carnet#820)

import React from 'react';
import { Switch, Text, View } from 'react-native';
import { useTranslation } from '@pierre/i18n';
import { Section } from '../../components/ui';
import { useThemeColors } from '../../constants/theme';
import { useHomePreferences } from '../../hooks/useHome';

/**
 * One switch: whether Home suggests building a training plan while the
 * athlete has none. Home's "Hide" turns it off; this is the way back.
 */
export function HomeSettingsSection(): React.JSX.Element {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const home = useHomePreferences();
  const label = t('home.settings.planSuggestion');

  return (
    <Section title={t('home.settings.title')} testID="home-settings">
      <View className="flex-row items-center justify-between gap-4 px-4">
        <View className="flex-1">
          <Text className="text-sm text-text-primary">{label}</Text>
          <Text className="mt-0.5 text-xs text-text-secondary">{t('home.settings.planSuggestionHint')}</Text>
        </View>
        <Switch
          testID="home-settings-plan-suggestion"
          accessibilityLabel={label}
          value={home.preferences ? !home.preferences.plan_suggestion_hidden : true}
          onValueChange={(offered) =>
            home.update({
              ...(home.preferences ?? { plan_suggestion_hidden: false }),
              plan_suggestion_hidden: !offered,
            })
          }
          trackColor={{ false: colors.border.default, true: colors.tokens.primary }}
          disabled={home.preferences === null || home.isUpdating}
        />
      </View>
      {home.isError ? (
        <Text className="mt-2 px-4 text-sm text-error" accessibilityRole="alert">
          {t('common.error')}
        </Text>
      ) : null}
    </Section>
  );
}
