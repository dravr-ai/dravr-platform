// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One-tap coach-access request for a coachless group the coach owns — onboarding step and Group info alike
// ABOUTME: Shows the latest request's state (pending, declined) from the server; asking grants nothing (ADR-018)

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import { userApi } from '../services/api';
import { QUERY_KEYS } from '../constants/queryKeys';
import { Button } from './ui';

/**
 * The one-tap coach-access request for a coachless group (carnet#738).
 *
 * Reads where the coach's latest request stands so a return visit shows it:
 * nothing asked, or a declined request, offers the button; a pending one says
 * it was sent. The request names this group, so a grant attaches the coach to
 * it. Asking grants nothing (ADR-018): a super-admin decides. One component
 * serves the onboarding group step and Group info, so the two cannot drift.
 */
export default function CoachAccessRequest({ groupId }: { groupId: string }) {
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
      <p className="mt-3 text-sm text-on-surface" role="status" data-testid="coach-access-pending">
        {t('humanCoach.requestAccessPending')}
      </p>
    );
  }

  return (
    <div className="mt-3 space-y-2">
      {status === 'declined' ? (
        <p className="text-sm text-on-surface-variant" data-testid="coach-access-declined">
          {t('humanCoach.requestAccessDeclined')}
        </p>
      ) : null}
      <Button
        variant="secondary"
        onClick={() => ask.mutate()}
        disabled={ask.isPending || latest.isLoading}
        data-testid="coach-access-request"
      >
        {ask.isPending ? t('humanCoach.requestAccessSending') : t('humanCoach.requestAccess')}
      </Button>
      {ask.isError ? (
        <p className="text-sm text-error" role="alert">
          {t('humanCoach.requestAccessFailed')}
        </p>
      ) : null}
    </div>
  );
}
