// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Follows one Home route read through the server's `pending` answers to the route or the reason there is none
// ABOUTME: With the route query's read policy — how long an answer stays fresh, when `unavailable` is rechecked, one retry

import type { Query } from '@tanstack/react-query';
import {
  HOME_ROUTE_PENDING_BACKOFF_MS,
  HOME_ROUTE_PENDING_SLACK_MS,
  HOME_ROUTE_UNAVAILABLE_RECHECK_DELAYS_MS,
} from '@pierre/shared-constants';
import type { ActivityRouteAnswer, ActivityRouteResponse } from '@pierre/shared-types';
import { classifyApiError } from './apiError';

/** Wait `ms`, or reject with the signal's reason once it aborts. */
function pause(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal?.aborted === true) {
      reject(signal.reason ?? new Error('route read aborted'));
      return;
    }
    const onAbort = () => {
      clearTimeout(timer);
      reject(signal?.reason ?? new Error('route read aborted'));
    };
    const timer = setTimeout(() => {
      signal?.removeEventListener('abort', onAbort);
      resolve();
    }, ms);
    signal?.addEventListener('abort', onAbort, { once: true });
  });
}

/**
 * One activity's route, asked through `ask` until the server answers
 * something other than `pending`.
 *
 * `pending` means the read is queued behind the athlete's other route reads
 * or is still running, and each one names how long the read can still take by
 * the server's own bounds (`settles_within_secs`). The read is asked again —
 * at once, then spaced by {@link HOME_ROUTE_PENDING_BACKOFF_MS} — for as long
 * as the server keeps answering `pending` within the bound the answer before
 * named, plus {@link HOME_ROUTE_PENDING_SLACK_MS}: a read queued behind
 * several slow ones keeps the map loading for as long as they may take. A
 * `pending` that arrives past that bound, or one that names none, is the
 * server overrunning its own word, and the read is answered `unavailable`,
 * which the map says could not be loaded and offers to retry.
 *
 * `signal` is the query's: once it aborts, nothing more is asked.
 */
export async function readActivityRoute(
  ask: () => Promise<ActivityRouteAnswer>,
  signal?: AbortSignal,
): Promise<ActivityRouteResponse> {
  let deadline = Number.POSITIVE_INFINITY;
  for (let follow = 0; ; follow += 1) {
    const answer = await ask();
    if (answer.reason !== 'pending') {
      return answer;
    }
    if (signal?.aborted === true) {
      throw signal.reason ?? new Error('route read aborted');
    }
    const now = Date.now();
    if (now > deadline || answer.settles_within_secs === null) {
      return { route: null, reason: 'unavailable' };
    }
    deadline = now + answer.settles_within_secs * 1000 + HOME_ROUTE_PENDING_SLACK_MS;
    const wait = HOME_ROUTE_PENDING_BACKOFF_MS[Math.min(follow, HOME_ROUTE_PENDING_BACKOFF_MS.length - 1)] ?? 0;
    if (wait > 0) {
      await pause(wait, signal);
    }
  }
}

/**
 * Whether a failed route read is asked once more before the map says so.
 *
 * Once, not the client's default: a route read can reach the athlete's
 * provider, each attempt is bounded by `ACTIVITY_ROUTE_REQUEST_TIMEOUT_MS`
 * (30 s, `@pierre/api-client`), and every attempt holds the map on its loading
 * line — three retries kept "Loading the map…" up for about two minutes before
 * the athlete was told anything. After one retry the map says it could not be
 * loaded and offers a retry of its own. An authorization refusal, or a 404 for
 * an activity that is not the caller's, is the server's answer and is not
 * asked again.
 */
export function retryRouteRead(failureCount: number, error: Error): boolean {
  const { kind } = classifyApiError(error);
  if (kind === 'unauthorized' || kind === 'forbidden' || kind === 'notFound') {
    return false;
  }
  return failureCount < 1;
}

/**
 * How long a route answer stays fresh: a drawn route or a settled
 * `too_short` for good — a finished activity's route never changes and the
 * server stores it once — and every other answer not at all. `unavailable` is
 * a read that did not settle anything yet, and a `no_gps` held while the list
 * still says `has_gps: true` is an answer the server has since taken back
 * (the query is disabled once the list agrees), so both are asked again
 * whenever the page mounts or regains focus.
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
 * then never — a schedule with an end, never an interval. It picks up a route
 * a read made since — the athlete's retry, here or on another device — has
 * stored over the `unavailable`.
 */
export function routeRecheckInterval(query: Query<ActivityRouteResponse>): number | false {
  if (query.state.data?.reason !== 'unavailable') {
    return false;
  }
  return HOME_ROUTE_UNAVAILABLE_RECHECK_DELAYS_MS[query.state.dataUpdateCount - 1] ?? false;
}
