// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Home's one strip for an invite the server holds for an active account that has not joined: who invited, the join, or the full group (carnet#1650)
// ABOUTME: The join the page makes by itself leaves a hold behind when the group is full or the call failed; this is where the athlete finishes it

import { useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { inviteShares, joinStripFull, joinStripJoin, joinStripTitle } from '@pierre/ui-logic';
import { useTranslation } from '@pierre/i18n';
import type { JoinHeldInviteResponse } from '@pierre/shared-types';
import { groupsApi } from '../../services/api';
import { QUERY_KEYS } from '../../constants/queryKeys';
import { useAuth } from '../../hooks/useAuth';
import { useServerHeldInvite } from '../../hooks/useInviteJourney';
import { useInviteCardOpen, useInviteSettling } from '../../utils/groupInvite';
import { Button, useSuccessToast, useToast } from '../ui';
import type { TodayActions } from './personalToday';

interface HeldInviteStripProps {
  actions: TodayActions;
  /** Dashboard route navigator, `tab[/subview]`. */
  onNavigate: (route: string) => void;
}

/** What the last press came to, beyond what the server holds. */
type JoinAttempt = 'none' | 'full' | 'failed';

/**
 * The invite the server holds for the signed-in account, once the account is
 * active and the page is not joining it itself. Nothing shows for no hold, a
 * pending account (the waiting screen keeps the promise), or while the page's
 * own hold and join are still in flight.
 */
export function HeldInviteStrip({ actions, onNavigate }: HeldInviteStripProps) {
  const { t } = useTranslation();
  const { user, impersonation } = useAuth();
  const queryClient = useQueryClient();
  const showSuccess = useSuccessToast();
  const { addToast } = useToast();
  const held = useServerHeldInvite().data?.held ?? null;
  const settling = useInviteSettling();
  const cardOpen = useInviteCardOpen();
  const [attempt, setAttempt] = useState<JoinAttempt>('none');

  const join = useMutation({
    mutationFn: () => groupsApi.joinHeldInvite(),
    onSuccess: (result: JoinHeldInviteResponse) => {
      void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groupInvite.held() });
      switch (result.outcome) {
        case 'joined':
          void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
          void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.all });
          showSuccess(t('humanCoach.joinStrip.joined', { group: result.group_name }));
          // The room opens as it does after the page's own join; a narrow
          // screen's Today sheet steps aside first.
          actions.dismiss();
          onNavigate(
            result.conversation_id === null ? 'chat' : `chat/${encodeURIComponent(result.conversation_id)}`,
          );
          break;
        case 'refused':
          if (result.reason === 'group_full') {
            setAttempt('full');
          } else {
            // A refusal for good drops the hold, and this strip with it: the
            // server's own words say why, as the page's own join does.
            addToast({ type: 'error', title: result.message });
          }
          break;
        case 'already_member':
        case 'none':
          break;
      }
    },
    onError: () => setAttempt('failed'),
  });

  if (
    held === null ||
    user?.user_status !== 'active' ||
    impersonation.isImpersonating ||
    settling ||
    cardOpen
  ) {
    return null;
  }

  const title = joinStripTitle(held);
  const shares = inviteShares(held);
  const fullWords = joinStripFull(held);
  const joinWords = joinStripJoin(held);
  const full = attempt === 'full' || (attempt === 'none' && held.state === 'group_full');
  const press = () => {
    setAttempt('none');
    join.mutate();
  };

  return (
    <section
      className="rounded-[10px] bg-surface-container-low px-3 py-3"
      aria-label={t(title.key, title.values)}
      data-testid="held-invite-strip"
      data-state={full ? 'group_full' : 'pending'}
    >
      <p className="text-sm font-semibold text-on-surface">
        {t(title.key, title.values)}
      </p>
      <p className="mt-1 text-xs text-on-surface-variant">
        {t(shares.key, shares.values)}
      </p>
      {full && (
        <p className="mt-2 text-sm text-on-warning-container" data-testid="held-invite-strip-full">
          {t(fullWords.key, fullWords.values)}
        </p>
      )}
      {attempt === 'failed' && (
        <p className="mt-2 text-sm text-error" role="alert" data-testid="held-invite-strip-failed">
          {t('humanCoach.joinStrip.failed')}
        </p>
      )}
      <div className="mt-3">
        <Button
          size="sm"
          variant="primary"
          loading={join.isPending}
          onClick={press}
          data-testid="held-invite-strip-join"
        >
          {join.isPending
            ? t('humanCoach.joinStrip.joining')
            : full || attempt === 'failed'
              ? t('humanCoach.joinStrip.retry')
              : t(joinWords.key, joinWords.values)}
        </Button>
      </div>
    </section>
  );
}
