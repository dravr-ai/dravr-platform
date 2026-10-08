// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One-tap coach-access request (mobile) for a coachless group the coach owns — onboarding step and Group info
// ABOUTME: Mirrors the web CoachAccessRequest; shows the latest request's state, and asking grants nothing (ADR-018)

import React from 'react';
import { View, Text } from 'react-native';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { Button } from './ui';
import { userApi } from '../services/api';

/**
 * The one-tap coach-access request for a coachless group (carnet#738),
 * mirroring the web: nothing asked or a declined request offers the button, a
 * pending one says it was sent. The request names this group so a grant
 * attaches the coach to it; asking grants nothing (ADR-018). One component
 * serves the onboarding group step and Group info, so the two cannot drift.
 */
export function CoachAccessRequest({ groupId }: { groupId: string }) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const latest = useQuery({
    queryKey: QUERY_KEYS.user.coachAccessRequest(),
    queryFn: () => userApi.getCoachAccessRequest(),
  });
  const ask = useMutation({
    mutationFn: () => userApi.requestCoachAccess(groupId),
    onSuccess: (data) => {
      queryClient.setQueryData(QUERY_KEYS.user.coachAccessRequest(), data);
    },
  });

  const status = latest.data?.request?.status;
  if (status === 'pending') {
    return (
      <Text className="mt-3 text-sm text-on-surface" testID="coach-access-pending">
        {t('humanCoach.requestAccessPending')}
      </Text>
    );
  }

  return (
    <View className="mt-3 gap-2">
      {status === 'declined' ? (
        <Text className="text-sm text-on-surface-variant" testID="coach-access-declined">
          {t('humanCoach.requestAccessDeclined')}
        </Text>
      ) : null}
      <Button
        title={ask.isPending ? t('humanCoach.requestAccessSending') : t('humanCoach.requestAccess')}
        variant="secondary"
        onPress={() => ask.mutate()}
        disabled={ask.isPending || latest.isLoading}
        testID="coach-access-request"
      />
      {ask.isError ? (
        <Text className="text-sm text-error" accessibilityRole="alert" testID="coach-access-failed">
          {t('humanCoach.requestAccessFailed')}
        </Text>
      ) : null}
    </View>
  );
}
