// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Session-only "continue without connecting" flag (mobile), matching the web escape hatch
// ABOUTME: Deliberately NOT persisted — the nudge should return next launch, unlike a completed step

import { useCallback } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';

/**
 * Whether the user chose to continue past the provider gate without connecting.
 *
 * Mirrors the web `skipProvider`, including the part that matters most: it is
 * **session-only**. Every other onboarding flag is durable because those steps
 * are genuinely finished; this one is a deferral, so it lives in the React Query
 * cache and nowhere else. Next launch the connect screen returns, which is the
 * intended nudge — the connect prompts in chat and on the coach screens carry it
 * in between.
 *
 * Kept in the shared query cache rather than component state so the routing gate
 * and the connect screen read one value, the same way the AsyncStorage-backed
 * flags do.
 */
export function useProviderSkipped(userId: string | undefined) {
  const queryClient = useQueryClient();
  const queryKey = ['provider-skipped', userId] as const;

  const { data: skipped } = useQuery({
    queryKey,
    // No AsyncStorage read: absence IS the answer at the start of a session.
    queryFn: () => false,
    staleTime: Infinity,
  });

  const skip = useCallback(() => {
    queryClient.setQueryData(queryKey, true);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- queryKey derives from userId
  }, [userId, queryClient]);

  return { skipped: skipped ?? false, skip };
}
