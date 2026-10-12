// ABOUTME: Onboarding step (mobile) — how often the coach checks in on its own: every morning, three a week, weekly or never
// ABOUTME: Mirrors the web OnboardingCheckIn; a tap stores the cadence before the flow moves on (carnet#831)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState } from 'react';
import { View, Text, ScrollView, Pressable, ActivityIndicator } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import type { CheckInCadence } from '@pierre/shared-types';
import { CHECK_IN_OPTIONS } from '@pierre/ui-logic';
import { useTranslation } from '@pierre/i18n';
import { Row } from '../../components/ui';
import { OnboardingProgressBar } from '../../components/ui/OnboardingProgressBar';
import { useAuth } from '../../contexts/AuthContext';
import { userApi } from '../../services/api';
import { useCheckIn } from '../../hooks/useCheckIn';
import { useOnboardingFlag } from '../../hooks/useOnboardingFlag';
import { useOnboardingProgress } from '../../hooks/useOnboardingProgress';

/** Web-matching storage key prefix for this step. */
const STORAGE_PREFIX = 'dravr.check_in_done.';

/**
 * The check-in cadence step (mobile).
 *
 * A choice is stored before the flow moves on: a failed write keeps the
 * athlete here to retry rather than advancing them past a question the server
 * never heard answered. Skipping stores nothing, which the worker reads as
 * never; Settings changes the answer later.
 */
export function OnboardingCheckInScreen() {
  const { t } = useTranslation();
  const { user } = useAuth();
  const { mark } = useOnboardingFlag(STORAGE_PREFIX, user?.id);
  const progress = useOnboardingProgress('check_in');
  const checkIn = useCheckIn();
  const [choosing, setChoosing] = useState<CheckInCadence | null>(null);
  const [saveFailed, setSaveFailed] = useState(false);

  const finish = async (status: 'complete' | 'skipped') => {
    userApi.setOnboardingStep('check_in', status).catch(() => {});
    await mark();
  };

  const choose = async (cadence: CheckInCadence) => {
    if (choosing) return;
    setChoosing(cadence);
    setSaveFailed(false);
    try {
      await checkIn.choose(cadence);
    } catch {
      setChoosing(null);
      setSaveFailed(true);
      return;
    }
    await finish('complete');
  };

  return (
    <SafeAreaView className="flex-1 bg-surface">
      <ScrollView contentContainerClassName="py-10">
        <View className="px-4">
          <OnboardingProgressBar steps={progress} />
          <Text className="mt-4 text-3xl font-display text-left text-on-surface">{t('onboarding.checkInHeading')}</Text>
          <Text className="mt-3 text-sm text-on-surface-variant">{t('onboarding.checkInIntro')}</Text>
        </View>

        <View className="mt-8">
          {CHECK_IN_OPTIONS.map((option, index) => (
            <Row
              key={option.value}
              title={t(option.labelKey)}
              onPress={choosing === null ? () => void choose(option.value) : undefined}
              showChevron={false}
              last={index === CHECK_IN_OPTIONS.length - 1}
              accessibilityRole="button"
              accessibilityState={{ disabled: choosing !== null, busy: choosing === option.value }}
              trailing={choosing === option.value ? <ActivityIndicator size="small" /> : undefined}
              testID={`check-in-option-${option.value}`}
            />
          ))}
        </View>

        {saveFailed ? (
          <Text className="mt-4 px-4 text-sm text-error" accessibilityRole="alert">
            {t('settings.checkInSaveFailed')}
          </Text>
        ) : null}

        <View className="px-4 mt-8">
          <Pressable
            onPress={() => void finish('skipped')}
            disabled={choosing !== null}
            accessibilityRole="button"
            testID="check-in-skip"
          >
            <Text className="text-center text-sm text-on-surface-variant">{t('app.skipForNow')}</Text>
          </Pressable>
        </View>
      </ScrollView>
    </SafeAreaView>
  );
}
