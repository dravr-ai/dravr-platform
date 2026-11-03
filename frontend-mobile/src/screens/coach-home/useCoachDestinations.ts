// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Where each of Coach Home's actions goes on the phone — the group's room, or the coach's plan thread on the athlete
// ABOUTME: Each action opens a screen the app has: the group's room (found or opened by the server, once per tap), the plan thread, or the reply review

import { useCallback } from 'react';
import { Alert } from 'react-native';
import { useRouter } from 'expo-router';
import type { CoachAttentionAction, CoachAttentionReason, CoachRosterAthlete } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { GROUP_ROOM_KEYS } from '@pierre/chat-utils';
import { openThreadRefusalText } from '@pierre/ui-logic';
import { useOpenAthleteThread, useOpenGroupRoom } from '../../hooks/useCoachPlan';
import { coachRepliesHref, threadHref } from '../../navigation/routes';

/** The group an action about `athlete` acts in: the one the reason names, else the athlete's first. */
export function actionGroupId(athlete: CoachRosterAthlete, reason: CoachAttentionReason | null): string | null {
  return reason?.target.group_id ?? athlete.groups[0]?.id ?? null;
}

/** What the actions do on the athlete they are about. */
export interface CoachDestinations {
  /** Open the group's room. */
  writeToGroup: (athlete: CoachRosterAthlete, reason: CoachAttentionReason | null) => void;
  /** Run the reason's primary action. */
  perform: (action: CoachAttentionAction, athlete: CoachRosterAthlete, reason: CoachAttentionReason) => void;
  /** Open the human coach's reply review on the athlete. */
  openReplies: (athlete: CoachRosterAthlete) => void;
  /** Open the coach's plan thread on the athlete. */
  openPlanThread: (athlete: CoachRosterAthlete, reason: CoachAttentionReason | null) => void;
  /** The athlete whose plan thread is opening, while one is. */
  pendingAthleteId: string | null;
  /** The athlete whose group room is opening, while one is: its buttons wait and say so. */
  pendingRoomAthleteId: string | null;
}

/**
 * The destinations of Coach Home's actions. A group's room is the
 * conversation the caller holds in that group, which the server finds or
 * opens in one call, so a second tap lands in the same room. While a room
 * is opening every other room tap is ignored; a room or plan thread that
 * cannot be opened says why in an alert, never by dropping the coach on a
 * list that does not hold it.
 */
export function useCoachDestinations(): CoachDestinations {
  const { t } = useTranslation();
  const router = useRouter();
  const { openThread, pendingAthleteId } = useOpenAthleteThread();
  const { openRoom, pendingRoomAthleteId } = useOpenGroupRoom();

  const writeToGroup = useCallback(
    (athlete: CoachRosterAthlete, reason: CoachAttentionReason | null) => {
      const groupId = actionGroupId(athlete, reason);
      if (groupId === null) {
        Alert.alert(t('common.error'), t(GROUP_ROOM_KEYS.openError));
        return;
      }
      openRoom(athlete.user_id, groupId)?.then(
        (room) => router.push(threadHref(room.id)),
        () => Alert.alert(t('common.error'), t(GROUP_ROOM_KEYS.openError)),
      );
    },
    [openRoom, router, t],
  );

  const openPlanThread = useCallback(
    (athlete: CoachRosterAthlete, reason: CoachAttentionReason | null) => {
      const groupId = actionGroupId(athlete, reason);
      if (groupId === null) {
        Alert.alert(t('common.error'), t('humanCoach.planThread.openError'));
        return;
      }
      openThread(athlete.user_id, groupId).then(
        (thread) => router.push(threadHref(thread.id)),
        (error: unknown) => Alert.alert(t('common.error'), openThreadRefusalText(error, athlete.name, t)),
      );
    },
    [openThread, router, t],
  );

  const openReplies = useCallback(
    (athlete: CoachRosterAthlete) => router.push(coachRepliesHref(athlete.user_id)),
    [router],
  );

  const perform = useCallback(
    (action: CoachAttentionAction, athlete: CoachRosterAthlete, reason: CoachAttentionReason) => {
      switch (action) {
        case 'write_to_athlete':
          writeToGroup(athlete, reason);
          return;
        case 'correct_reply':
          openReplies(athlete);
          return;
        case 'open_plan_thread':
        case 'review_draft':
        case 'create_plan':
          openPlanThread(athlete, reason);
          return;
      }
    },
    [openPlanThread, openReplies, writeToGroup],
  );

  return { writeToGroup, perform, openReplies, openPlanThread, pendingAthleteId, pendingRoomAthleteId };
}
