// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: What the Home activity list says about its provider sync — fetching, failed or settled — on web and mobile alike
// ABOUTME: One derivation from the real in-flight state, plus the in-flight tracker the athlete's Retry is counted with

import { useCallback, useEffect, useRef, useState } from 'react';
import type { SyncFailure } from '@pierre/shared-types';

/**
 * Where the Home activity list stands against the athlete's providers.
 *
 * - `fetching` — new activities are being read from a provider right now:
 *   the list shows its in-progress row at the top, and no failure.
 * - `failed` — nothing is being read, and the latest refresh of a provider
 *   failed: the list shows the failure and its retry, and no in-progress row.
 * - `settled` — nothing is being read and nothing failed.
 *
 * Exactly one at a time, so the in-progress row and the failure are never on
 * the page together.
 */
export type RecentActivitiesSync = 'fetching' | 'failed' | 'settled';

/** The in-flight state the list's sync is derived from. */
export interface RecentActivitiesSyncInput {
  /**
   * The server answered `stale: true` — it started, or found running, a
   * background refresh — and the client's follow-up schedule still owes a
   * read: the refresh is running as far as the page can know.
   */
  refreshing: boolean;
  /** The athlete's Retry after a failed sync is in flight. */
  retrying: boolean;
  /** The provider whose latest refresh failed, from the last answer; null when none did. */
  syncFailure: SyncFailure | null;
}

/**
 * The list's sync, from what is in flight.
 *
 * A read in flight is what the athlete is waiting on, so it is said first: a
 * refresh the server runs after a failure — its own retry once the pause
 * ends, or the one the athlete's Retry asked for — is a new attempt, and the
 * failure it may replace is said again only if that attempt ends without a
 * good sync.
 */
export function recentActivitiesSync({ refreshing, retrying, syncFailure }: RecentActivitiesSyncInput): RecentActivitiesSync {
  if (refreshing || retrying) {
    return 'fetching';
  }
  return syncFailure !== null ? 'failed' : 'settled';
}

/** What {@link useRequestsInFlight} returns. */
export interface RequestsInFlight {
  /** True while at least one tracked request has neither resolved nor rejected. */
  inFlight: boolean;
  /** Count `request` as in flight until it settles; it is returned untouched. */
  track: <T>(request: Promise<T>) => Promise<T>;
}

/**
 * Whether any of the requests handed to `track` is still in flight — a count,
 * so two overlapping requests read as in flight until the second settles. A
 * request that settles after the component unmounted changes nothing.
 */
export function useRequestsInFlight(): RequestsInFlight {
  const [count, setCount] = useState(0);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const track = useCallback(<T>(request: Promise<T>): Promise<T> => {
    setCount((current) => current + 1);
    const settle = () => {
      if (mounted.current) {
        setCount((current) => current - 1);
      }
    };
    request.then(settle, settle);
    return request;
  }, []);
  return { inFlight: count > 0, track };
}
