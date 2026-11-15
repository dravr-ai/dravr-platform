// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The health-check sheet on a phone — the PAR-Q+ « Construire mon plan » opens first while it is still to answer
// ABOUTME: Answers through the same endpoint as the onboarding screen, writes the parq step complete, then runs what waited (carnet#1530)

import React from 'react';
import { ActivityIndicator, Pressable, ScrollView, Text, View } from 'react-native';
import { useQueryClient } from '@tanstack/react-query';
import type { OnboardingStatusResponse } from '@pierre/api-client';
import type { ParqAnswer } from '@pierre/shared-types';
import { QUERY_KEYS, withStepRecorded } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { userApi } from '../services/api';
import { useThemeColors } from '../constants/theme';
import { Sheet } from './ui';
import { ParqForm, useParqQuestions } from './ParqForm';

interface HealthCheckSheetProps {
  visible: boolean;
  onClose: () => void;
  /** Runs once the answers and the step row are stored — never when the sheet is only closed. */
  onAnswered: () => void;
}

/**
 * No plan before the health check (carnet#1530): a PAR-Q put off at
 * onboarding is asked here, the phone's « Bilan de santé ». Both writes are
 * waited on, so a refusal keeps the sheet open with its error; closing the
 * sheet answers nothing and the plan waits.
 */
export function HealthCheckSheet({ visible, onClose, onAnswered }: HealthCheckSheetProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const queryClient = useQueryClient();
  const { data, isLoading, isError, refetch } = useParqQuestions();

  const answer = async (answers: ParqAnswer[]) => {
    await userApi.submitParq(answers);
    await userApi.setOnboardingStep('parq', 'complete');
    queryClient.setQueryData<OnboardingStatusResponse>(
      QUERY_KEYS.user.onboardingStatus(),
      (cached) => cached && { ...cached, steps: withStepRecorded(cached.steps, 'parq', 'complete') },
    );
    void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.user.onboardingStatus() });
    void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.home.gettingStarted() });
    onClose();
    onAnswered();
  };

  return (
    <Sheet visible={visible} onClose={onClose} maxHeight="max-h-[92%]" testID="health-check-sheet">
      <ScrollView>
        <Text className="text-lg font-semibold text-text-primary" accessibilityRole="header">
          {t('home.gettingStarted.rows.healthCheck.title')}
        </Text>
        {isLoading ? (
          <View className="py-8 items-center">
            <ActivityIndicator color={colors.tokens.primary} />
          </View>
        ) : isError || !data ? (
          <View className="py-4" accessibilityLiveRegion="polite">
            <Text className="text-sm text-text-primary">{t('onboarding.parq.loadFailed')}</Text>
            <Pressable onPress={() => void refetch()} accessibilityRole="button" className="min-h-11 justify-center">
              <Text className="text-sm font-medium text-primary">{t('common.retry')}</Text>
            </Pressable>
          </View>
        ) : (
          <>
            <Text className="mt-2 mb-4 text-sm text-text-secondary">{t('onboarding.parq.intro')}</Text>
            <ParqForm questions={data.questions} onSubmit={answer} />
          </>
        )}
      </ScrollView>
    </Sheet>
  );
}
