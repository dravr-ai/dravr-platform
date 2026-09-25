// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query reads behind the athlete Home page — recent activities, one activity's route, the plan for today
// ABOUTME: A stale activity answer is asked for once more after a delay, and only while somebody is using the tab; never polled

import { useEffect, useState } from 'react';
import { focusManager, useQuery } from '@tanstack/react-query';
import { HOME_STALE_REFETCH_DELAY_MS, QUERY_KEYS } from '@pierre/shared-constants';
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
   * True while the server has said its cache is stale and the one delayed
   * re-read has not answered yet. Once it answers, this stays false for the
   * life of the page even if the second answer is still stale: the page then
   * shows when it last synced instead of asking again.
   */
  refreshing: boolean;
}

/**
 * The athlete's newest activities, newest first, straight from the server's
 * durable cache — the read never reaches a provider.
 *
 * A `stale: true` answer means the server started a background refresh. The
 * page asks once more after `HOME_STALE_REFETCH_DELAY_MS`, and only while the
 * tab is focused in React Query's sense — the idle watch marks an untouched or
 * hidden tab unfocused, so the second read waits for the athlete to come back
 * rather than waking an instance for nobody. It is one read, never an
 * interval.
 */
export function useRecentActivities(): RecentActivitiesState {
  const query = useQuery({
    queryKey: QUERY_KEYS.home.recentActivities(),
    queryFn: () => athleteApi.getRecentActivities(),
  });
  const { refetch } = query;
  const [settled, setSettled] = useState(false);
  const waiting = query.data?.stale === true && !settled;

  useEffect(() => {
    if (!waiting) return;
    let cancelled = false;
    let unsubscribe: (() => void) | null = null;
    const ask = () => {
      unsubscribe?.();
      unsubscribe = null;
      // Coming back to the tab also fires React Query's own focus refetch;
      // joining a read already in flight keeps the return one request.
      void refetch({ cancelRefetch: false }).finally(() => {
        if (!cancelled) setSettled(true);
      });
    };
    const timer = setTimeout(() => {
      if (focusManager.isFocused()) {
        ask();
        return;
      }
      unsubscribe = focusManager.subscribe((focused) => {
        if (focused) ask();
      });
    }, HOME_STALE_REFETCH_DELAY_MS);
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
 * `enabled` is how a caller that already has the geometry — a summary
 * polyline — or knows there is none (`has_gps: false`) stays off the wire.
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

/**
 * Whether the athlete has any provider connected, read from the same
 * provider-status query the chat pane and the connect banner share.
 *
 * `loaded` is false until that query answers, so a connected athlete is never
 * shown a connect prompt while the answer is still in flight.
 */
export function useProviderConnection(): { loaded: boolean; connected: boolean } {
  const { data, isSuccess } = useQuery({
    queryKey: QUERY_KEYS.providers.status(),
    queryFn: () => providersApi.getProvidersStatus(),
  });
  return {
    loaded: isSuccess,
    connected: data?.providers?.some((provider) => provider.connected) ?? false,
  };
}
