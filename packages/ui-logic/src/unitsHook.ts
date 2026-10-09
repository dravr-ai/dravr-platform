// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hook for the athlete's units, bound to each client's athlete API and device locale, plus the context surfaces read
// ABOUTME: Settings writes the choice; every distance, elevation and pace on Home and the activity view reads the resolved system

import { createContext, useContext, useEffect, useMemo, useRef } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { AthleteApi } from '@pierre/api-client';
import type { DistanceUnit } from '@pierre/chat-utils';
import type { UnitPreference, UnitPreferences } from '@pierre/shared-types';

/** The units change only when the athlete or their provider changes them; the cache stays warm. */
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

/** The Settings control's three choices, in order, each with its catalogue label. */
export const UNIT_PREFERENCE_OPTIONS: readonly { value: UnitPreference; labelKey: string }[] = [
  { value: 'automatic', labelKey: 'settings.unitsAutomatic' },
  { value: 'metric', labelKey: 'settings.unitsMetric' },
  { value: 'imperial', labelKey: 'settings.unitsImperial' },
];

/** The sentence under the control that says what Automatic resolved to and why. */
export interface UnitsAutomaticHint {
  key: 'settings.unitsFromProvider' | 'settings.unitsFromLocale';
  /** The catalogue key of the system's name (`metric`, `imperial`). */
  systemKey: 'settings.unitsSystemMetric' | 'settings.unitsSystemImperial';
  /** The provider whose setting decided it, by name (`Strava`); absent on the locale rung. */
  provider?: string;
}

/**
 * What Automatic resolved to, for the sentence under the Settings control —
 * null while the athlete's own choice decides, since then nothing automatic
 * applies.
 */
export function unitsAutomaticHint(preferences: UnitPreferences): UnitsAutomaticHint | null {
  if (preferences.preference !== 'automatic') return null;
  const systemKey = preferences.units === 'imperial' ? 'settings.unitsSystemImperial' : 'settings.unitsSystemMetric';
  if (preferences.source === 'provider' && preferences.provider !== null) {
    const name = preferences.provider.charAt(0).toUpperCase() + preferences.provider.slice(1);
    return { key: 'settings.unitsFromProvider', systemKey, provider: name };
  }
  return { key: 'settings.unitsFromLocale', systemKey };
}

/** What `useUnitPreferences` hands a surface. */
export interface UseUnitPreferencesResult {
  /** The server's answer, or null until it has answered. */
  preferences: UnitPreferences | null;
  /** The resolved system, or null until the server has answered. */
  units: DistanceUnit | null;
  isLoading: boolean;
  isError: boolean;
  /** Store a Settings choice; the control shows it at once and settles on the server's answer. */
  update: (preference: UnitPreference) => void;
  isUpdating: boolean;
  /** The last choice could not be stored; the control shows what the server still holds. */
  saveFailed: boolean;
}

/**
 * Build `useUnitPreferences` over one client's athlete API and the locale
 * its device reports (`en-US`).
 *
 * The read sends the device locale, so the server resolves the right units
 * on the first answer. When the locale the server has stored differs, the
 * hook stores this device's once, alone — never the choice, which another
 * device may have changed since — so the agent, which has no device to ask,
 * writes in the same units the page prints. That report runs in the
 * background: it neither locks the control nor reads as a failed save.
 */
export function createUnitPreferencesHook(
  athleteApi: Pick<AthleteApi, 'getUnitPreferences' | 'updateUnitPreferences'>,
  deviceLocale: () => string,
) {
  return function useUnitPreferences(): UseUnitPreferencesResult {
    const queryClient = useQueryClient();
    const key = QUERY_KEYS.user.units();
    const locale = deviceLocale();
    const query = useQuery({
      queryKey: key,
      queryFn: () => athleteApi.getUnitPreferences(locale),
      staleTime: UNITS_STALE_MS,
    });

    const mutation = useMutation({
      mutationFn: (preference: UnitPreference) =>
        athleteApi.updateUnitPreferences(locale === '' ? { preference } : { preference, device_locale: locale }),
      onMutate: async (preference: UnitPreference) => {
        await queryClient.cancelQueries({ queryKey: key });
        const previous = queryClient.getQueryData<UnitPreferences>(key);
        if (previous !== undefined) {
          // An explicit system shows at once; automatic waits for the server,
          // which alone knows what the provider and the locale say.
          const units = preference === 'automatic' ? previous.units : preference;
          queryClient.setQueryData<UnitPreferences>(key, { ...previous, preference, units });
        }
        return { previous };
      },
      onError: (_error, _preference, context) => {
        if (context?.previous !== undefined) queryClient.setQueryData(key, context.previous);
        void queryClient.invalidateQueries({ queryKey: key });
      },
      onSuccess: (stored) => {
        queryClient.setQueryData(key, stored);
      },
    });

    const localeReport = useMutation({
      mutationFn: (reported: string) => athleteApi.updateUnitPreferences({ device_locale: reported }),
      onSuccess: (stored) => {
        queryClient.setQueryData(key, stored);
      },
    });

    const { mutate, isPending, isError: saveFailed } = mutation;
    const reportLocale = localeReport.mutate;
    const data = query.data;
    const syncedLocale = useRef<string | null>(null);
    useEffect(() => {
      if (data === undefined || locale === '' || data.device_locale === locale || syncedLocale.current === locale) {
        return;
      }
      syncedLocale.current = locale;
      reportLocale(locale);
    }, [data, locale, reportLocale]);

    return useMemo(
      () => ({
        preferences: data ?? null,
        units: data?.units ?? null,
        isLoading: query.isLoading,
        isError: query.isError,
        update: (preference: UnitPreference) => mutate(preference),
        isUpdating: isPending,
        saveFailed,
      }),
      [data, query.isLoading, query.isError, mutate, isPending, saveFailed],
    );
  };
}
