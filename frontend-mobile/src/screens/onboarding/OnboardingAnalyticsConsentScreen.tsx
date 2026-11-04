// ABOUTME: Onboarding step (mobile) — the usage-analytics question for an account no form asked: Apple/Google sign-in, chat sign-up, older accounts
// ABOUTME: Mirrors the web OnboardingAnalyticsConsent; Yes and No weigh the same and either is stored before moving on (carnet#1366)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState } from 'react';
import { View, Text, ScrollView, Pressable, ActivityIndicator } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { ANALYTICS_POLICY_URL } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { Row } from '../../components/ui';
import { useAuth } from '../../contexts/AuthContext';
import { userApi } from '../../services/api';
import { openExternal } from '../../utils/openExternal';

const CHOICES = [
  { enabled: true, labelKey: 'onboarding.analyticsConsentAccept', testID: 'analytics-consent-accept' },
  { enabled: false, labelKey: 'onboarding.analyticsConsentDecline', testID: 'analytics-consent-decline' },
] as const;

/**
 * The analytics consent step (mobile).
 *
 * Shown once to an account that has never answered; the sign-up form asks
 * with its own box. Its own question, nothing bundled, declining as easy as
 * accepting (Law 25). No progress bar: it is a question, not a stage. The
 * answer is stored before the flow moves on, and the session's user carries
 * it so the root layout routes past the step and analytics start on a yes.
 */
export function OnboardingAnalyticsConsentScreen() {
  const { t } = useTranslation();
  const { updateUser } = useAuth();
  const [answering, setAnswering] = useState<boolean | null>(null);
  const [saveFailed, setSaveFailed] = useState(false);

  const answer = async (enabled: boolean) => {
    if (answering !== null) return;
    setAnswering(enabled);
    setSaveFailed(false);
    try {
      await userApi.updateAnalyticsConsent(enabled);
      await updateUser({ analytics_consent: enabled, analytics_consent_answered: true });
    } catch {
      setAnswering(null);
      setSaveFailed(true);
    }
  };

  return (
    <SafeAreaView className="flex-1 bg-surface">
      <ScrollView contentContainerClassName="py-10">
        <View className="px-4">
          <Text className="mt-4 text-3xl font-display text-left text-on-surface">
            {t('onboarding.analyticsConsentTitle')}
          </Text>
          <Text className="mt-3 text-sm text-on-surface-variant">{t('onboarding.analyticsConsentBody')}</Text>
          <Pressable
            onPress={() => {
              void openExternal(ANALYTICS_POLICY_URL, t);
            }}
            accessibilityRole="link"
            className="mt-3 min-h-11 justify-center"
            testID="analytics-consent-policy"
          >
            <Text className="text-sm font-medium text-primary">{t('auth.analyticsConsentPolicyLink')}</Text>
          </Pressable>
        </View>

        <View className="mt-6">
          {CHOICES.map((choice, index) => (
            <Row
              key={choice.testID}
              title={t(choice.labelKey)}
              onPress={answering === null ? () => void answer(choice.enabled) : undefined}
              showChevron={false}
              last={index === CHOICES.length - 1}
              accessibilityRole="button"
              accessibilityState={{ disabled: answering !== null, busy: answering === choice.enabled }}
              trailing={answering === choice.enabled ? <ActivityIndicator size="small" /> : undefined}
              testID={choice.testID}
            />
          ))}
        </View>

        {saveFailed ? (
          <Text className="mt-4 px-4 text-sm text-error" accessibilityRole="alert">
            {t('onboarding.analyticsConsentSaveFailed')}
          </Text>
        ) : null}
      </ScrollView>
    </SafeAreaView>
  );
}
