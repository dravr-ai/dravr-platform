// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tracks whether the user has completed the athlete-vs-coach profile-type onboarding step (mobile)
// ABOUTME: AsyncStorage-backed via React Query so RootLayoutNav and the screen share one source of truth

import { useCallback } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import AsyncStorage from '@react-native-async-storage/async-storage';

const storageKey = (userId: string) => `dravr.profile_type_chosen.${userId}`;
const queryKey = (userId: string | undefined) => ['profile-type-chosen', userId] as const;

/**
 * Resolves whether the profile-type onboarding step is already complete for
 * `userId`, backed by AsyncStorage but cached in React Query so the routing gate
 * (RootLayoutNav) and the screen read the *same* value — matching the web key
 * `dravr.profile_type_chosen.{id}`. When the screen calls `markChosen`, the
 * shared cache flips and the gate routes the user on.
 *
 * `chosen` is `undefined` while the read is in flight; callers hold routing until
 * it resolves. Fails open (no user / storage error ⇒ `true`) so a hiccup never
 * traps the user on the profile-type step.
 */
export function useProfileTypeChosen(userId: string | undefined) {
  const queryClient = useQueryClient();

  const { data: chosen } = useQuery({
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

  const markChosen = useCallback(async () => {
    if (userId) {
      try {
        await AsyncStorage.setItem(storageKey(userId), '1');
      } catch {
        // Non-fatal: the cache update below still advances the user.
      }
    }
    queryClient.setQueryData(queryKey(userId), true);
  }, [userId, queryClient]);

  return { chosen, markChosen };
}
