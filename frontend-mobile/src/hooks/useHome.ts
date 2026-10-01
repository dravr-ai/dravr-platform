// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hooks behind the Home tab — recent activities, one activity's route, the plan for today, the provider status
// ABOUTME: A stale activity answer is followed up on the HOME_STALE_REFETCH_DELAYS_MS schedule, which ends, and only while the app is in use

import { useCallback, useEffect, useRef, useState } from 'react';
import { focusManager, useQuery, useQueryClient, type Query } from '@tanstack/react-query';
import {
  HOME_ROUTE_UNAVAILABLE_RECHECK_DELAYS_MS,
  HOME_STALE_REFETCH_DELAYS_MS,
  QUERY_KEYS,
} from '@pierre/shared-constants';
import type { ActivityRouteResponse } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { classifyApiError, readActivityRoute } from '@pierre/ui-logic';
import { athleteApi, oauthApi } from '../services/api';

/**
 * Where the follow-up reads for a stale answer stand.
 *
 * - `none` — no stale answer has arrived, so nothing is scheduled.
 * - `pending` — a schedule is running: the server said stale and started its
 *   own refresh, and a follow-up read is still owed.
 * - `done` — the schedule ended, on an answer that was not stale or on the
 *   answer to its last follow-up. A list still stale then is shown with its
 *   sync time, and nothing more is asked until another stale answer arrives,
 *   which starts a schedule of its own.
 */
export type StaleRefetchPhase = 'none' | 'pending' | 'done';

/** The answers a schedule has judged: none, for a list no read has answered. */
const NO_ANSWER = { dataUpdatedAt: 0, errorUpdateCount: 0 } as const;

/**
 * The newest cached activities, and the follow-up reads a stale answer earns.
 *
 * Reading this never reaches a provider: the server answers from its durable
 * cache and, when the cache is past its freshness window, refreshes it in the
 * background and says `stale`. That refresh can take minutes through a
 * scraped provider, so the hook follows {@link HOME_STALE_REFETCH_DELAYS_MS}:
 * it waits the first delay and asks, and while the answer is still stale it
 * waits the next delay and asks again, each wait measured from the answer
 * before it. The schedule ends at the first answer that is not stale, and
 * after the last delay whatever that answer says. A follow-up that fails is
 * an answer too: the list keeps the stale answer it had, and the schedule
 * moves on to its next delay.
 *
 * Each delay is a timer, cancelled when the screen unmounts; when it expires
 * while the app is backgrounded or idle — the idle watch expresses both as
 * "not focused" — the read waits for the athlete to come back instead of
 * spending a request nobody will see. A follow-up that falls due while
 * another read is in flight joins that read.
 *
 * One schedule runs at a time, on one timer. A stale answer from elsewhere —
 * a refocus, a pull to refresh — changes nothing while a schedule is
 * running, and starts a schedule once none is: an ended schedule is ended
 * for the answer it ended on, never for the ones after it. The athlete's
 * `retry` after a failed sync is the exception: the refresh it asks the
 * server for is what the screen waits on now, so it drops the running
 * schedule and its answer starts one of its own. An answer the cache already
 * holds when the screen mounts, restored from disk or left by an earlier
 * visit, is judged like any other.
 */
