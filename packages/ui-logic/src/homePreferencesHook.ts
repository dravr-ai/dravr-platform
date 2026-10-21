// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hook for the athlete's Home preferences, bound to each client's athlete API
// ABOUTME: Home reads it to decide what to offer; a change names only the choices it touches, and Settings brings them back

import { useMemo } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { AthleteApi } from '@pierre/api-client';
import { DEFAULT_HOME_PREFERENCES } from '@pierre/shared-types';
import type { HomePreferences, HomePreferencesUpdate } from '@pierre/shared-types';

/** The athlete's own choices change only when they change them; the cache stays warm. */
const PREFERENCES_STALE_MS = 5 * 60_000;

/** What `useHomePreferences` hands a surface. */
export interface UseHomePreferencesResult {
  /** The stored choices, or null until the server has answered. */
  preferences: HomePreferences | null;
  isLoading: boolean;
  isError: boolean;
  /**
   * Store a change to the choices it names; the others keep what is stored.
   * The page shows it at once and settles on the server's answer.
   */
  update: (change: HomePreferencesUpdate) => void;
  isUpdating: boolean;
}

/**
 * Build `useHomePreferences` over one client's athlete API.
 *
 * A change is merged into the cache before the request lands, so a dismissed
 * suggestion leaves the page on the tap rather than a round trip later; a
 * refused write rolls it back to what the server holds. Only the changed
 * choices travel: the server merges them, so a write made before the first
 * read cannot reset a choice this client never saw.
 */
export function createHomePreferencesHook(
  athleteApi: Pick<AthleteApi, 'getHomePreferences' | 'updateHomePreferences'>,
) {
  return function useHomePreferences(): UseHomePreferencesResult {
    const queryClient = useQueryClient();
    const key = QUERY_KEYS.home.preferences();
    const query = useQuery({
      queryKey: key,
      queryFn: () => athleteApi.getHomePreferences(),
      staleTime: PREFERENCES_STALE_MS,
    });

    const mutation = useMutation({
      mutationFn: (change: HomePreferencesUpdate) => athleteApi.updateHomePreferences(change),
      onMutate: async (change: HomePreferencesUpdate) => {
        await queryClient.cancelQueries({ queryKey: key });
        const previous = queryClient.getQueryData<HomePreferences>(key);
        // Before the first answer there is nothing to merge into: the page
        // waits for the server's, rather than showing defaults as stored.
        if (previous !== undefined) {
          queryClient.setQueryData<HomePreferences>(key, { ...DEFAULT_HOME_PREFERENCES, ...previous, ...change });
        }
        return { previous };
      },
      onError: (_error, _prefs, context) => {
        if (context?.previous !== undefined) queryClient.setQueryData(key, context.previous);
        void queryClient.invalidateQueries({ queryKey: key });
      },
      onSuccess: (stored) => {
        queryClient.setQueryData(key, stored);
      },
    });

    const { mutate, isPending } = mutation;
    return useMemo(
      () => ({
        preferences: query.data ?? null,
        isLoading: query.isLoading,
        isError: query.isError,
        update: (change: HomePreferencesUpdate) => mutate(change),
        isUpdating: isPending,
      }),
      [query.data, query.isLoading, query.isError, mutate, isPending],
    );
  };
}
