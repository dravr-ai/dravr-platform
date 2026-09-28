// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Store group of the coach editor — submit one of the athlete's own agents to the admin review queue
// ABOUTME: One ink action; the confirmation or the server's refusal is said in place, under it

import React from 'react';
import { Text, TouchableOpacity, ActivityIndicator, View } from 'react-native';
import { useMutation } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import { describeApiError } from '@pierre/ui-logic';
import { coachesApi } from '../../services/api';
import { Section } from '../../components/ui';
import { useThemeColors } from '../../constants/theme';

export interface CoachStoreSubmitProps {
  agentId: string;
}

export function CoachStoreSubmit({ agentId }: CoachStoreSubmitProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const submit = useMutation({ mutationFn: () => coachesApi.submitToStore(agentId) });

  return (
    <Section
      title={t('discover.submitToStore')}
      description={t('discover.submitToStoreHint')}
      className="mt-6 -mx-4"
      testID="agent-store-submit"
    >
      <View className="px-4 py-3">
        {submit.isSuccess ? (
          <Text className="text-sm text-text-secondary" testID="agent-store-submitted">
            {t('discover.submittedToStore')}
          </Text>
        ) : submit.isPending ? (
          <ActivityIndicator size="small" color={colors.tokens.primary} />
        ) : (
          <TouchableOpacity
            className="py-2"
            accessibilityRole="button"
            onPress={() => submit.mutate()}
            testID="agent-store-submit-button"
          >
            <Text className="text-sm font-semibold text-primary">{t('discover.submitToStore')}</Text>
          </TouchableOpacity>
        )}
        {submit.isError && (
          <Text className="text-sm text-error mt-1" accessibilityRole="alert" testID="agent-store-submit-error">
            {describeApiError(submit.error, { t, fallbackKey: 'discover.submitToStoreFailed' })}
          </Text>
        )}
      </View>
    </Section>
  );
}
