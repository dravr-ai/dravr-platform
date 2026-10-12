// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Session-only "Continue" past the connect step after a first provider connects (mobile), matching web
// ABOUTME: Until it is set the connect screen stays up, so a second service is one tap away rather than in Settings

import { useCallback } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';

/**
 * Whether the user pressed Continue on the connect screen after connecting a
 * provider this session.
 *
 * Connecting a first provider does not leave the screen: the list stays up
 * with the new row marked connected, and Continue moves on. Session-only like
 * `useProviderSkipped`, and kept in the shared query cache for the same reason
 * — the routing gate and the connect screen read one value.
 */
export function useConnectProviderDone(userId: string | undefined) {
  const queryClient = useQueryClient();
  const queryKey = ['connect-provider-done', userId] as const;

  const { data: done } = useQuery({
    queryKey,
    queryFn: () => false,
    staleTime: Infinity,
  });

  const complete = useCallback(() => {
    queryClient.setQueryData(queryKey, true);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- queryKey derives from userId
  }, [userId, queryClient]);

  return { done: done ?? false, complete };
}
