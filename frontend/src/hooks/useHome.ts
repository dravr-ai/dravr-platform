// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query reads behind the athlete Home page — recent activities, one activity's route, the plan for today
// ABOUTME: A stale activity answer is followed up on a schedule that ends, and only while somebody is using the tab; never polled

import { useEffect, useState } from 'react';
import { focusManager, useQuery } from '@tanstack/react-query';
import { HOME_STALE_REFETCH_DELAYS_MS, QUERY_KEYS } from '@pierre/shared-constants';
import type {
  ActivityRouteResponse,
  RecentActivitiesResponse,
  TrainingPlanResponse,
} from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { athleteApi, providersApi } from '../services/api';

/** What the Home page reads from the recent-activities query. */
export interface RecentActivitiesState {
  data: RecentActivitiesResponse | undefined;
  isPending: boolean;
  isError: boolean;
  refetch: () => void;
  /**
   * True while the server has said its cache is stale and a follow-up read is
   * still owed. False once an answer is not stale, and once the schedule has
   * ended on an answer that still is: the page then shows when it last synced
   * instead of asking again.
   */
  refreshing: boolean;
}

/**
 * The athlete's newest activities, newest first, straight from the server's
 * durable cache — the read never reaches a provider.
 *
 * A `stale: true` answer means the server started a background refresh, which
 * a scraped provider can take minutes to finish. The page follows
 * `HOME_STALE_REFETCH_DELAYS_MS`: it waits the first delay and asks, and while
 * the answer is still stale it waits the next delay and asks again. It stops
 * at the first answer that is not stale, and after the last delay whatever
 * that answer says. A read that fails is an answer too, so the schedule moves
 * on to its next delay instead of asking in a loop.
 *
 * Every ask goes out only while the tab is focused in React Query's sense —
 * the idle watch marks an untouched or hidden tab unfocused, so an ask that
 * comes due then waits for the athlete to come back rather than waking an
 * instance for nobody. It is a schedule with an end, never an interval.
 *
 * A schedule that ended is ended for the answer it ended on. A stale answer
 * that arrives after it — a focus refetch, a retry — starts a schedule of its
 * own, while one that arrives during a schedule changes nothing: the running
 * schedule keeps its one timer. A stale answer the cache already holds when
 * the page mounts, left by an earlier visit, is owed a schedule like any
 * other: what ended is forgotten with the page that ended it.
 */
export function useRecentActivities(): RecentActivitiesState {
  const query = useQuery({
    queryKey: QUERY_KEYS.home.recentActivities(),
    queryFn: () => athleteApi.getRecentActivities(),
  });
  const { refetch } = query;
  // The answer the last schedule ran out on, named by when it arrived. Only
  // that answer is owed nothing more; any stale answer after it is owed a
  // schedule, even one whose body is the same.
  const [endedOn, setEndedOn] = useState<number | null>(null);
  const waiting = query.data?.stale === true && query.dataUpdatedAt !== endedOn;

  useEffect(() => {
    if (!waiting) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let unsubscribe: (() => void) | null = null;
    const ask = (step: number) => {
      unsubscribe?.();
      unsubscribe = null;
      // Coming back to the tab also fires React Query's own focus refetch;
      // joining a read already in flight keeps the return one request.
      void refetch({ cancelRefetch: false }).then((answer) => {
        // A fresh answer ends the schedule through `waiting`. A failed read
        // keeps the stale answer it followed, so it advances like one.
        if (cancelled || answer.data?.stale !== true) return;
        if (step + 1 < HOME_STALE_REFETCH_DELAYS_MS.length) {
          wait(step + 1);
        } else {
          setEndedOn(answer.dataUpdatedAt);
        }
      });
    };
    const wait = (step: number) => {
      timer = setTimeout(() => {
        if (focusManager.isFocused()) {
          ask(step);
          return;
        }
        unsubscribe = focusManager.subscribe((focused) => {
          if (focused) ask(step);
        });
      }, HOME_STALE_REFETCH_DELAYS_MS[step]);
    };
    wait(0);
    return () => {
      cancelled = true;
      clearTimeout(timer);
      unsubscribe?.();
    };
  }, [waiting, refetch]);

  return {
    data: query.data,
    isPending: query.isPending,
    isError: query.isError,
    refetch: () => void refetch(),
    refreshing: waiting,
  };
}

/**
 * One activity's route, or the reason there is none.
 *
 * `enabled` is how a caller stays off the wire: one that already has the
 * geometry — a summary polyline — or whose activity says `has_gps: false`,
 * which means the route was read once and the recording held no GPS. An
 * activity with `has_gps: true` may never have had its route read, so this
 * answer, not the flag, is what says whether there is a track to draw.
 * A finished activity's route never changes and the server stores it once,
 * so the answer is kept for the whole session.
 */
export function useActivityRoute(provider: string, activityId: string, enabled: boolean) {
  return useQuery<ActivityRouteResponse>({
    queryKey: QUERY_KEYS.home.activityRoute(provider, activityId),
    queryFn: () => athleteApi.getActivityRoute(provider, activityId),
    enabled,
    staleTime: Number.POSITIVE_INFINITY,
  });
}

/**
 * The active plan as `/plan` shows it, in the language the app is rendering,
 * plus the athlete-local today it was projected for.
 */
export function useTrainingPlan() {
  const { language } = useTranslation();
  return useQuery<TrainingPlanResponse>({
    queryKey: QUERY_KEYS.home.trainingPlan(language),
    queryFn: () => athleteApi.getTrainingPlan(language),
  });
}

/** What the Home page reads from the provider-status query. */
export interface ProviderConnectionState {
  loaded: boolean;
  connected: boolean;
  /**
   * The display name of every connected provider whose session is not usable
   * until the athlete reconnects it, in the order the server lists them and
   * each name once: a provider served by a mirror backend is listed as two
   * cards that share one name.
   */
  needsReconnect: string[];
}

/**
 * Whether the athlete has any provider connected, and which of the connected
 * ones have to be reconnected, read from the same provider-status query the
 * chat pane and the connect banner share.
 *
 * `loaded` is false until that query answers, so a connected athlete is never
 * shown a connect prompt while the answer is still in flight.
 *
 * The server refreshes nothing for a connection flagged `needs_reauth`, so
 * its activities stay as old as its last sync. The flag only means something
 * on a connected provider, which is why a disconnected one is never named.
 */
export function useProviderConnection(): ProviderConnectionState {
  const { data, isSuccess } = useQuery({
    queryKey: QUERY_KEYS.providers.status(),
    queryFn: () => providersApi.getProvidersStatus(),
  });
  const providers = data?.providers ?? [];
  return {
    loaded: isSuccess,
    connected: providers.some((provider) => provider.connected),
    needsReconnect: [
      ...new Set(
        providers.filter((provider) => provider.connected && provider.needs_reauth).map((provider) => provider.display_name),
      ),
    ],
  };
}
