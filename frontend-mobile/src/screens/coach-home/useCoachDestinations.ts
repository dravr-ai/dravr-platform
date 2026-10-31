// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Where each of Coach Home's actions goes on the phone — the group's room, or the coach's plan thread on the athlete
// ABOUTME: Only destinations the app has: a reply review does not exist on mobile, so its action is reported as unavailable, never faked

import { useCallback } from 'react';
import { Alert } from 'react-native';
import { useRouter } from 'expo-router';
import type { CoachAttentionAction, CoachAttentionReason, CoachRosterAthlete } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { openThreadRefusalText } from '@pierre/ui-logic';
import { useConversationRows } from '../conversations/useConversationList';
import { useOpenAthleteThread } from '../../hooks/useCoachPlan';
import { CHAT_LIST_ROUTE, threadHref } from '../../navigation/routes';

/** The group an action about `athlete` acts in: the one the reason names, else the athlete's first. */
export function actionGroupId(athlete: CoachRosterAthlete, reason: CoachAttentionReason | null): string | null {
  return reason?.target.group_id ?? athlete.groups[0]?.id ?? null;
}

/**
 * The actions a reason names that this app can carry out.
 *
 * LIMITATION(registre#1304): `actionAvailableOnPhone` refuses `correct_reply`: the human coach's reply review is web-only, so the phone offers the group's room instead.
 */
export function actionAvailableOnPhone(action: CoachAttentionAction): boolean {
  return action !== 'correct_reply';
}

/** What the actions do on the athlete they are about. */
export interface CoachDestinations {
  /** Open the group's room. */
  writeToGroup: (athlete: CoachRosterAthlete, reason: CoachAttentionReason | null) => void;
  /** Run the reason's primary action; a no-op for one the phone does not have. */
  perform: (action: CoachAttentionAction, athlete: CoachRosterAthlete, reason: CoachAttentionReason) => void;
  /** Open the coach's plan thread on the athlete. */
  openPlanThread: (athlete: CoachRosterAthlete, reason: CoachAttentionReason | null) => void;
  /** The athlete whose plan thread is opening, while one is. */
  pendingAthleteId: string | null;
}

/**
 * The destinations of Coach Home's actions. A group's room is the
 * conversation the caller holds in that group; with none in the list the Chat
 * tab opens instead. A refused plan thread says why in an alert.
 */
export function useCoachDestinations(): CoachDestinations {
  const { t } = useTranslation();
  const router = useRouter();
  const { conversations } = useConversationRows();
  const { openThread, pendingAthleteId } = useOpenAthleteThread();

  const writeToGroup = useCallback(
    (athlete: CoachRosterAthlete, reason: CoachAttentionReason | null) => {
      const groupId = actionGroupId(athlete, reason);
      const room = groupId === null ? undefined : conversations.find((c) => c.group_id === groupId);
      router.push(room === undefined ? CHAT_LIST_ROUTE : threadHref(room.id));
    },
    [conversations, router],
  );

  const openPlanThread = useCallback(
    (athlete: CoachRosterAthlete, reason: CoachAttentionReason | null) => {
      const groupId = actionGroupId(athlete, reason);
      if (groupId === null) {
        router.push(CHAT_LIST_ROUTE);
        return;
      }
      openThread(athlete.user_id, groupId).then(
        (thread) => router.push(threadHref(thread.id)),
        (error: unknown) => Alert.alert(t('common.error'), openThreadRefusalText(error, athlete.name, t)),
      );
    },
    [openThread, router, t],
  );

  const perform = useCallback(
    (action: CoachAttentionAction, athlete: CoachRosterAthlete, reason: CoachAttentionReason) => {
      switch (action) {
        case 'write_to_athlete':
          writeToGroup(athlete, reason);
          return;
        case 'correct_reply':
          return;
        case 'open_plan_thread':
        case 'review_draft':
        case 'create_plan':
          openPlanThread(athlete, reason);
          return;
      }
    },
    [openPlanThread, writeToGroup],
  );

  return { writeToGroup, perform, openPlanThread, pendingAthleteId };
}
