// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The human coach of a group the athlete is in, for the screens that end on « ton coach » (carnet#1649)
// ABOUTME: Read from the athlete's own group conversations and those groups' detail; never for an account that coaches

import { useMemo } from 'react';
import { useQueries } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { groupsApi } from '../services/api';
import { useAuth } from './useAuth';
import { useConversationsQuery } from './useConversationList';

/**
 * The most groups read to find the athlete's coach. An athlete sits in one or
 * two groups; the cap keeps a long conversation history from fanning out into
 * a read per room.
 */
const MAX_GROUPS_READ = 5;

/** How long a group's detail stays fresh here, as the web's other group reads keep it. */
const GROUP_DETAIL_STALE_MS = 30_000;

/**
 * The human coach attached to one of the athlete's groups, or null: no group,
 * no group with a coach, or an account that coaches itself (a coach reads
 * their athletes, never « ton coach »). The athlete's groups are the ones
 * their conversations sit in, most recent first, and the first of them with a
 * coach other than the athlete is theirs.
 */
export function useMyCoach(): { name: string } | null {
  const { user } = useAuth();
  const isAthlete = user?.manages_roster !== true;
  const conversations = useConversationsQuery(isAthlete);
  const groupIds = useMemo(() => {
    const rows = conversations.data?.pages.flatMap((page) => page.conversations) ?? [];
    const ids = rows
      .map((row) => row.group_id)
      .filter((id): id is string => typeof id === 'string' && id !== '');
    return [...new Set(ids)].slice(0, MAX_GROUPS_READ);
  }, [conversations.data]);
  const groups = useQueries({
    queries: groupIds.map((groupId) => ({
      queryKey: QUERY_KEYS.groups.detail(groupId),
      queryFn: () => groupsApi.getGroup(groupId),
      enabled: isAthlete,
      staleTime: GROUP_DETAIL_STALE_MS,
    })),
  });
  if (!isAthlete) return null;
  for (const { data: group } of groups) {
    if (!group || group.coach_user_id === null || group.coach_user_id === user?.id) continue;
    if (group.coach_display_name !== null) return { name: group.coach_display_name };
  }
  return null;
}
