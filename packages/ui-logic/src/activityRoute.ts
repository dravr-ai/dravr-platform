// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Follows one Home route read through the server's `pending` answers to the route or the reason there is none
// ABOUTME: Both clients' route queries read through it; it keeps asking within the bound each `pending` names, never a fixed count

import { HOME_ROUTE_PENDING_BACKOFF_MS, HOME_ROUTE_PENDING_SLACK_MS } from '@pierre/shared-constants';
import type { ActivityRouteAnswer, ActivityRouteResponse } from '@pierre/shared-types';

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
