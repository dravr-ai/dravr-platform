// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Where each of Coach Home's actions goes — the group room, the coach's plan thread on an athlete, the reply review
// ABOUTME: Every action reaches a real screen; a refused plan thread says why in a toast instead of doing nothing

import { useCallback } from 'react';
import type { CoachAttentionAction, CoachAttentionReason, CoachRosterAthlete } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { openThreadRefusalText } from '@pierre/ui-logic';
import { useConversationsQuery } from '../../hooks/useConversationList';
import { useOpenAthleteThread } from '../../hooks/useCoachPlan';
import { useErrorToast } from '../ui';

/** The group an action about `athlete` acts in: the one the reason names, else the athlete's first. */
function actionGroupId(athlete: CoachRosterAthlete, reason: CoachAttentionReason | null): string | null {
  return reason?.target.group_id ?? athlete.groups[0]?.id ?? null;
}

/** What the actions do on the athlete they are about. */
export interface CoachDestinations {
  /** Open the group's room. */
  writeToGroup: (athlete: CoachRosterAthlete, reason: CoachAttentionReason | null) => void;
  /** Run the reason's primary action. */
  perform: (action: CoachAttentionAction, athlete: CoachRosterAthlete, reason: CoachAttentionReason) => void;
  /** Open the coach's plan thread on the athlete. */
  openPlanThread: (athlete: CoachRosterAthlete, reason: CoachAttentionReason | null) => void;
  /** The athlete whose plan thread is opening, while one is. */
  pendingAthleteId: string | null;
  /** The primary action goes to the group room, so "Écrire dans le groupe" would repeat it. */
  primaryIsGroupRoom: (action: CoachAttentionAction) => boolean;
}

/**
 * The destinations of Coach Home's actions. `onNavigate` is the dashboard's
 * `tab[/subview]` navigator. A group's room is the conversation the caller
 * holds in that group; without one in the list the Groups tab opens instead.
 */
export function useCoachDestinations(onNavigate: (route: string) => void): CoachDestinations {
  const { t } = useTranslation();
  const conversations = useConversationsQuery(true);
  const { openThread, pendingAthleteId } = useOpenAthleteThread();
  const showError = useErrorToast();

  const writeToGroup = useCallback(
    (athlete: CoachRosterAthlete, reason: CoachAttentionReason | null) => {
      const groupId = actionGroupId(athlete, reason);
      const room =
        groupId === null
          ? undefined
          : conversations.data?.pages
              .flatMap((page) => page.conversations)
              .find((conversation) => conversation.group_id === groupId);
      onNavigate(room === undefined ? 'chat' : `chat/${encodeURIComponent(room.id)}`);
    },
    [conversations.data, onNavigate],
  );

  const openPlanThread = useCallback(
    (athlete: CoachRosterAthlete, reason: CoachAttentionReason | null) => {
      const groupId = actionGroupId(athlete, reason);
      if (groupId === null) {
        onNavigate('chat');
        return;
      }
      openThread(athlete.user_id, groupId).then(
        (thread) => onNavigate(`chat/${encodeURIComponent(thread.id)}`),
        (error: unknown) => showError(openThreadRefusalText(error, athlete.name, t)),
      );
    },
    [onNavigate, openThread, showError, t],
  );

  const perform = useCallback(
    (action: CoachAttentionAction, athlete: CoachRosterAthlete, reason: CoachAttentionReason) => {
      switch (action) {
        case 'write_to_athlete':
          writeToGroup(athlete, reason);
          return;
        case 'correct_reply':
          onNavigate(`coach-replies/${encodeURIComponent(athlete.user_id)}`);
          return;
        case 'open_plan_thread':
        case 'review_draft':
        case 'create_plan':
          openPlanThread(athlete, reason);
          return;
      }
    },
    [onNavigate, openPlanThread, writeToGroup],
  );

  return {
    writeToGroup,
    perform,
    openPlanThread,
    pendingAthleteId,
    primaryIsGroupRoom: (action) => action === 'write_to_athlete',
  };
}
