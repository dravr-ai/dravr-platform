// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The two threads an athlete workspace draws — the coach's plan thread on the athlete, and the athlete's own room with the coach
// ABOUTME: Each is found among what the coach holds; the plan thread is opened when none is, the room only when the coach asks (carnet#1816)

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useQueries } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { CoachingGroup, CoachRosterAthlete } from '@pierre/shared-types';
import { athleteRoomGroup, roomConversationOf, usablePlanThread } from '@pierre/ui-logic';
import { groupsApi } from '../../../services/api';
import { useAuth } from '../../../hooks/useAuth';
import { useConversationList } from '../../../hooks/useConversationList';
import { useOpenAthleteThread, useOpenGroupRoom } from '../../../hooks/useCoachPlan';

/** How long a group read stays fresh: the web's `useGroup` freshness, which shares the cache entry. */
const GROUP_STALE_MS = 30_000;

/** The athlete's own room with the coach. */
export interface WorkspaceRoom {
  group: CoachingGroup;
  /** The coach's conversation in it; `null` until the coach holds one. */
  conversationId: string | null;
}

/** What the workspace knows of its two threads. */
export interface WorkspaceThreads {
  /** The coach's plan thread on the athlete; `null` while it is found or opened. */
  planThreadId: string | null;
  /** The group the plan thread acts under; `null` for an athlete in no group of the coach's. */
  planGroup: CoachingGroup | null;
  /** Why the plan thread could not be opened, until a retry. */
  planError: unknown;
  retryPlanThread: () => void;
  /** The athlete's own room, once the groups are read; `null` when every group of theirs holds other athletes. */
  room: WorkspaceRoom | null;
  /** The groups have been read, so `room` is known. */
  roomKnown: boolean;
  /** Open the coach's conversation in the room, which the server finds or opens. */
  openRoom: () => void;
  roomOpening: boolean;
  /** The room could not be opened. */
  roomError: boolean;
}

/**
 * The plan thread and the room of `athlete`'s workspace.
 *
 * The plan thread is bound to the athlete through a group (carnet#1070): the
 * athlete's own room's group when they have one, so the thread is the one
 * that room's private lane writes to, else their first group. One found in
 * the coach's list is used as it is; otherwise it is opened once — the server
 * answers the existing thread, un-archived, or a new one — because the
 * workspace reads and writes the plan through it.
 *
 * The room is the group whose one athlete the server names to this coach as
 * them. The coach's conversation in it is found in the list and opened only
 * when the coach asks, so looking at the workspace never creates one.
 */
export function useWorkspaceThreads(athlete: CoachRosterAthlete): WorkspaceThreads {
  const { user } = useAuth();
  const viewerId = user?.id ?? null;
  const { conversations, isLoading: listLoading } = useConversationList();
  const { openThread } = useOpenAthleteThread();
  const { openRoom: openGroupRoom, pendingRoomAthleteId } = useOpenGroupRoom();

  const groupReads = useQueries({
    queries: athlete.groups.map((group) => ({
      queryKey: QUERY_KEYS.groups.detail(group.id),
      queryFn: () => groupsApi.getGroup(group.id),
      staleTime: GROUP_STALE_MS,
    })),
  });
  const groups = useMemo(
    () => groupReads.map((read) => read.data).filter((group): group is CoachingGroup => group !== undefined),
    [groupReads],
  );
  const roomKnown = groupReads.every((read) => !read.isLoading);

  const roomGroup = useMemo(
    () => athleteRoomGroup(groups, athlete.user_id, viewerId),
    [groups, athlete.user_id, viewerId],
  );
  const planGroupId = roomGroup?.id ?? athlete.groups[0]?.id ?? null;
  const planGroup = groups.find((group) => group.id === planGroupId) ?? null;

  // A thread opened here, held with the athlete and group it was opened for,
  // so another athlete's workspace mounted in its place never inherits it.
  const [opened, setOpened] = useState<{ athleteId: string; groupId: string; id: string } | null>(null);
  const [planError, setPlanError] = useState<unknown>(null);
  // A retry asks again: it clears what was asked and moves the effect on.
  const [attempt, setAttempt] = useState(0);
  const asked = useRef<string | null>(null);

  const found = planGroupId === null ? null : usablePlanThread(conversations, athlete.user_id, planGroupId);
  const openedId =
    opened !== null && opened.athleteId === athlete.user_id && opened.groupId === planGroupId ? opened.id : null;
  const planThreadId = found?.id ?? openedId;

  // Opened once per athlete and group, after the list and the groups are read,
  // so a thread the coach already holds is never opened a second time.
  useEffect(() => {
    if (planThreadId !== null || planGroupId === null || listLoading || !roomKnown) return;
    const key = `${athlete.user_id}:${planGroupId}`;
    if (asked.current === key) return;
    asked.current = key;
    openThread(athlete.user_id, planGroupId).then(
      (thread) => setOpened({ athleteId: athlete.user_id, groupId: planGroupId, id: thread.id }),
      (error: unknown) => setPlanError(error),
    );
  }, [planThreadId, planGroupId, listLoading, roomKnown, athlete.user_id, openThread, attempt]);

  const retryPlanThread = useCallback(() => {
    asked.current = null;
    setPlanError(null);
    setAttempt((count) => count + 1);
  }, []);

  const [openedRoom, setOpenedRoom] = useState<{ groupId: string; id: string } | null>(null);
  const [roomError, setRoomError] = useState(false);
  const roomConversation = roomGroup === null ? null : roomConversationOf(conversations, roomGroup.id);
  const roomConversationId =
    roomConversation?.id ?? (openedRoom !== null && openedRoom.groupId === roomGroup?.id ? openedRoom.id : null);

  const openRoom = useCallback(() => {
    if (roomGroup === null) return;
    setRoomError(false);
    openGroupRoom(athlete.user_id, roomGroup.id)?.then(
      (conversation) => setOpenedRoom({ groupId: roomGroup.id, id: conversation.id }),
      () => setRoomError(true),
    );
  }, [athlete.user_id, openGroupRoom, roomGroup]);

  return {
    planThreadId,
    planGroup,
    planError: planThreadId === null ? planError : null,
    retryPlanThread,
    room: roomGroup === null ? null : { group: roomGroup, conversationId: roomConversationId },
    roomKnown,
    openRoom,
    roomOpening: pendingRoomAthleteId === athlete.user_id,
    roomError,
  };
}
