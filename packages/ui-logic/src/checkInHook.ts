// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hook for how often the coach checks in, bound to each client's athlete API (carnet#831)
// ABOUTME: The onboarding step and Settings write the one server-side choice; off is an answer like the others

import { useMemo } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { AthleteApi } from '@pierre/api-client';
import type { CheckInCadence, CheckInPreference } from '@pierre/shared-types';

/** The cadence changes only when the athlete changes it; the cache stays warm. */
const CHECK_IN_STALE_MS = 5 * 60_000;

/** The four answers, in the order they are offered, each with its catalogue label. */
export const CHECK_IN_OPTIONS: readonly { value: CheckInCadence; labelKey: string }[] = [
  { value: 'daily', labelKey: 'settings.checkInDaily' },
  { value: 'three_per_week', labelKey: 'settings.checkInThreePerWeek' },
  { value: 'weekly', labelKey: 'settings.checkInWeekly' },
  { value: 'off', labelKey: 'settings.checkInOff' },
];

/** What `useCheckIn` hands a surface. */
export interface UseCheckInResult {
  /**
   * The server's answer, or null until it has answered. Its `cadence` is null
   * for an athlete who never chose, who is checked in on exactly as on `off`.
   */
  preference: CheckInPreference | null;
  isLoading: boolean;
  isError: boolean;
  /** Settings: store a choice; the control shows it at once and settles on the server's answer. */
  update: (cadence: CheckInCadence) => void;
  /** Onboarding: store a choice and resolve once it is stored, rejecting when it is not. */
  choose: (cadence: CheckInCadence) => Promise<CheckInPreference>;
  isUpdating: boolean;
  /** The last choice could not be stored; the control shows what the server still holds. */
  saveFailed: boolean;
}

/**
 * Build `useCheckIn` over one client's athlete API.
 *
 * A choice is written into the cache before the request lands; a refused
 * write rolls it back to what the server holds.
 */
export function createCheckInHook(athleteApi: Pick<AthleteApi, 'getCheckIn' | 'updateCheckIn'>) {
  return function useCheckIn(): UseCheckInResult {
    const queryClient = useQueryClient();
    const key = QUERY_KEYS.user.checkIn();
    const query = useQuery({
      queryKey: key,
      queryFn: () => athleteApi.getCheckIn(),
      staleTime: CHECK_IN_STALE_MS,
    });

    const mutation = useMutation({
      mutationFn: (cadence: CheckInCadence) => athleteApi.updateCheckIn(cadence),
      onMutate: async (cadence: CheckInCadence) => {
        await queryClient.cancelQueries({ queryKey: key });
        const previous = queryClient.getQueryData<CheckInPreference>(key);
        queryClient.setQueryData<CheckInPreference>(key, { cadence });
        return { previous };
      },
      onError: (_error, _cadence, context) => {
        if (context?.previous !== undefined) queryClient.setQueryData(key, context.previous);
        void queryClient.invalidateQueries({ queryKey: key });
      },
      onSuccess: (stored) => {
        queryClient.setQueryData(key, stored);
      },
    });

    const { mutate, mutateAsync, isPending, isError: saveFailed } = mutation;
    return useMemo(
      () => ({
        preference: query.data ?? null,
        isLoading: query.isLoading,
        isError: query.isError,
        update: (cadence: CheckInCadence) => mutate(cadence),
        choose: (cadence: CheckInCadence) => mutateAsync(cadence),
        isUpdating: isPending,
        saveFailed,
      }),
      [query.data, query.isLoading, query.isError, mutate, mutateAsync, isPending, saveFailed],
    );
  };
}
