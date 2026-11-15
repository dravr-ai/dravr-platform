// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Session-only "the coach's group was just made" (mobile), matching web: the invite is the next segment
// ABOUTME: Kept in the shared query cache so the routing gate, the progress bar and the group screen read one value

import { useCallback } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { SESSION_ONLY_QUERY_META } from '../providers/sessionOnlyQuery';

/**
 * Whether the coach's group was made this session, so the invite — the
 * screen that holds the link the creation just issued — is the current step.
 *
 * Session-only like `useConnectProviderDone`: after a relaunch the link is
 * gone, and the group step is offered again until it is recorded.
 */
export function useCoachGroupCreated(userId: string | undefined) {
  const queryClient = useQueryClient();
  const queryKey = ['coach-group-created', userId] as const;

  const { data: created } = useQuery({
    queryKey,
    queryFn: () => false,
    staleTime: Infinity,
    // Kept off disk: the persister would otherwise restore it on the next launch.
    meta: SESSION_ONLY_QUERY_META,
  });

  const mark = useCallback(() => {
    queryClient.setQueryData(queryKey, true);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- queryKey derives from userId
  }, [userId, queryClient]);

  return { created: created ?? false, mark };
}
