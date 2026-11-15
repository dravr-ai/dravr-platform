// ABOUTME: Onboarding step (mobile) — Santé, the PAR-Q+ as the third screen, after Tes données and before the team
// ABOUTME: Mirrors the web OnboardingParq: one tap for the common answer; a "yes" raises an agent-visible flag and never blocks sign-up

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React from 'react';
import { View, Text, ScrollView, ActivityIndicator } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { OnboardingProgressBar } from '../../components/ui/OnboardingProgressBar';
import { ParqForm, useParqQuestions } from '../../components/ParqForm';
import { useAuth } from '../../contexts/AuthContext';
import { userApi } from '../../services/api';
import { useOnboardingFlag } from '../../hooks/useOnboardingFlag';
import { useOnboardingProgress } from '../../hooks/useOnboardingProgress';
import { useRecordOnboardingStep } from '../../hooks/useRecordOnboardingStep';
import { useTranslation } from '@pierre/i18n';

/** Web-matching storage key prefix for this step. */
const STORAGE_PREFIX = 'dravr.parq_done.';

/**
 * The seven standard PAR-Q+ questions (mobile, carnet#1530).
 *
 * A "yes" never blocks sign-up — it writes an agent-visible medical flag with
 * a 12-month freshness horizon. Put off, it becomes the web Home's first
 * « Pour bien démarrer » row, and « Construire mon plan » opens the health
 * check first on both clients.
 */
export function OnboardingParqScreen() {
  const { t } = useTranslation();
  const { user } = useAuth();
  const { mark } = useOnboardingFlag(STORAGE_PREFIX, user?.id);
  const progress = useOnboardingProgress('parq');
  const recordStep = useRecordOnboardingStep();
  const { data, isLoading, isError } = useParqQuestions();

  const finish = async (status: 'complete' | 'skipped') => {
    recordStep('parq', status);
    await mark();
  };

  // A screen the user can't answer is a screen they shouldn't be held on.
  React.useEffect(() => {
    if (isError) void finish('skipped');
    // eslint-disable-next-line react-hooks/exhaustive-deps -- fire once on failure
  }, [isError]);

  if (isLoading || isError || !data) {
    return (
      <SafeAreaView className="flex-1 bg-surface items-center justify-center">
        <ActivityIndicator size="large" />
      </SafeAreaView>
    );
  }

  return (
    <SafeAreaView className="flex-1 bg-surface">
      <ScrollView contentContainerClassName="py-10">
        <View className="px-4">
          <OnboardingProgressBar steps={progress} />
          <Text className="mt-4 text-3xl font-display text-left text-on-surface">{t('onboarding.parqHeading')}</Text>
          <Text className="mt-3 text-sm text-on-surface-variant">{t('onboarding.parq.intro')}</Text>
        </View>

        <View className="mt-6 px-4">
          <ParqForm
            questions={data.questions}
            onSubmit={async (answers) => {
              try {
                await userApi.submitParq(answers);
              } catch {
                // Non-fatal: this screen is explicitly not a gate.
              }
              await finish('complete');
            }}
            onSkip={() => void finish('skipped')}
          />
        </View>
      </ScrollView>
    </SafeAreaView>
  );
}
