// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Group info's « this group has no coach yet » block — the owner who holds coach access becomes the group's coach in one press
// ABOUTME: Drawn only for the owner of a coachless group whose session user manages a roster (carnet#1650); the server decides the rest

import React from 'react';
import { Text, View } from 'react-native';
import Toast from 'react-native-toast-message';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { Button } from '../../components/ui';
import { groupsApi } from '../../services/api';

export interface ClaimCoachBlockProps {
  groupId: string;
}

/**
 * The owner's way to coach their own group: what is missing, what becoming
 * its coach shows them, and the one filled button. The coach attaches through
 * `PUT /api/groups/:id/coach`; the group and the conversations are then read
 * again, so the roster line names them and the block goes away.
 */
export function ClaimCoachBlock({ groupId }: ClaimCoachBlockProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const claim = useMutation({
    mutationFn: () => groupsApi.claimCoach(groupId),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.all });
      void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
      Toast.show({ type: 'success', text1: t('humanCoach.claimCoach.done') });
    },
    onError: () => {
      Toast.show({ type: 'error', text1: t('common.error'), text2: t('humanCoach.claimCoach.failed') });
    },
  });

  return (
    <View className="mt-4 gap-2 rounded-lg border border-border-faint bg-surface-container-low p-4" testID="group-info-claim-coach">
      <Text accessibilityRole="header" className="text-sm font-semibold text-text-primary">
        {t('humanCoach.claimCoach.none')}
      </Text>
      <Text className="text-sm text-text-secondary">{t('humanCoach.claimCoach.body')}</Text>
      <Button
        title={claim.isPending ? t('humanCoach.claimCoach.claiming') : t('humanCoach.claimCoach.action')}
        onPress={() => claim.mutate()}
        disabled={claim.isPending}
        fullWidth
        testID="group-info-claim-coach-button"
      />
    </View>
  );
}
