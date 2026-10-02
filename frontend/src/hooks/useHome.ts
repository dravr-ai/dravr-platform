// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query reads behind the athlete Home page — recent activities, one activity's route, the plan for today, the training status
// ABOUTME: A stale activity answer is followed up on a schedule that ends, and only while somebody is using the tab; never polled

import { useCallback, useEffect, useState } from 'react';
import { focusManager, useQuery, useQueryClient, type Query } from '@tanstack/react-query';
import {
  HOME_ROUTE_UNAVAILABLE_RECHECK_DELAYS_MS,
  HOME_STALE_REFETCH_DELAYS_MS,
  QUERY_KEYS,
} from '@pierre/shared-constants';
import type {
  ActivityRouteResponse,
  RecentActivitiesResponse,
  TrainingPlanResponse,
  TrainingStatusResponse,
} from '@pierre/shared-types';
import { planDayOn } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import {
  classifyApiError,
  readActivityRoute,
  recentActivitiesSync,
  useRequestsInFlight,
  type RecentActivitiesSync,
} from '@pierre/ui-logic';
import { athleteApi, providersApi } from '../services/api';
import { planDayRouteDraft } from '../components/home/homeFormat';

/** What the Home page reads from the recent-activities query. */
export interface RecentActivitiesState {
  data: RecentActivitiesResponse | undefined;
  isPending: boolean;
  isError: boolean;
  refetch: () => void;
  /**
   * The athlete's retry after a failed sync: asks the server to refresh the
   * failing provider now, past the pause it otherwise leaves it in, and
   * restarts the follow-up schedule from that answer.
   */
  retry: () => void;
  /**
   * True while the server has said its cache is stale and a follow-up read is
   * still owed. False once an answer is not stale, and once the schedule has
   * ended on an answer that still is: the page then shows when it last synced
   * instead of asking again.
   */
  refreshing: boolean;
  /** True while the athlete's `retry` request is in flight. */
  retrying: boolean;
  /**
   * Whether the list is fetching, failed or settled, from `refreshing`,
   * `retrying` and the answer's `sync_failure` ({@link recentActivitiesSync}).
   */
  sync: RecentActivitiesSync;
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
 * that arrives after it — a focus refetch — starts a schedule of its own,
 * while one that arrives during a schedule changes nothing: the running
 * schedule keeps its one timer. The athlete's `retry` is the exception: the
 * refresh it starts is what the page waits on now, so the schedule starts
 * again from the retry, its first ask one delay after it. A stale answer the
 * cache already holds when the page mounts, left by an earlier visit, is owed
 * a schedule like any other: what ended is forgotten with the page that ended
 * it.
 */
export function useRecentActivities(): RecentActivitiesState {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: QUERY_KEYS.home.recentActivities(),
    queryFn: () => athleteApi.getRecentActivities(),
  });
  const { refetch } = query;
  // The answer the last schedule ran out on, named by when it arrived. Only
  // that answer is owed nothing more; any stale answer after it is owed a
  // schedule, even one whose body is the same.
  const [endedOn, setEndedOn] = useState<number | null>(null);
  // How many retries the athlete has made: each one restarts the schedule.
  const [retries, setRetries] = useState(0);
  const retryRequest = useRequestsInFlight();
  const { track } = retryRequest;
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
  }, [waiting, refetch, retries]);

  const retry = useCallback(() => {
    setRetries((count) => count + 1);
    // Written into the same cache entry the page reads, so the answer is
    // judged like any other; a failed retry leaves the rows it had.
    track(
      queryClient.fetchQuery({
        queryKey: QUERY_KEYS.home.recentActivities(),
        queryFn: () => athleteApi.getRecentActivities(undefined, { retry: true }),
        staleTime: 0,
      }),
    ).catch(() => undefined);
  }, [queryClient, track]);

  return {
    data: query.data,
    isPending: query.isPending,
    isError: query.isError,
    refetch: () => void refetch(),
    retry,
    refreshing: waiting,
    retrying: retryRequest.inFlight,
    sync: recentActivitiesSync({
      refreshing: waiting,
      retrying: retryRequest.inFlight,
      syncFailure: query.data?.sync_failure ?? null,
    }),
  };
}

/**
 * Whether a failed route read is asked once more before the map says so.
 *
 * Once, not the client's default three times: a route read can reach the
 * athlete's provider, the transport waits up to
 * `ACTIVITY_ROUTE_REQUEST_TIMEOUT_MS` (30 s) for each attempt, and every
 * attempt holds the map on its loading line — three retries kept "Loading the
 * map…" up for about two minutes before the athlete was told anything. After
 * one retry the map says it could not be loaded and offers a retry of its own.
 * An authorization refusal, or a 404 for an activity that is not the caller's,
 * is the server's answer and is not asked again.
 */
function retryRouteRead(failureCount: number, error: Error): boolean {
  const { kind } = classifyApiError(error);
  if (kind === 'unauthorized' || kind === 'forbidden' || kind === 'notFound') {
    return false;
  }
  return failureCount < 1;
}

/**
 * How long a route answer stays fresh: a drawn route or a settled
 * `too_short` for the whole session — a finished activity's route never
 * changes and the server stores it once — and every other answer not at all.
 * `unavailable` is a read that did not settle anything yet, and a `no_gps`
 * held while the list still says `has_gps: true` is an answer the server has
 * since taken back (the query is disabled once the list agrees), so both are
 * asked again whenever the page mounts or regains focus.
 */
