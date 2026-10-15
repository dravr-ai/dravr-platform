// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hook for the athlete's units, bound to each client's athlete API, plus the context surfaces read
// ABOUTME: Metric unless Settings chose imperial; every distance, elevation and pace on Home and the activity view reads it

import { createContext, useContext, useMemo } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { AthleteApi } from '@pierre/api-client';
import type { DistanceUnit } from '@pierre/chat-utils';
import type { UnitPreferences, UnitSystem } from '@pierre/shared-types';

/** The units change only when the athlete changes them in Settings; the cache stays warm. */
const UNITS_STALE_MS = 5 * 60_000;

/**
 * The unit system every distance a surface prints is written in. Provided
 * once at the signed-in app's root ({@link useUnitPreferences}'s `units`); a
 * surface outside that root — and a test that mounts one alone — reads metric.
 */
export const UnitsContext = createContext<DistanceUnit>('metric');

/** The unit system to print distances, elevation and pace in. */
export function useDistanceUnit(): DistanceUnit {
  return useContext(UnitsContext);
}

/** The Settings control's two choices, in order, each with its catalogue label. */
export const UNIT_SYSTEM_OPTIONS: readonly { value: UnitSystem; labelKey: string }[] = [
  { value: 'metric', labelKey: 'settings.unitsMetric' },
  { value: 'imperial', labelKey: 'settings.unitsImperial' },
];

/** What `useUnitPreferences` hands a surface. */
export interface UseUnitPreferencesResult {
  /** The athlete's system, or null until the server has answered. */
  units: DistanceUnit | null;
  isLoading: boolean;
  isError: boolean;
  /** Store a Settings choice; the control shows it at once and settles on the server's answer. */
  update: (units: UnitSystem) => void;
  isUpdating: boolean;
  /** The last choice could not be stored; the control shows what the server still holds. */
  saveFailed: boolean;
}

/**
 * Build `useUnitPreferences` over one client's athlete API. The server
 * answers metric unless the athlete chose imperial; nothing the device
 * reports changes it (carnet#1044).
 */
export function createUnitPreferencesHook(
  athleteApi: Pick<AthleteApi, 'getUnitPreferences' | 'updateUnitPreferences'>,
) {
  return function useUnitPreferences(): UseUnitPreferencesResult {
    const queryClient = useQueryClient();
    const key = QUERY_KEYS.user.units();
    const query = useQuery({
      queryKey: key,
      queryFn: () => athleteApi.getUnitPreferences(),
      staleTime: UNITS_STALE_MS,
    });

    const mutation = useMutation({
      mutationFn: (units: UnitSystem) => athleteApi.updateUnitPreferences(units),
      onMutate: async (units: UnitSystem) => {
        await queryClient.cancelQueries({ queryKey: key });
        const previous = queryClient.getQueryData<UnitPreferences>(key);
        queryClient.setQueryData<UnitPreferences>(key, { units });
        return { previous };
      },
      onError: (_error, _units, context) => {
        if (context?.previous !== undefined) queryClient.setQueryData(key, context.previous);
        void queryClient.invalidateQueries({ queryKey: key });
      },
      onSuccess: (stored) => {
        queryClient.setQueryData(key, stored);
      },
    });

    const { mutate, isPending, isError: saveFailed } = mutation;
    const units = query.data?.units ?? null;

    return useMemo(
      () => ({
        units,
        isLoading: query.isLoading,
        isError: query.isError,
        update: (next: UnitSystem) => mutate(next),
        isUpdating: isPending,
        saveFailed,
      }),
      [units, query.isLoading, query.isError, mutate, isPending, saveFailed],
    );
  };
}
