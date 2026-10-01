// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: recentActivitiesSync names the Home list's one sync state from what is in flight, and the failure only when nothing is
// ABOUTME: useRequestsInFlight counts overlapping requests until the last one settles, whether it resolves or rejects

import { describe, expect, it } from 'vitest';
import { act, renderHook } from '@testing-library/react';
import type { SyncFailure } from '@pierre/shared-types';
import { recentActivitiesSync, useRequestsInFlight } from '../src/recentActivitiesSync';

const FAILURE: SyncFailure = {
  provider: 'strava',
  provider_name: 'Strava',
  failed_at: '2026-10-01T08:00:00Z',
  last_synced_at: '2026-09-30T06:00:00Z',
};

/** A promise and the two hands that settle it. */
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe('recentActivitiesSync', () => {
  it('is fetching while the server refresh is followed up', () => {
    expect(recentActivitiesSync({ refreshing: true, retrying: false, syncFailure: null })).toBe('fetching');
  });

  it("is fetching while the athlete's retry is in flight, over the failure it retries", () => {
    expect(recentActivitiesSync({ refreshing: false, retrying: true, syncFailure: FAILURE })).toBe('fetching');
  });

  it('is fetching while a refresh after a failure runs', () => {
    expect(recentActivitiesSync({ refreshing: true, retrying: false, syncFailure: FAILURE })).toBe('fetching');
  });

  it('is failed once nothing is in flight and a provider failed', () => {
    expect(recentActivitiesSync({ refreshing: false, retrying: false, syncFailure: FAILURE })).toBe('failed');
  });

  it('is settled once nothing is in flight and nothing failed', () => {
    expect(recentActivitiesSync({ refreshing: false, retrying: false, syncFailure: null })).toBe('settled');
  });
});

describe('useRequestsInFlight', () => {
  it('is in flight until the last of two overlapping requests settles', async () => {
    const { result } = renderHook(() => useRequestsInFlight());
    expect(result.current.inFlight).toBe(false);

    const first = deferred<string>();
    const second = deferred<string>();
    let tracked: Promise<string> | undefined;
    act(() => {
      tracked = result.current.track(first.promise);
      result.current.track(second.promise).catch(() => undefined);
    });
    expect(result.current.inFlight).toBe(true);

    await act(async () => {
      first.resolve('answer');
      await first.promise;
    });
    expect(result.current.inFlight).toBe(true);
    await expect(tracked).resolves.toBe('answer');

    await act(async () => {
      second.reject(new Error('offline'));
      await second.promise.catch(() => undefined);
    });
    expect(result.current.inFlight).toBe(false);
  });
});
