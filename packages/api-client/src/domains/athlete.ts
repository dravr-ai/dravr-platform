// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Athlete domain API — the Home page's reads: recent activities, one activity's view and route, the plan for today
// ABOUTME: Each read checks the body with its shared-types parser, so a malformed answer is an error rather than a half-drawn card

import type { AxiosInstance } from 'axios';
import {
  parseActivityDetailResponse,
  parseActivityRouteResponse,
  parseRecentActivitiesResponse,
  parseTrainingPlanResponse,
  type ActivityDetailResponse,
  type ActivityRouteAnswer,
  type ActivityRouteResponse,
  type ActivityRouteUnavailableReason,
  type HomeActivity,
  type RecentActivitiesResponse,
  type TrainingPlanResponse,
} from '@pierre/shared-types';
import { ENDPOINTS } from '../core/endpoints';

// Re-export types for consumers
export type {
  ActivityDetailResponse,
  ActivityRouteResponse,
  ActivityRouteUnavailableReason,
  HomeActivity,
  RecentActivitiesResponse,
  TrainingPlanResponse,
};

/**
 * How long one route read may take on the wire before the client gives up on
 * it, whatever the transport's own default — the phone's is five minutes, for
 * chat turns.
 *
 * Above the server's own bound on a route answer (25 s,
 * `ROUTE_READ_TIMEOUT_SECS` in pierre-server), which always answers a route,
 * a reason or `pending` in time; this one only fires on a connection that
 * stalled, so the map says it could not be loaded instead of loading for
 * minutes.
 */
export const ACTIVITY_ROUTE_REQUEST_TIMEOUT_MS = 30_000;

/** Options of one Home read. */
export interface HomeReadOptions {
  /**
   * The athlete's own retry after a failure: the server reads the provider
   * again at once, past the pause (recent activities) or the stored
   * `unavailable` answer (a route) that otherwise holds it off.
   */
  retry?: boolean;
}

/** Options of one route read. */
export interface ActivityRouteReadOptions extends HomeReadOptions {
  /**
   * The read is one of a Home list's burst of route reads: finding the
   * athlete's provider turn free, the server waits briefly for the rest of
   * the burst so the newest activity is read first. Any other read — an
   * activity view's own map — leaves it unset and is read at once.
   */
  burst?: boolean;
  signal?: AbortSignal;
}

/**
 * The parsed body, or an error naming the endpoint whose answer broke the
 * contract — the query then shows its error state instead of a card built
 * from a shape nobody agreed to.
 */
function requireShape<T>(parsed: T | null, endpoint: string): T {
  if (parsed === null) {
    throw new Error(`${endpoint} answered with a body that does not match its contract`);
  }
  return parsed;
}

/**
 * Creates the athlete API methods bound to an axios instance.
 */
export function createAthleteApi(axios: AxiosInstance) {
  return {
    /**
     * The athlete's newest activities, newest first, from the durable cache.
     *
     * Reading it never calls a provider. When the cache is older than the
     * freshness window the server starts a background refresh and answers
     * `stale: true`; ask again on the `HOME_STALE_REFETCH_DELAYS_MS` schedule.
     * `sync_failure` names a provider whose refresh failed after its last good
     * sync; the server leaves a failing provider alone for a pause, which the
     * athlete's `retry` reads past.
     * `limit` is clamped to 1..=20 by the server; omitted, it is 5.
     */
    async getRecentActivities(limit?: number, options: HomeReadOptions = {}): Promise<RecentActivitiesResponse> {
      const params = {
        ...(limit === undefined ? {} : { limit }),
        ...(options.retry === true ? { retry: true } : {}),
      };
      const response = await axios.get<unknown>(ENDPOINTS.ATHLETE.RECENT_ACTIVITIES, {
        params: Object.keys(params).length === 0 ? undefined : params,
      });
      return requireShape(parseRecentActivitiesResponse(response.data), ENDPOINTS.ATHLETE.RECENT_ACTIVITIES);
    },

    /**
     * One workout's own view — its Home row, the figures the server's cache
     * holds for it, its splits and laps — served from the cache. The one
     * provider call it can make is the workout's detail read, once per
     * workout while no copy of it has had one, which the server waits for a
     * few seconds at most. A workout the athlete does not hold answers 404.
     */
    async getActivityDetail(
      provider: string,
      activityId: string,
      options: { signal?: AbortSignal } = {},
    ): Promise<ActivityDetailResponse> {
      const endpoint = ENDPOINTS.ATHLETE.ACTIVITY_DETAIL(provider, activityId);
      const response = await axios.get<unknown>(endpoint, { signal: options.signal });
      return requireShape(parseActivityDetailResponse(response.data), endpoint);
    },

    /**
     * Link the conversation an activity's view opened to the activity, or
     * forget the link with `null`, so reopening the activity — on this device
     * or another — resumes that thread. Only the athlete's own conversation
     * can be linked; any other answers 404.
     */
    async linkActivityConversation(
      provider: string,
      activityId: string,
      conversationId: string | null,
    ): Promise<void> {
      await axios.put(ENDPOINTS.ATHLETE.ACTIVITY_CONVERSATION(provider, activityId), {
        conversation_id: conversationId,
      });
    },

    /**
     * One activity's route, ready for the map component, or the reason there
     * is none (`no_gps`, `too_short`) — a refusal is an answer, not an error —
     * or `unavailable` when a read finished without settling it, which the
     * server reads again on a later request, or at once for a `retry`; or
     * `pending` when no read has finished yet, with how long it can still
     * take by the server's bounds, which is asked again within that
     * (`readActivityRoute` in `@pierre/ui-logic`).
     *
     * The first read of an activity without a stored track may reach the
     * provider; every later read is served from the stored track. Bounded by
     * {@link ACTIVITY_ROUTE_REQUEST_TIMEOUT_MS}, and cancelled with `signal`.
     */
    async getActivityRoute(
      provider: string,
      activityId: string,
      options: ActivityRouteReadOptions = {},
    ): Promise<ActivityRouteAnswer> {
      const endpoint = ENDPOINTS.ATHLETE.ACTIVITY_ROUTE(provider, activityId);
      const params = {
        ...(options.retry === true ? { retry: true } : {}),
        ...(options.burst === true ? { burst: true } : {}),
      };
      const response = await axios.get<unknown>(endpoint, {
        params: Object.keys(params).length > 0 ? params : undefined,
        timeout: ACTIVITY_ROUTE_REQUEST_TIMEOUT_MS,
        signal: options.signal,
      });
      return requireShape(parseActivityRouteResponse(response.data), endpoint);
    },

    /**
     * The athlete's active plan as `/plan` shows it — the current and next
     * week — plus the athlete-local today it was projected for.
     *
     * `locale` names the flavour in the language the app is rendering, so the
     * card follows a language switch without waiting for the account to be
     * saved; omitted, the server uses the account's stored locale.
     */
    async getTrainingPlan(locale?: string): Promise<TrainingPlanResponse> {
      const response = await axios.get<unknown>(ENDPOINTS.ATHLETE.TRAINING_PLAN, {
        params: locale ? { locale } : undefined,
      });
      return requireShape(parseTrainingPlanResponse(response.data), ENDPOINTS.ATHLETE.TRAINING_PLAN);
    },
  };
}

export type AthleteApi = ReturnType<typeof createAthleteApi>;