export function routeAnswerStaleTime(query: Query<ActivityRouteResponse>): number {
  const answer = query.state.data;
  return answer !== undefined && (answer.route !== null || answer.reason === 'too_short')
    ? Number.POSITIVE_INFINITY
    : 0;
}

/**
 * When an `unavailable` route answer is asked again on its own: after each
 * wait of `HOME_ROUTE_UNAVAILABLE_RECHECK_DELAYS_MS`, counted in answers, and
 * then never — a schedule with an end, which picks up a route a read made
 * since has stored over the `unavailable`, and never an interval.
 */
export function routeRecheckInterval(query: Query<ActivityRouteResponse>): number | false {
  if (query.state.data?.reason !== 'unavailable') {
    return false;
  }
  return HOME_ROUTE_UNAVAILABLE_RECHECK_DELAYS_MS[query.state.dataUpdateCount - 1] ?? false;
}

/** What a Home map or sketch reads of one activity's route. */
export interface ActivityRouteState {
  data: ActivityRouteResponse | undefined;
  isError: boolean;
  /** True while a read of the route is in flight, the athlete's retry included. */
  isFetching: boolean;
  /**
   * The athlete's retry after `unavailable` or a failed read: the server
   * reads the provider again at once, past the answer it stored.
   */
  retry: () => void;
}

/** How a caller of {@link useActivityRoute} reads the route. */
export interface ActivityRouteOptions {
  /**
   * The caller is a Home list row, whose page asks for all its routes in one
   * burst: the server lets the burst queue before it reads, so the newest
   * activity is read first. An activity view's own map leaves it unset and is
   * read at once; the athlete's retry never carries it.
   */
  burst?: boolean;
}

/**
 * One activity's route, or the reason there is none.
 *
 * `enabled` is how a caller stays off the wire: one that already has the
 * geometry — a summary polyline — or whose activity says `has_gps: false`,
 * which means the route was read once and the recording held no GPS. An
 * activity with `has_gps: true` may never have had its route read, so this
 * answer, not the flag, is what says whether there is a track to draw.
 * A read the server answers `pending` — queued behind the athlete's other
 * route reads, or still running — is asked again, for as long as the bound
 * each `pending` names, and stays loading ({@link readActivityRoute}); the
 * server hands its turn to the newest activity of a Home burst
 * (`options.burst`) first, so the page's big map is read before its
 * sketches. A
 * drawn route is kept for the whole session; `unavailable`, a read that
 * finished without settling anything, is asked again on its own a bounded
 * number of times ({@link routeRecheckInterval}), on focus, and at the retry.
 */
export function useActivityRoute(
  provider: string,
  activityId: string,
  enabled: boolean,
  options: ActivityRouteOptions = {},
): ActivityRouteState {
  const burst = options.burst === true;
  const queryClient = useQueryClient();
  const query = useQuery<ActivityRouteResponse>({
    queryKey: QUERY_KEYS.home.activityRoute(provider, activityId),
    queryFn: ({ signal }) =>
      readActivityRoute(() => athleteApi.getActivityRoute(provider, activityId, { burst, signal }), signal),
    enabled,
    staleTime: routeAnswerStaleTime,
    refetchInterval: routeRecheckInterval,
    retry: retryRouteRead,
  });
  const retry = useCallback(() => {
    queryClient
      .fetchQuery({
        queryKey: QUERY_KEYS.home.activityRoute(provider, activityId),
        queryFn: ({ signal }) =>
          readActivityRoute(
            () => athleteApi.getActivityRoute(provider, activityId, { retry: true, signal }),
            signal,
          ),
        staleTime: 0,
      })
      .catch(() => undefined);
  }, [queryClient, provider, activityId]);
  return {
    data: query.data,
    isError: query.isError,
    isFetching: query.isFetching,
    retry,
  };
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
 * The athlete's training status on their own today: form and its band, the
 * form trend, the load ratio and the recovery days.
 *
 * The server computes it from its stored activities, so it moves when they
 * do: a refreshed activity list invalidates nothing here on its own, and the
 * page reads it again on mount and on focus like the plan.
 */
export function useTrainingStatus() {
  return useQuery<TrainingStatusResponse>({
    queryKey: QUERY_KEYS.home.trainingStatus(),
    queryFn: () => athleteApi.getTrainingStatus(),
  });
}

/**
 * The question the empty chat suggests: a route for today's session.
 *
 * When the plan holds a session today in a sport with routes, the draft names
 * it — the day, the workout, its distance when the plan gives one — exactly
 * as Home's link does. With no plan, no session today, or the plan not yet
 * read, it is the plain question and the agent looks the session up itself.
 */
export function useTodayRouteDraft(): string {
  const { t, language } = useTranslation();
  const plan = useTrainingPlan();
  const response = plan.data;
  if (response?.plan) {
    try {
      const named = planDayRouteDraft(t, language, response.today, planDayOn(response.plan, response.today));
      if (named !== null) return named;
    } catch (error) {
      // A `today` that is not a calendar day cannot be named; the plain
      // question still can be asked.
      if (!(error instanceof RangeError)) throw error;
    }
  }
  return t('chat.quickRouteDraft');
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
