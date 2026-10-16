// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query reads behind Home on both clients — recent activities, one activity's route, the plan, status and volume
// ABOUTME: A stale activity answer is followed up on a schedule that ends, and only while somebody is using the app; never polled

import { useCallback, useEffect, useRef, useState } from 'react';
import { focusManager, useQuery, useQueryClient, type UseQueryResult } from '@tanstack/react-query';
import { HOME_STALE_REFETCH_DELAYS_MS, QUERY_KEYS } from '@pierre/shared-constants';
import type { AthleteApi } from '@pierre/api-client';
import type {
  ActivityRouteResponse,
  RecentActivitiesResponse,
  TrainingPlanResponse,
  TrainingStatusResponse,
  TrainingVolumeResponse,
} from '@pierre/shared-types';
import { readActivityRoute, retryRouteRead, routeAnswerStaleTime, routeRecheckInterval } from './activityRoute';
import { recentActivitiesSync, useRequestsInFlight, type RecentActivitiesSync } from './recentActivitiesSync';

/** The athlete reads Home binds, from each client's athlete API. */
export type HomeReadApi = Pick<
  AthleteApi,
  'getRecentActivities' | 'getActivityRoute' | 'getTrainingPlan' | 'getTrainingStatus' | 'getTrainingVolume'
>;

/** What only the client knows: the language the app renders in, read with its own hook. */
export interface HomeReadPlatform {
  useLanguage: () => string;
}

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

/** What Home reads from the recent-activities query. */
export interface RecentActivitiesState {
  data: RecentActivitiesResponse | undefined;
  isPending: boolean;
  isError: boolean;
  isRefetching: boolean;
  /** Read the list again; a read already in flight is joined. Its identity never changes. */
  refetch: UseQueryResult<RecentActivitiesResponse>['refetch'];
  /**
   * The athlete's retry after a failed sync: asks the server to refresh the
   * failing provider now, past the pause it otherwise leaves it in, and
   * restarts the follow-up schedule from that answer.
   */
  retry: () => void;
  /** Where the stale-answer schedule stands. */
  staleRefetch: StaleRefetchPhase;
  /**
   * True while a read made since mount answered stale and the schedule still
   * owes a follow-up: the refresh is running as far as the page can know.
   */
  refreshing: boolean;
  /** True while the athlete's `retry` request is in flight. */
  retrying: boolean;
  /** Fetching, failed or settled — the one sync state the list shows ({@link recentActivitiesSync}). */
  sync: RecentActivitiesSync;
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

/** How a caller of `useActivityRoute` reads the route. */
export interface ActivityRouteOptions {
  /**
   * The caller is a Home list row, whose page asks for all its routes in one
   * burst: the server lets the burst queue before it reads, so the newest
   * activity is read first. An activity view's own map leaves it unset and is
   * read at once; the athlete's retry never carries it.
   */
  burst?: boolean;
}

/** The answers a schedule has judged: none, for a list no read has answered. */
const NO_ANSWER = { dataUpdatedAt: 0, errorUpdateCount: 0 } as const;

/**
 * Build Home's reads over one client's athlete API.
 *
 * Each client binds this once, in its own `hooks/useHome`; the keys, the read
 * policy and the stale-answer schedule are this one definition.
 */
export function createHomeReadHooks(athleteApi: HomeReadApi, { useLanguage }: HomeReadPlatform) {
  /**
   * The recent-activities read, one cache entry for every reader: Home's list,
   * which also runs the stale-answer schedule, and any line that only shows
   * what the list last read.
   */
  function recentActivitiesQuery() {
    return {
      queryKey: QUERY_KEYS.home.recentActivities(),
      queryFn: () => athleteApi.getRecentActivities(),
    };
  }

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
   * Each delay is a timer, cancelled when the page unmounts; when it expires
   * while the app is out of use — the idle watch marks a hidden, backgrounded
   * or untouched app "not focused" — the read waits for the athlete to come
   * back instead of spending a request nobody will see. A follow-up that falls
   * due while another read is in flight joins that read.
   *
   * One schedule runs at a time, on one timer. A stale answer from elsewhere —
   * a refocus, a pull to refresh — changes nothing while a schedule is
   * running, and starts a schedule once none is: an ended schedule is ended
   * for the answer it ended on, never for the ones after it. The athlete's
   * `retry` after a failed sync is the exception: the refresh it asks the
   * server for is what the page waits on now, so it drops the running
   * schedule and its answer starts one of its own. An answer the cache already
   * holds when the page mounts — left by an earlier visit, or restored from
   * disk on the phone — is judged like any other.
   */
  function useRecentActivities(): RecentActivitiesState {
    const queryClient = useQueryClient();
    const query = useQuery(recentActivitiesQuery());

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
    const retryRequest = useRequestsInFlight();
    const { track } = retryRequest;

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
      // Written into the entry the page reads, so the answer is judged like
      // any other; a failed retry leaves the rows it had.
      track(
        queryClient.fetchQuery({
          queryKey: QUERY_KEYS.home.recentActivities(),
          queryFn: () => athleteApi.getRecentActivities(undefined, { retry: true }),
          staleTime: 0,
        }),
      ).catch(() => undefined);
    }, [queryClient, track]);

