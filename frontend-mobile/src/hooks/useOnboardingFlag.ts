// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One AsyncStorage-backed per-step onboarding flag, cached in React Query (mobile)
// ABOUTME: The single mechanism behind every "has this step been done" flag; keys mirror the web ones

import { useCallback } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import AsyncStorage from '@react-native-async-storage/async-storage';

/**
 * Resolves whether a per-user onboarding step is already complete, backed by
 * AsyncStorage and cached in React Query so the routing gate (RootLayoutNav) and
 * the screen read the *same* value. When a screen calls `mark`, the shared cache
 * flips and the gate routes the user on.
 *
 * `storagePrefix` must match the web key exactly (e.g. `dravr.profile_type_chosen.`)
 * — the two clients write the same per-user localStorage/AsyncStorage names, and
 * the server step record is what actually carries state across devices.
 *
 * `done` is `undefined` while the read is in flight; callers hold routing until it
 * resolves, avoiding a flash of the wrong screen. Fails **open** (no user, or a
 * storage error ⇒ `true`) on purpose: a storage hiccup must never trap someone on
 * an onboarding step they've already finished.
 */
export function useOnboardingFlag(storagePrefix: string, userId: string | undefined) {
  const queryClient = useQueryClient();
  const queryKey = ['onboarding-flag', storagePrefix, userId] as const;

  const { data: done } = useQuery({
    queryKey,
    queryFn: async () => {
      if (!userId) return true;
      try {
        return (await AsyncStorage.getItem(`${storagePrefix}${userId}`)) === '1';
      } catch {
        return true;
      }
    },
    staleTime: Infinity,
  });

  const mark = useCallback(async () => {
    if (userId) {
      try {
        await AsyncStorage.setItem(`${storagePrefix}${userId}`, '1');
      } catch {
        // Non-fatal: the cache update below still advances the user, and the
        // server step record is the durable copy.
      }
    }
    queryClient.setQueryData(queryKey, true);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- queryKey is derived from the two deps below
  }, [storagePrefix, userId, queryClient]);

  return { done, mark };
}
