// ABOUTME: Privacy settings screen — the app's analytics-consent (GDPR) control, mirroring the web Privacy & Data tab
// ABOUTME: Optimistic switch that writes through userApi.updateAnalyticsConsent and reverts when the write fails
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState, useEffect } from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  Switch,
  Alert,
  type ViewStyle,
} from 'react-native';
import { PaneScrollView } from '../../components/ui';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useRouter } from 'expo-router';
import { useMutation } from '@tanstack/react-query';
import { Feather } from '@expo/vector-icons';
import { spacing, useCardStyle, useThemeColors } from '../../constants/theme';
import { userApi } from '../../services/api';
import { useAuth } from '../../contexts/AuthContext';
import { useTranslation } from '@pierre/i18n';

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

export function PrivacySettingsScreen(): React.JSX.Element {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const sectionCardStyle: ViewStyle = {
    borderRadius: 12,
    padding: spacing.md,
    ...useCardStyle(),
  };
  const router = useRouter();
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
      const message = err instanceof Error ? err.message : t('app.failedAnalyticsConsent');
      Alert.alert(t('app.couldNotSavePreference'), message);
    },
  });

  const handleToggle = (value: boolean): void => {
    setAnalyticsConsent(value);
    consentMutation.mutate(value);
  };

  return (
    <SafeAreaView className="flex-1 bg-background-primary" testID="privacy-settings-screen">
      {/* Header */}
      <View className="flex-row items-center px-4 py-3 border-b border-white/10">
        <TouchableOpacity className="p-2 mr-2" onPress={() => router.back()} testID="back-button">
          <Feather name="arrow-left" size={24} color={colors.text.primary} />
        </TouchableOpacity>
        <Text className="flex-1 text-lg font-bold text-text-primary">{t('app.privacyAndData')}</Text>
      </View>

      <PaneScrollView className="flex-1 px-4" showsVerticalScrollIndicator={false}>
        {/* Analytics consent */}
        <Text className="text-text-secondary text-sm font-semibold mt-6 mb-2 ml-2 uppercase tracking-wide">
          {t('app.usageAnalytics')}
        </Text>
        <View style={sectionCardStyle}>
          <View className="flex-row items-center py-2">
            <View
              className="w-10 h-10 rounded-full justify-center items-center mr-4"
              style={{ backgroundColor: colors.pierre.violet + '20' }}
            >
              <Feather name="bar-chart-2" size={20} color={colors.pierre.violet} />
            </View>
            <View className="flex-1 mr-4">
              <Text className="text-text-primary text-base font-semibold">{t('app.usageAnalytics')}</Text>
              <Text className="text-text-tertiary text-sm mt-0.5">
                {t('app.analyticsBlurb')}
              </Text>
            </View>
            <Switch
              testID="analytics-consent-switch"
              value={analyticsConsent}
              onValueChange={handleToggle}
              trackColor={{ false: colors.background.tertiary, true: colors.pierre.violet + '60' }}
              thumbColor={analyticsConsent ? colors.pierre.violet : colors.text.tertiary}
              disabled={consentMutation.isPending}
            />
          </View>
        </View>

        {/* What we collect */}
        <Text className="text-text-secondary text-sm font-semibold mt-6 mb-2 ml-2 uppercase tracking-wide">
          {t('app.whatWeCollect')}
        </Text>
        <View style={sectionCardStyle}>
          {COLLECTED_WHEN_ENABLED_KEYS.map((item) => (
            <View key={item} className="flex-row items-start py-1.5">
              <Feather name="check" size={16} color={colors.pierre.activity} style={{ marginTop: 2 }} />
              <Text className="flex-1 text-text-secondary text-sm ml-2">{t(item)}</Text>
            </View>
          ))}
        </View>

        <Text className="text-text-secondary text-sm font-semibold mt-6 mb-2 ml-2 uppercase tracking-wide">
          {t('app.whatWeNeverCollect')}
        </Text>
        <View style={sectionCardStyle}>
          {NEVER_COLLECTED_KEYS.map((item) => (
            <View key={item} className="flex-row items-start py-1.5">
              <Feather name="x" size={16} color={colors.pierre.red} style={{ marginTop: 2 }} />
              <Text className="flex-1 text-text-secondary text-sm ml-2">{t(item)}</Text>
            </View>
          ))}
        </View>

        <View className="h-6" />
      </PaneScrollView>
    </SafeAreaView>
  );
}
