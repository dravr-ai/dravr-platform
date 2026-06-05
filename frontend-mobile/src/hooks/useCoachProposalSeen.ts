// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tracks whether the user has seen the one-time onboarding coach proposal (mobile)
// ABOUTME: AsyncStorage-backed via React Query so RootLayoutNav and the screen share one source of truth

import { useCallback } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import AsyncStorage from '@react-native-async-storage/async-storage';

const storageKey = (userId: string) => `dravr.coach_proposal_done.${userId}`;
const queryKey = (userId: string | undefined) => ['coach-proposal-seen', userId] as const;

/**
 * Resolves whether the coach-proposal onboarding step is already complete for
 * `userId`, backed by AsyncStorage but cached in React Query so the routing
 * gate (RootLayoutNav) and the proposal screen read the *same* value. When the
 * screen calls `markSeen`, the shared cache flips and RootLayoutNav routes the
 * user on to chat — no manual navigation, no redirect loop.
 *
 * `seen` is `undefined` while the read is in flight; callers hold routing until
 * it resolves. Fails open (no user / storage error ⇒ `true`) so a hiccup never
 * traps the user on the proposal step.
 */
export function useCoachProposalSeen(userId: string | undefined) {
  const queryClient = useQueryClient();

  const { data: seen } = useQuery({
    queryKey: queryKey(userId),
    queryFn: async () => {
      if (!userId) return true;
      try {
        return (await AsyncStorage.getItem(storageKey(userId))) === '1';
      } catch {
        return true;
      }
    },
    staleTime: Infinity,
  });

  const markSeen = useCallback(async () => {
    if (userId) {
      try {
        await AsyncStorage.setItem(storageKey(userId), '1');
      } catch {
        // Non-fatal: the cache update below still advances the user.
      }
    }
    queryClient.setQueryData(queryKey(userId), true);
  }, [userId, queryClient]);

  return { seen, markSeen };
}
