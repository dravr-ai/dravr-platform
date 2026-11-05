// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hook for the calendar Dravr's plan and workout pushes go to, bound to each client's athlete API
// ABOUTME: Connections settings on web and phone read and write it; the agent's set_preferred_calendar writes the same row (carnet#1393)

import { useEffect, useMemo, useRef } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { AthleteApi } from '@pierre/api-client';
import type { PreferredCalendar } from '@pierre/shared-types';

/**
 * Whether the athlete has a calendar choice to make or to see: two or more
 * connected calendars, or one recorded — even one no longer connected, so they
 * learn their plan goes elsewhere now. One calendar and no choice: nothing to
 * show, the plan goes on it.
 */
export function hasCalendarChoice(calendar: PreferredCalendar | null): boolean {
  if (calendar === null) return false;
  return calendar.chosen !== null || calendar.calendars.length > 1;
}

/** What `usePreferredCalendar` hands a surface. */
export interface UsePreferredCalendarResult {
  /** The choice and the calendars to choose from, or null until the server has answered. */
  calendar: PreferredCalendar | null;
  /** Whether the setting is worth showing ({@link hasCalendarChoice}). */
  visible: boolean;
  /**
   * The provider the control shows as chosen: the recorded calendar while it
   * is connected, else null ("none chosen").
   */
  selected: string | null;
  isLoading: boolean;
  isError: boolean;
  /** Record one of `calendar.calendars`, or forget the choice with null; settles on the server's answer. */
  choose: (provider: string | null) => void;
  isUpdating: boolean;
  /** The last choice could not be stored; the control shows what the server still holds. */
  saveFailed: boolean;
}

/**
 * Build `usePreferredCalendar` over one client's athlete API. Read on every
 * mount, and again whenever `connected` changes: a connection made or removed
 * on the same screen changes the calendars to choose from. `connected` is the
 * screen's connected providers in any stable form.
 */
export function createPreferredCalendarHook(
  athleteApi: Pick<AthleteApi, 'getPreferredCalendar' | 'setPreferredCalendar' | 'clearPreferredCalendar'>,
) {
  return function usePreferredCalendar(connected: string): UsePreferredCalendarResult {
    const queryClient = useQueryClient();
    const key = QUERY_KEYS.user.preferredCalendar();
    const query = useQuery({
      queryKey: key,
      queryFn: () => athleteApi.getPreferredCalendar(),
      refetchOnMount: 'always',
    });
    const seen = useRef(connected);

    useEffect(() => {
      if (seen.current === connected) return;
      seen.current = connected;
      void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.user.preferredCalendar() });
    }, [connected, queryClient]);

    const mutation = useMutation({
      mutationFn: (provider: string | null) =>
        provider === null ? athleteApi.clearPreferredCalendar() : athleteApi.setPreferredCalendar(provider),
      onMutate: async (provider: string | null) => {
        await queryClient.cancelQueries({ queryKey: key });
        const previous = queryClient.getQueryData<PreferredCalendar>(key);
        if (previous !== undefined) {
          const option = previous.calendars.find((calendar) => calendar.provider === provider) ?? null;
          queryClient.setQueryData<PreferredCalendar>(key, {
            ...previous,
            chosen: option,
            chosen_connected: option !== null,
          });
        }
        return { previous };
      },
      onError: (_error, _provider, context) => {
        if (context?.previous !== undefined) queryClient.setQueryData(key, context.previous);
        void queryClient.invalidateQueries({ queryKey: key });
      },
      onSuccess: (stored) => {
        queryClient.setQueryData(key, stored);
      },
    });

    const { mutate, isPending, isError: saveFailed } = mutation;
    const calendar = query.data ?? null;
    const selected = calendar?.chosen_connected ? (calendar.chosen?.provider ?? null) : null;

    return useMemo(
      () => ({
        calendar,
        visible: hasCalendarChoice(calendar),
        selected,
        isLoading: query.isLoading,
        isError: query.isError,
        choose: (provider: string | null) => mutate(provider),
        isUpdating: isPending,
        saveFailed,
      }),
      [calendar, selected, query.isLoading, query.isError, mutate, isPending, saveFailed],
    );
  };
}