export function useRecentActivities() {
  const queryClient = useQueryClient();
  const query = useQuery({
    // Home asks for the server's default of five; the key says so with a null.
    queryKey: QUERY_KEYS.home.recentActivities(),
    queryFn: () => athleteApi.getRecentActivities(),
  });

  const stale = query.data?.stale === true;
  const { refetch, dataUpdatedAt, errorUpdateCount } = query;
  const [staleRefetch, setStaleRefetch] = useState<StaleRefetchPhase>('none');
  // The schedule is kept in refs: an answer is judged against where the
  // schedule stands when it arrives, not where it stood when an effect
  // closed over it, and a timer outlives the render that set it.
  const phase = useRef<StaleRefetchPhase>('none');
  /** How many follow-ups the running schedule has asked. */
  const asked = useRef(0);
  /** Whether the last of them is still waiting for its answer. */
  const awaiting = useRef(false);
  /** The newest answer judged, so each is judged once. */
  const judged = useRef<{ dataUpdatedAt: number; errorUpdateCount: number }>(NO_ANSWER);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const unsubscribeFocus = useRef<(() => void) | null>(null);

  // Unmounting cancels whatever is scheduled — the delay and the wait for
  // focus — and forgets the schedule with it.
  useEffect(
    () => () => {
      if (timer.current !== null) {
        clearTimeout(timer.current);
        timer.current = null;
      }
      unsubscribeFocus.current?.();
      unsubscribeFocus.current = null;
      phase.current = 'none';
      asked.current = 0;
      awaiting.current = false;
      judged.current = NO_ANSWER;
    },
    [],
  );

  useEffect(() => {
    const answered = dataUpdatedAt !== judged.current.dataUpdatedAt;
    const failed = errorUpdateCount !== judged.current.errorUpdateCount;
    if (!answered && !failed) {
      return;
    }
    judged.current = { dataUpdatedAt, errorUpdateCount };

    const end = () => {
      if (timer.current !== null) {
        clearTimeout(timer.current);
        timer.current = null;
      }
      unsubscribeFocus.current?.();
      unsubscribeFocus.current = null;
      awaiting.current = false;
      phase.current = 'done';
      setStaleRefetch('done');
    };

    const ask = () => {
      unsubscribeFocus.current?.();
      unsubscribeFocus.current = null;
      asked.current += 1;
      awaiting.current = true;
      // A read already in flight — a pull to refresh, a refocus — is joined,
      // never cancelled: its answer serves as this ask's answer.
      void refetch({ cancelRefetch: false });
    };

    /** Wait the schedule's next delay and ask, or end the schedule when it has none left. */
    const waitOrEnd = () => {
      if (asked.current >= HOME_STALE_REFETCH_DELAYS_MS.length) {
        end();
        return;
      }
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
      }, HOME_STALE_REFETCH_DELAYS_MS[asked.current]);
    };

    if (awaiting.current) {
      // The follow-up's own answer. One that failed left the stale answer it
      // followed in place, and moves the schedule on as a stale one does.
      awaiting.current = false;
      if (stale) {
        waitOrEnd();
      } else {
        end();
      }
      return;
    }
    if (!answered) {
      // A read that failed outside a follow-up says nothing about the cache.
      return;
    }
    if (phase.current === 'pending') {
      // An answer from elsewhere while a delay is running: a fresh one ends
      // the schedule, a stale one leaves it on its one timer.
      if (!stale) {
        end();
      }
      return;
    }
    if (stale) {
      asked.current = 0;
      phase.current = 'pending';
      setStaleRefetch('pending');
      waitOrEnd();
    }
  }, [dataUpdatedAt, errorUpdateCount, stale, refetch]);

  const retry = useCallback(() => {
    // Forget the running schedule, so the retry's own answer is judged as
    // the first of a new one.
    if (timer.current !== null) {
      clearTimeout(timer.current);
      timer.current = null;
    }
    unsubscribeFocus.current?.();
    unsubscribeFocus.current = null;
    phase.current = 'none';
    asked.current = 0;
    awaiting.current = false;
    setStaleRefetch('none');
    // Written into the entry the screen reads; a failed retry leaves the rows
    // it had.
    queryClient
      .fetchQuery({
        queryKey: QUERY_KEYS.home.recentActivities(),
        queryFn: () => athleteApi.getRecentActivities(undefined, { retry: true }),
        staleTime: 0,
      })
      .catch(() => undefined);
  }, [queryClient]);

  return {
    activities: query.data?.activities ?? [],
    asOf: query.data?.as_of ?? null,
    /** The provider whose latest refresh failed after its last good sync, or null. */
    syncFailure: query.data?.sync_failure ?? null,
    stale,
    staleRefetch,
    // What the page may say is under way. The server answers `stale` only
    // when a refresh it started — or one already running — is reading a
    // provider, so the answer is the evidence. An answer restored from disk
    // or left by an earlier visit is not: it can be hours old, and the
    // refresh it reported long finished. Until a read made since this mount
    // answers, nothing is said to be checking; the schedule still judges the
    // held answer, and its follow-up is the read that confirms or ends it.
    refreshing: stale && staleRefetch !== 'done' && query.isFetchedAfterMount,
    hasData: query.data !== undefined,
    isError: query.isError,
    isRefetching: query.isRefetching,
    refetch,
    /** The athlete's retry after a failed sync: the server refreshes the failing provider now. */
    retry,
  };
}

/**
 * Whether a failed route read is asked once more before the map says so.
 *
 * Once, not the client's default twice with backoff: a route read can reach
 * the athlete's provider, the route request is bounded by
 * `ACTIVITY_ROUTE_REQUEST_TIMEOUT_MS` (30 s, `@pierre/api-client`) rather
 * than the five minutes the phone's transport gives a chat turn, and each
 * attempt holds the map on its loading line. After one retry the map says it
 * could not be loaded and offers a retry of its own. An authorization
 * refusal, or a 404 for an activity that is not the caller's, is the server's
 * answer and is not asked again.
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
 * `too_short` for good — a completed activity's route never changes and the
 * server stores it after the first read — and every other answer not at all.
 * `unavailable` is a read that did not settle anything yet, and a `no_gps`
 * held while the row still says `has_gps: true` is one the server has since
 * taken back (the query is disabled once the row agrees), so both are asked
 * again whenever the screen mounts or the app comes back to the foreground.
 */
