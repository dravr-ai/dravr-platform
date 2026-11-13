// ABOUTME: First-run onboarding step (mobile) — asks whether the user is an athlete, a coach, or both
// ABOUTME: Mirrors the web OnboardingProfileType; records the coaching role, never the style; coach-only drops the athlete steps

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState } from 'react';
import { View, Text, ScrollView } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { Feather } from '@expo/vector-icons';
import { useQueryClient } from '@tanstack/react-query';
import type { OnboardingStatusResponse } from '@pierre/api-client';
import { Button, Row } from '../../components/ui';
import { OnboardingProgressBar } from '../../components/ui/OnboardingProgressBar';
import { userApi } from '../../services/api';
import { useAuth } from '../../contexts/AuthContext';
import { useProfileTypeChosen } from '../../hooks/useProfileTypeChosen';
import { ATHLETE_STEPS_WAIVED_PREFIX, useOnboardingFlag } from '../../hooks/useOnboardingFlag';
import { useOnboardingProgress } from '../../hooks/useOnboardingProgress';
import { useThemeColors } from '../../constants/theme';
import { ATHLETE_STEP_IDS, QUERY_KEYS, type OnboardingProgressItem } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';

/** The three answers the step offers. */
type ProfileChoice = 'athlete' | 'coach' | 'coach_and_athlete';

/**
 * Athlete / coach / both onboarding step (mobile).
 *
 * Reached via RootLayoutNav for a fresh account before the connect-provider step.
 * The answer records a role — whether the user coaches others — never the
 * reply style, which stays the user's own under Settings → Coaching style
 * (carnet#827). The athlete answer records the role as off. A coach who does not
 * train also takes the athlete steps (about-you, PAR-Q) out of the journey —
 * locally, and as `not_applicable` step rows on the server — before the
 * profile-type step is marked done, so the routing gate never lands them on
 * an athlete question. Mirrors the web OnboardingProfileType.
 *
 * A tap selects; the answer registers on Continue, so a mis-tap is corrected
 * by tapping another row rather than by a journey started in error.
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
  const queryClient = useQueryClient();
  const [selected, setSelected] = useState<ProfileChoice | null>(null);
  const [choosing, setChoosing] = useState<ProfileChoice | null>(null);
  const [saveFailed, setSaveFailed] = useState(false);

  const finish = async (choice: ProfileChoice) => {
    if (choosing) return;
    setChoosing(choice);
    setSaveFailed(false);
    const coachesOthers = choice !== 'athlete';
    try {
      await userApi.setCoachingRole(coachesOthers);
    } catch {
      // The role decides which journey and toolset follow: keep the user
      // here to retry rather than route them on without it recorded.
      setChoosing(null);
      setSaveFailed(true);
      return;
    }
    // The role picks the journey, and the progress hairline's segments with
    // it: the cached status carries it before the next screen draws.
    queryClient.setQueryData<OnboardingStatusResponse>(
      QUERY_KEYS.user.onboardingStatus(),
      (cached) => cached && { ...cached, coaches_others: coachesOthers },
    );
    if (choice === 'coach') {
      for (const stepId of ATHLETE_STEP_IDS) {
        userApi.setOnboardingStep(stepId, 'not_applicable').catch(() => {});
      }
      await markAthleteStepsWaived();
    }
    userApi.setOnboardingStep('profile_type', 'complete').catch(() => {});
    // The status carries `coaches_others`, which puts the coach steps in the
    // journey: it is read again before the gate decides the next step.
    await queryClient.invalidateQueries({ queryKey: QUERY_KEYS.user.onboardingStatus() }).catch(() => {});
    await markChosen();
  };

  const check = (choice: ProfileChoice) =>
    selected === choice ? <Feather name="check" size={18} color={colors.tokens.primary} /> : undefined;
  const choose = (choice: ProfileChoice) => {
    if (!choosing) setSelected(choice);
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
          trailing={check('athlete')}
          showChevron={false}
          onPress={() => choose('athlete')}
          accessibilityLabel={t('app.imAnAthlete')}
          accessibilityRole="radio"
          accessibilityState={{ selected: selected === 'athlete' }}
          testID="profile-type-athlete"
        />
        <Row
          title={t('humanCoach.iCoachOthers')}
          subtitle={t('humanCoach.cardDescription')}
          trailing={check('coach')}
          showChevron={false}
          onPress={() => choose('coach')}
          accessibilityLabel={t('humanCoach.iCoachOthers')}
          accessibilityRole="radio"
          accessibilityState={{ selected: selected === 'coach' }}
          testID="profile-type-coach"
        />
        <Row
          title={t('humanCoach.iCoachAndTrain')}
          subtitle={t('humanCoach.coachAndTrainCardDescription')}
          trailing={check('coach_and_athlete')}
          showChevron={false}
          onPress={() => choose('coach_and_athlete')}
          accessibilityLabel={t('humanCoach.iCoachAndTrain')}
          accessibilityRole="radio"
          accessibilityState={{ selected: selected === 'coach_and_athlete' }}
          last
          testID="profile-type-coach-and-athlete"
        />
      </View>
      <View className="mt-8 px-4">
        <Button
          title={choosing ? t('onboarding.settingUp') : t('onboarding.continueButton')}
          onPress={() => {
            if (selected) void finish(selected);
          }}
          disabled={selected === null || choosing !== null}
          loading={choosing !== null}
          fullWidth
          testID="profile-type-continue"
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
