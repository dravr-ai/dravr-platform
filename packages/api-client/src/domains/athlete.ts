// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Athlete domain API — the Home page's reads: recent activities, one activity's route, the plan for today
// ABOUTME: Each read checks the body with its shared-types parser, so a malformed answer is an error rather than a half-drawn card

import type { AxiosInstance } from 'axios';
import {
  parseActivityRouteResponse,
  parseRecentActivitiesResponse,
  parseTrainingPlanResponse,
  type ActivityRouteResponse,
  type ActivityRouteUnavailableReason,
  type HomeActivity,
  type RecentActivitiesResponse,
  type TrainingPlanResponse,
} from '@pierre/shared-types';
import { ENDPOINTS } from '../core/endpoints';

// Re-export types for consumers
export type {
  ActivityRouteResponse,
  ActivityRouteUnavailableReason,
  HomeActivity,
  RecentActivitiesResponse,
  TrainingPlanResponse,
};

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
     * `stale: true`; ask once more after `HOME_STALE_REFETCH_DELAY_MS`.
     * `limit` is clamped to 1..=20 by the server; omitted, it is 5.
     */
    async getRecentActivities(limit?: number): Promise<RecentActivitiesResponse> {
      const response = await axios.get<unknown>(ENDPOINTS.ATHLETE.RECENT_ACTIVITIES, {
        params: limit === undefined ? undefined : { limit },
      });
      return requireShape(parseRecentActivitiesResponse(response.data), ENDPOINTS.ATHLETE.RECENT_ACTIVITIES);
    },

    /**
     * One activity's route, ready for the map component, or the reason there
     * is none (`no_gps`, `too_short`) — a refusal is an answer, not an error.
     *
     * The first read of an activity without a stored track may reach the
     * provider; every later read is served from the stored track.
     */
    async getActivityRoute(provider: string, activityId: string): Promise<ActivityRouteResponse> {
      const endpoint = ENDPOINTS.ATHLETE.ACTIVITY_ROUTE(provider, activityId);
      const response = await axios.get<unknown>(endpoint);
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