export function routeAnswerStaleTime(query: Query<ActivityRouteResponse>): number {
  const answer = query.state.data;
  return answer !== undefined && (answer.route !== null || answer.reason === 'too_short') ? Infinity : 0;
}

/**
 * When an `unavailable` route answer is asked again on its own: after each
 * wait of `HOME_ROUTE_UNAVAILABLE_RECHECK_DELAYS_MS`, counted in answers, and
 * then never — a schedule with an end, never an interval. It picks up a
 * route a read made since — the athlete's retry, here or on another device —
 * has stored over the `unavailable`.
 */
export function routeRecheckInterval(query: Query<ActivityRouteResponse>): number | false {
  if (query.state.data?.reason !== 'unavailable') {
    return false;
  }
  return HOME_ROUTE_UNAVAILABLE_RECHECK_DELAYS_MS[query.state.dataUpdateCount - 1] ?? false;
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
 * `enabled` keeps a caller off the wire when there is nothing to ask: the row
 * says `has_gps: false` — its route was read once and the recording held no
 * GPS, the answer the row already carries — or the caller draws from the
 * row's own polyline. Every other row may have a route, one whose route was
 * never read included: most providers' activity lists carry no position, so
 * this answer, not the flag, is what says whether there is a track. A read
 * the server answers `pending` — queued behind the athlete's other route
 * reads, or still running — is asked again, for as long as the bound each
 * `pending` names, and stays loading ({@link readActivityRoute}); the server
 * hands its turn to the newest activity of a Home burst (`options.burst`)
 * first, so the screen's big map is read before its sketches. A
 * drawn route is kept for good; `unavailable`, a read that finished without
 * settling anything, is asked again on its own a bounded number of times
 * ({@link routeRecheckInterval}), in the foreground, and at the athlete's
 * `retry`, which the server reads past its stored answer.
 */
export function useActivityRoute(
  provider: string,
  activityId: string,
  enabled: boolean,
  options: ActivityRouteOptions = {},
) {
  const burst = options.burst === true;
  const queryClient = useQueryClient();
  const query = useQuery({
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
    route: query.data?.route ?? null,
    reason: query.data?.reason ?? null,
    isError: query.isError,
    /** True while a read of the route is in flight, the retry's included. */
    isFetching: query.isFetching,
    retry,
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
 * Whether any fitness provider is connected, whether one of them still
 * syncs, and which connected ones have to be reconnected, from the provider
 * status the Connections pane reads — the activity list carries none of it.
 *
 * `connected` is `null` until the status answers; a status read that fails
 * reads as connected, because the empty sentence ("they show up once your
 * provider syncs") is true either way while telling a connected athlete to
 * connect is not.
 *
 * `needsReconnect` names the connected providers flagged `needs_reauth`, in
 * the order the server lists them. The server refreshes nothing for such a
 * connection, so its activities stay as old as its last sync. The flag only
 * means something on a connected provider, so a disconnected one is never
 * named; and two rows can carry one name — the `sciotte` mirror and the
 * `strava` OAuth row are both "Strava" — so a name is listed once.
 */
export function useProviderConnected() {
  const query = useQuery({
    queryKey: QUERY_KEYS.providers.status(),
    queryFn: () => oauthApi.getProvidersStatus(),
  });

  let connected: boolean | null = null;
  if (query.data !== undefined) {
    connected = query.data.providers.some((provider) => provider.connected);
  } else if (query.isError) {
    connected = true;
  }

  const needsReconnect = [
    ...new Set(
      (query.data?.providers ?? [])
        .filter((provider) => provider.connected && provider.needs_reauth)
        .map((provider) => provider.display_name),
    ),
  ];

  // Whether a connected provider is still syncing — one not flagged. Same
  // reading as `connected`: `null` until the status answers, and a failed
  // read counts as syncing, since nothing then says otherwise.
  let syncing: boolean | null = null;
  if (query.data !== undefined) {
    syncing = query.data.providers.some((provider) => provider.connected && !provider.needs_reauth);
  } else if (query.isError) {
    syncing = true;
  }

  return {
    connected,
    syncing,
    needsReconnect,
    // The rows themselves, for a reader that words them its own way — the
    // thread header. `null` until the status answers.
    providers: query.data?.providers ?? null,
    refetch: query.refetch,
  };
}
