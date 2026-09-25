// ABOUTME: Privacy settings screen — the app's analytics-consent (GDPR) control, mirroring the web Privacy & Data tab
// ABOUTME: Optimistic switch that writes through userApi.updateAnalyticsConsent and reverts when the write fails
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState, useEffect } from 'react';
import { View, Text, Switch, Alert } from 'react-native';
import { PaneScrollView, Section } from '../../components/ui';
import { useMutation } from '@tanstack/react-query';
import { Feather } from '@expo/vector-icons';
import { spacing, useThemeColors } from '../../constants/theme';
import { userApi } from '../../services/api';
import { useAuth } from '../../contexts/AuthContext';
import { useTranslation } from '@pierre/i18n';
import { describeApiError } from '@pierre/ui-logic';

// Same lists the web Privacy & Data tab shows, so the two surfaces cannot
// promise different things about what leaves the device.
const COLLECTED_WHEN_ENABLED_KEYS = [
  'app.analyticsCollect0',
  'app.analyticsCollect1',
  'app.analyticsCollect2',
] as const;

const NEVER_COLLECTED_KEYS = [
  'app.analyticsNever0',
  'app.analyticsNever1',
  'app.analyticsNever2',
] as const;

/**
 * Three sections separated by space, not by cards: the consent switch as the
 * first section's action, then the two promise lists as glyph-led lines
 * (DESIGN.md §10).
 */
export function PrivacySettingsScreen(): React.JSX.Element {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const { user, updateUser } = useAuth();

  // Analytics consent is stored on the user record, so the switch is seeded
  // from the auth context rather than a screen-local fetch.
  const [analyticsConsent, setAnalyticsConsent] = useState(user?.analytics_consent ?? false);

  useEffect(() => {
    if (user?.analytics_consent != null) {
      setAnalyticsConsent(user.analytics_consent);
    }
  }, [user?.analytics_consent]);

  /**
   * Optimistic so the switch does not lag the tap, but reverted on failure —
   * a switch that stays flipped after a failed write tells the user their data
   * sharing is off when it is still on, which is the one place this screen
   * must not be wrong.
   */
  const consentMutation = useMutation({
    mutationFn: (value: boolean) => userApi.updateAnalyticsConsent(value),
    onSuccess: async (_data, value) => {
      await updateUser({ analytics_consent: value });
    },
    onError: (err: unknown, value) => {
      setAnalyticsConsent(!value);
      const message = describeApiError(err, { t, fallbackKey: 'app.failedAnalyticsConsent' });
      Alert.alert(t('app.couldNotSavePreference'), message);
    },
  });

  const handleToggle = (value: boolean): void => {
    setAnalyticsConsent(value);
    consentMutation.mutate(value);
  };

  return (
    <View className="flex-1 bg-background-primary" testID="privacy-settings-screen">
      <PaneScrollView
        contentContainerStyle={{ paddingTop: spacing.lg, paddingBottom: spacing.xl }}
        showsVerticalScrollIndicator={false}
      >
        <View className="gap-8">
          <Section
            title={t('app.usageAnalytics')}
            description={t('app.analyticsBlurb')}
            testID="privacy-section-analytics"
            actions={
              <Switch
                testID="analytics-consent-switch"
                value={analyticsConsent}
                onValueChange={handleToggle}
                trackColor={{ false: colors.border.default, true: colors.tokens.primary }}
                disabled={consentMutation.isPending}
              />
            }
          />

          {/* Plain lines, not rows: they pay the pane's inset themselves. */}
          <Section title={t('app.whatWeCollect')} testID="privacy-section-collected">
            <View className="gap-2 px-4">
              {COLLECTED_WHEN_ENABLED_KEYS.map((item) => (
                <View key={item} className="flex-row items-start gap-2">
                  <Feather name="check" size={16} color={colors.success} style={{ marginTop: 1 }} />
                  <Text className="flex-1 text-sm text-text-secondary">{t(item)}</Text>
                </View>
              ))}
            </View>
          </Section>

          <Section title={t('app.whatWeNeverCollect')} testID="privacy-section-never">
            <View className="gap-2 px-4">
              {NEVER_COLLECTED_KEYS.map((item) => (
                <View key={item} className="flex-row items-start gap-2">
                  <Feather name="x" size={16} color={colors.text.tertiary} style={{ marginTop: 1 }} />
                  <Text className="flex-1 text-sm text-text-secondary">{t(item)}</Text>
                </View>
              ))}
            </View>
          </Section>
        </View>
      </PaneScrollView>
    </View>
  );
}
