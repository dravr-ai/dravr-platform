// ABOUTME: First-run onboarding step (mobile) — asks whether the user is an athlete, a coach, or both
// ABOUTME: Mirrors the web OnboardingProfileType; coach answers set coaching_persona=coach; coach-only drops the athlete steps

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
import { ATHLETE_STEPS_WAIVED_PREFIX, useOnboardingFlag } from '../../hooks/useOnboardingFlag';
import { useOnboardingProgress } from '../../hooks/useOnboardingProgress';
import { useThemeColors } from '../../constants/theme';
import { ATHLETE_STEP_IDS, type OnboardingProgressItem } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';

/** The three answers the step offers. */
type ProfileChoice = 'athlete' | 'coach' | 'coach_and_athlete';

/**
 * Athlete / coach / both onboarding step (mobile).
 *
 * Reached via RootLayoutNav for a fresh account before the connect-provider step.
 * Both coach answers persist `coaching_persona=coach`. A coach who does not
 * train also takes the athlete steps (about-you, PAR-Q) out of the journey —
 * locally, and as `not_applicable` step rows on the server — before the
 * profile-type step is marked done, so the routing gate never lands them on
 * an athlete question. Mirrors the web OnboardingProfileType.
 */
export function OnboardingProfileTypeScreen() {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const { user } = useAuth();
  const { markChosen } = useProfileTypeChosen(user?.id);
  const { mark: markAthleteStepsWaived } = useOnboardingFlag(
    ATHLETE_STEPS_WAIVED_PREFIX,
    user?.id,
    false,
  );
  const progress = useOnboardingProgress('profile_type');
  const [choosing, setChoosing] = useState<ProfileChoice | null>(null);
  const [saveFailed, setSaveFailed] = useState(false);

  const finish = async (choice: ProfileChoice) => {
    if (choosing) return;
    setChoosing(choice);
    setSaveFailed(false);
    if (choice !== 'athlete') {
      try {
        await userApi.setCoachingPersona('coach');
      } catch {
        // The persona unlocks the coach tools this choice promises: keep the
        // user here to retry rather than route them on without them.
        setChoosing(null);
        setSaveFailed(true);
        return;
      }
    }
    if (choice === 'coach') {
      for (const stepId of ATHLETE_STEP_IDS) {
        userApi.setOnboardingStep(stepId, 'not_applicable').catch(() => {});
      }
      await markAthleteStepsWaived();
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
          testID="profile-type-coach"
        />
        <Row
          title={t('humanCoach.iCoachAndTrain')}
          subtitle={t('humanCoach.coachAndTrainCardDescription')}
          trailing={choosing === 'coach_and_athlete' ? <ActivityIndicator size="small" color={colors.tokens.primary} /> : undefined}
          onPress={() => void finish('coach_and_athlete')}
          accessibilityLabel={t('humanCoach.iCoachAndTrain')}
          last
          testID="profile-type-coach-and-athlete"
        />
      </View>
      {saveFailed ? (
        <Text
          accessibilityRole="alert"
          testID="profile-type-save-failed"
          className="mt-4 px-4 text-sm text-error"
        >
          {t('onboarding.profileTypeSaveFailed')}
        </Text>
      ) : null}
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
