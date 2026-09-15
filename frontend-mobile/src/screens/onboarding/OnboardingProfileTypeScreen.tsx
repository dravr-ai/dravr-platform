// ABOUTME: First-run onboarding step (mobile) — asks whether the user is an athlete or a coach
// ABOUTME: Mirrors the web OnboardingProfileType; "coach" sets coaching_persona=coach; advances via the shared flag cache

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState } from 'react';
import { View, Text, ScrollView, ActivityIndicator } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { Row } from '../../components/ui';
import { OnboardingProgressBar } from '../../components/ui/OnboardingProgressBar';
import { userApi } from '../../services/api';
import { useAuth } from '../../contexts/AuthContext';
import { useProfileTypeChosen } from '../../hooks/useProfileTypeChosen';
import { useOnboardingProgress } from '../../hooks/useOnboardingProgress';
import { useThemeColors } from '../../constants/theme';
import type { OnboardingProgressItem } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';

/**
 * Athlete-vs-coach onboarding step (mobile).
 *
 * Reached via RootLayoutNav for a fresh account before the connect-provider step.
 * Picking t('humanCoach.iCoachOthers') persists `coaching_persona=coach`; either choice marks
 * the profile-type step done (locally + on the server) and flips the shared
 * `useProfileTypeChosen` cache, which routes the user on.
 */
export function OnboardingProfileTypeScreen() {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const { user } = useAuth();
  const { markChosen } = useProfileTypeChosen(user?.id);
  const progress = useOnboardingProgress('profile_type');
  const [choosing, setChoosing] = useState<'athlete' | 'coach' | null>(null);

  const finish = async (persona: 'athlete' | 'coach') => {
    if (choosing) return;
    setChoosing(persona);
    if (persona === 'coach') {
      try {
        await userApi.setCoachingPersona('coach');
      } catch {
        // Non-fatal: the default Casual voice is a harmless start; changeable in Settings.
      }
    }
    userApi.setOnboardingStep('profile_type', 'complete').catch(() => {});
    await markChosen();
  };

  const heading = user?.display_name
    ? t('onboarding.welcomeNamed', { name: user.display_name })
    : t('app.welcomeToDravr');

  return (
    <Shell heading={heading} progress={progress}>
      <View className="px-4">
        <Text className="mt-3 text-sm text-on-surface-variant">
          {t('app.profileTypeBlurb')}
        </Text>
      </View>
      <View className="mt-6">
        <Row
          title={t('app.imAnAthlete')}
          subtitle={t('onboarding.athleteCardDescription')}
          trailing={choosing === 'athlete' ? <ActivityIndicator size="small" color={colors.tokens.primary} /> : undefined}
          onPress={() => void finish('athlete')}
          accessibilityLabel={t('app.imAnAthlete')}
          testID="profile-type-athlete"
        />
        <Row
          title={t('humanCoach.iCoachOthers')}
          subtitle={t('humanCoach.cardDescription')}
          trailing={choosing === 'coach' ? <ActivityIndicator size="small" color={colors.tokens.primary} /> : undefined}
          onPress={() => void finish('coach')}
          accessibilityLabel={t('humanCoach.iCoachOthers')}
          last
          testID="profile-type-coach"
        />
      </View>
    </Shell>
  );
}

function Shell({
  heading,
  children,
  progress,
}: {
  heading?: string;
  children: React.ReactNode;
  progress: OnboardingProgressItem[];
}) {
  return (
    <SafeAreaView className="flex-1 bg-background-primary" testID="profile-type-screen">
      <ScrollView contentContainerClassName="py-8">
        <View className="px-4">
          <OnboardingProgressBar steps={progress} />
          {heading ? (
            <Text className="mt-4 text-3xl font-display text-left text-on-surface">{heading}</Text>
          ) : null}
        </View>
        {children}
      </ScrollView>
    </SafeAreaView>
  );
}