    // What the page may say is under way. The server answers `stale` only
    // when a refresh it started — or one already running — is reading a
    // provider, so the answer is the evidence. An answer held from before this
    // mount is not: it can be hours old (the phone restores its cache from
    // disk), and the refresh it reported long finished. Until a read made
    // since this mount answers, nothing is said to be checking; the schedule
    // still judges the held answer, and its follow-up is the read that
    // confirms or ends it.
    const refreshing = stale && staleRefetch !== 'done' && query.isFetchedAfterMount;

    return {
      data: query.data,
      isPending: query.isPending,
      isError: query.isError,
      isRefetching: query.isRefetching,
      refetch,
      retry,
      staleRefetch,
      refreshing,
      retrying: retryRequest.inFlight,
      sync: recentActivitiesSync({
        refreshing,
        retrying: retryRequest.inFlight,
        syncFailure: query.data?.sync_failure ?? null,
      }),
    };
  }

  /**
   * One activity's route, or the reason there is none.
   *
   * `enabled` keeps a caller off the wire when there is nothing to ask: the
   * activity says `has_gps: false` — its route was read once and the recording
   * held no GPS — or the caller draws from the activity's own polyline. Every
   * other activity may have a route, one whose route was never read included,
   * so this answer, not the flag, is what says whether there is a track. A
   * read the server answers `pending` — queued behind the athlete's other
   * route reads, or still running — is asked again, for as long as the bound
   * each `pending` names, and stays loading ({@link readActivityRoute}); the
   * server hands its turn to the newest activity of a Home burst
   * (`options.burst`) first, so the page's big map is read before its
   * sketches. A drawn route is kept for good ({@link routeAnswerStaleTime});
   * `unavailable`, a read that finished without settling anything, is asked
   * again on its own a bounded number of times ({@link routeRecheckInterval}),
   * on focus, and at the athlete's `retry`, which the server reads past its
   * stored answer.
   */
  function useActivityRoute(
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
   * The active plan as `/plan` shows it, and the athlete-local today the
   * server projected it for. Keyed by the language the app renders in: the
   * server names the plan's flavour in it, so a language switch is a
   * different card. `data.plan === null` is the server saying there is no
   * plan, and is never read off a load that has not finished or has failed.
   */
  function useTrainingPlan(): UseQueryResult<TrainingPlanResponse> {
    const language = useLanguage();
    return useQuery<TrainingPlanResponse>({
      queryKey: QUERY_KEYS.home.trainingPlan(language),
      queryFn: () => athleteApi.getTrainingPlan(language),
    });
  }

  /**
   * The athlete's training status on their own today: form and its band, the
   * form trend, the load ratio and the recovery days. The server computes it
   * from its stored activities; `data.form === null` is the server saying the
   * stored history is too thin to read form from.
   */
  function useTrainingStatus(): UseQueryResult<TrainingStatusResponse> {
    return useQuery<TrainingStatusResponse>({
      queryKey: QUERY_KEYS.home.trainingStatus(),
      queryFn: () => athleteApi.getTrainingStatus(),
    });
  }

  /**
   * The athlete's weekly volume: distance, time and climbing per sport for
   * each of the last twelve weeks, summed by the server from its stored
   * activities. Reading it reaches no provider; `data.weeks` empty means
   * nothing is stored yet, never a load that failed.
   */
  function useTrainingVolume(): UseQueryResult<TrainingVolumeResponse> {
    return useQuery<TrainingVolumeResponse>({
      queryKey: QUERY_KEYS.home.trainingVolume(),
      queryFn: () => athleteApi.getTrainingVolume(),
    });
  }

  return {
    recentActivitiesQuery,
    useRecentActivities,
    useActivityRoute,
    useTrainingPlan,
    useTrainingStatus,
    useTrainingVolume,
  };
}
