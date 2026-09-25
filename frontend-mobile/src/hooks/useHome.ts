// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hooks behind the Home tab — recent activities, one activity's route, the plan for today
// ABOUTME: A stale activity answer is asked again once, after HOME_STALE_REFETCH_DELAY_MS and only while the app is in use

import { useEffect, useRef, useState } from 'react';
import { focusManager, useQuery } from '@tanstack/react-query';
import { HOME_STALE_REFETCH_DELAY_MS, QUERY_KEYS } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { athleteApi, oauthApi } from '../services/api';

/**
 * Where the one follow-up read for a stale answer stands.
 *
 * - `none` — no stale answer has arrived, so nothing is scheduled.
 * - `pending` — the server said stale and started its own refresh; the hook
 *   asks again after the delay.
 * - `done` — the follow-up answered. Whatever it said, nothing more is asked:
 *   a second stale answer is shown with its sync time, never polled.
 */
export type StaleRefetchPhase = 'none' | 'pending' | 'done';

/**
 * The newest cached activities, and the one follow-up read a stale answer earns.
 *
 * Reading this never reaches a provider: the server answers from its durable
 * cache and, when the cache is past its freshness window, refreshes it in the
 * background and says `stale`. The hook then asks once more after
 * {@link HOME_STALE_REFETCH_DELAY_MS}. The delay is a timer, cancelled when
 * the screen unmounts; when it expires while the app is backgrounded or idle —
 * the idle watch expresses both as "not focused" — the read waits for the
 * athlete to come back instead of spending a request nobody will see.
 */
export function useRecentActivities() {
  const query = useQuery({
    // Home asks for the server's default of five; the key says so with a null.
    queryKey: QUERY_KEYS.home.recentActivities(),
    queryFn: () => athleteApi.getRecentActivities(),
  });

  const stale = query.data?.stale === true;
  const { refetch } = query;
  const [staleRefetch, setStaleRefetch] = useState<StaleRefetchPhase>('none');
  // The phase is also kept in a ref: the effect below must read the current
  // value, not the one it closed over, to schedule at most one follow-up.
  const phase = useRef<StaleRefetchPhase>('none');
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const unsubscribeFocus = useRef<(() => void) | null>(null);

  // Unmounting cancels whatever is scheduled — the delay and the wait for focus.
  useEffect(
    () => () => {
      if (timer.current !== null) {
        clearTimeout(timer.current);
        timer.current = null;
      }
      unsubscribeFocus.current?.();
      unsubscribeFocus.current = null;
    },
    [],
  );

  useEffect(() => {
    if (!stale || phase.current !== 'none') {
      return;
    }
    phase.current = 'pending';
    setStaleRefetch('pending');

    const ask = () => {
      unsubscribeFocus.current?.();
      unsubscribeFocus.current = null;
      void refetch().finally(() => {
        phase.current = 'done';
        setStaleRefetch('done');
      });
    };

    timer.current = setTimeout(() => {
      timer.current = null;
      if (focusManager.isFocused()) {
        ask();
        return;
      }
      unsubscribeFocus.current = focusManager.subscribe((focused) => {
        if (focused) {
          ask();
        }
      });
    }, HOME_STALE_REFETCH_DELAY_MS);
  }, [stale, refetch]);

  return {
    activities: query.data?.activities ?? [],
    asOf: query.data?.as_of ?? null,
    stale,
    staleRefetch,
    hasData: query.data !== undefined,
    isError: query.isError,
    isRefetching: query.isRefetching,
    refetch,
  };
}

/**
 * One activity's route, or the reason there is none.
 *
 * `enabled` is false for an activity without GPS: an indoor ride has no route
 * to ask for, and asking would spend a provider call on the answer the row
 * already carries. A completed activity's route never changes and the server
 * stores it after the first read, so the answer is never considered stale.
 */
export function useActivityRoute(provider: string, activityId: string, enabled: boolean) {
  const query = useQuery({
    queryKey: QUERY_KEYS.home.activityRoute(provider, activityId),
    queryFn: () => athleteApi.getActivityRoute(provider, activityId),
    enabled,
    staleTime: Infinity,
  });

  return {
    route: query.data?.route ?? null,
    reason: query.data?.reason ?? null,
    isError: query.isError,
    refetch: query.refetch,
  };
}

/**
 * The athlete's active plan as `/plan` shows it, and the athlete-local today
 * the server projected it for.
 *
 * Keyed by the language the app renders in: the server names the plan's
 * flavour in it, so a language switch is a different card. `response` stays
 * `null` until an answer arrives — `response.plan === null` is the server
 * saying there is no plan, which the screen answers with a call to build one,
 * and must never be read off a load that has not finished or has failed.
 */
export function useTrainingPlan() {
  const { language } = useTranslation();
  const query = useQuery({
    queryKey: QUERY_KEYS.home.trainingPlan(language),
    queryFn: () => athleteApi.getTrainingPlan(language),
  });

  return {
    response: query.data ?? null,
    isError: query.isError,
    isRefetching: query.isRefetching,
    refetch: query.refetch,
  };
}

/**
 * Whether any fitness provider is connected, from the provider status the
 * Connections pane reads — the activity list carries no such flag.
 *
 * `enabled` is true only while the list is empty: a list with rows already
 * says a provider is there, so asking would spend a request on a known
 * answer. `null` until the status answers; a status read that fails reads
 * as connected, because the empty sentence ("they show up once your
 * provider syncs") is true either way while telling a connected athlete to
 * connect is not.
 */
export function useProviderConnected(enabled: boolean) {
  const query = useQuery({
    queryKey: QUERY_KEYS.providers.status(),
    queryFn: () => oauthApi.getProvidersStatus(),
    enabled,
  });

  let connected: boolean | null = null;
  if (query.data !== undefined) {
    connected = query.data.providers.some((provider) => provider.connected);
  } else if (query.isError) {
    connected = true;
  }

  return { connected, refetch: query.refetch };
}
