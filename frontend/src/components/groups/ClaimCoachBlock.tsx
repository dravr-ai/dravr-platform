// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Group info's offer to the owner of a coachless group who holds coach access: become its coach, one filled button (carnet#1650)
// ABOUTME: The seat request for an owner without coach access stays where it is; this is for the owner who can already coach

import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import { groupsApi } from '../../services/api';
import { QUERY_KEYS } from '../../constants/queryKeys';
import { Button, useSuccessToast } from '../ui';

/**
 * « Ce groupe n'a pas encore de coach » and the way to become it. The server
 * attaches the caller as the group's human coach (`PUT /api/groups/:id/coach`);
 * the group's queries are re-read, so the row beneath names them at once.
 */
export default function ClaimCoachBlock({ groupId }: { groupId: string }) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const showSuccess = useSuccessToast();
  const claim = useMutation({
    mutationFn: () => groupsApi.claimCoach(groupId),
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.detail(groupId) }),
        queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() }),
        queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coachRoster.all }),
      ]);
      showSuccess(t('humanCoach.claimCoach.done'));
    },
  });

  return (
    <div
      className="space-y-3 rounded-[10px] bg-surface-container-low px-4 py-4"
      data-testid="group-info-claim-coach"
    >
      <div>
        <p className="text-sm font-semibold text-on-surface">{t('humanCoach.claimCoach.none')}</p>
        <p className="mt-1 text-sm text-on-surface-variant">{t('humanCoach.claimCoach.body')}</p>
      </div>
      {claim.isError && (
        <p className="text-sm text-error" role="alert" data-testid="group-info-claim-coach-failed">
          {t('humanCoach.claimCoach.failed')}
        </p>
      )}
      <Button
        variant="primary"
        loading={claim.isPending}
        onClick={() => claim.mutate()}
        data-testid="group-info-claim-coach-action"
      >
        {claim.isPending ? t('humanCoach.claimCoach.claiming') : t('humanCoach.claimCoach.action')}
      </Button>
    </div>
  );
}
