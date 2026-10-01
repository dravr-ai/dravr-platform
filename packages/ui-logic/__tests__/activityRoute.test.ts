// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: readActivityRoute follows `pending` route answers to the route for as long as the bound each one names
// ABOUTME: Pins that a read queued behind several slow ones is never reported as a map that could not be loaded

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HOME_ROUTE_PENDING_BACKOFF_MS, HOME_ROUTE_PENDING_SLACK_MS } from '@pierre/shared-constants';
import type { ActivityRouteAnswer, ActivityRouteResponse } from '@pierre/shared-types';
import { readActivityRoute } from '../src/activityRoute';

/** The provider-read bound the server names per read (`ROUTE_PROVIDER_READ_TIMEOUT_SECS`). */
const READ_BOUND_SECS = 330;

/** A `pending` answer with `ahead` reads ahead of it in the athlete's turn. */
function pending(ahead: number): ActivityRouteAnswer {
  return { route: null, reason: 'pending', settles_within_secs: (ahead + 1) * READ_BOUND_SECS };
}

const DRAWN: ActivityRouteResponse = {
  route: {
    coordinates: [
      [45.5259, -73.5697],
      [45.5301, -73.5652],
    ],
    bounds: { min_latitude: 45.5259, max_latitude: 45.5301, min_longitude: -73.5697, max_longitude: -73.5652 },
    elevation_meters: null,
    distances_meters: null,
    climbs: [],
    title: 'Morning Trail Run',
    source_tool: 'strava',
  },
  reason: null,
};

/**
 * An `ask` that answers `answers` in order, each after the server held it
 * `holdMs` (as it holds a queued ask up to its own bound), then fails the
 * test.
 */
function scripted(holdMs: number, ...answers: ActivityRouteAnswer[]) {
  const queue = [...answers];
  return vi.fn(async () => {
    const next = queue.shift();
    if (next === undefined) throw new Error('asked once too often');
    if (holdMs > 0) {
      await new Promise((resolve) => setTimeout(resolve, holdMs));
    }
    return next;
  });
}

/** Run the read to its end, advancing the fake clock as it waits. */
async function settle(read: Promise<ActivityRouteResponse>): Promise<ActivityRouteResponse> {
  let outcome: ActivityRouteResponse | undefined;
  let failure: unknown;
  let done = false;
  read.then(
    (value) => {
      outcome = value;
      done = true;
    },
    (error: unknown) => {
      failure = error;
      done = true;
    },
  );
  while (!done) {
    await vi.advanceTimersByTimeAsync(1000);
  }
  if (failure !== undefined) throw failure;
  return outcome as ActivityRouteResponse;
}

describe('readActivityRoute', () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
    vi.setSystemTime(new Date('2026-09-30T12:00:00Z'));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('answers a settled answer as it came', async () => {
    const ask = scripted(0, { route: null, reason: 'no_gps' });
    await expect(settle(readActivityRoute(ask))).resolves.toEqual({ route: null, reason: 'no_gps' });
    expect(ask).toHaveBeenCalledTimes(1);
  });

  it('asks again after pending until the queued read lands, and draws it', async () => {
    const ask = scripted(0, pending(2), pending(1), pending(0), DRAWN);
    await expect(settle(readActivityRoute(ask))).resolves.toEqual(DRAWN);
    expect(ask).toHaveBeenCalledTimes(4);
  });

  // The fixed fourteen asks (~350 s) said "could not be loaded" while a read
  // queued behind several slow ones was still coming.
  it('follows a read queued behind several slow ones far past any fixed count of asks', async () => {
    const started = Date.now();
    // Five reads ahead, each a full provider-read bound: thirty-odd minutes
    // of 25-second held asks, every one answered within the bound before it.
    const asks = Math.ceil((5 * READ_BOUND_SECS) / 25);
    const answers = Array.from({ length: asks }, (_, index) =>
      pending(Math.max(0, 5 - Math.floor((index * 25) / READ_BOUND_SECS))),
    );
    const ask = scripted(25_000, ...answers, DRAWN);
    await expect(settle(readActivityRoute(ask))).resolves.toEqual(DRAWN);
    expect(ask).toHaveBeenCalledTimes(asks + 1);
    expect(asks).toBeGreaterThan(14);
    expect(Date.now() - started).toBeGreaterThan(14 * 25_000);
  });

  it('spaces the asks that come back at once by the backoff, starting immediately', async () => {
    const answers = Array.from({ length: HOME_ROUTE_PENDING_BACKOFF_MS.length + 2 }, () => pending(0));
    const ask = scripted(0, ...answers, DRAWN);
    const read = readActivityRoute(ask);
    await vi.advanceTimersByTimeAsync(0);
    expect(ask).toHaveBeenCalledTimes(2);
    let asked = 2;
    for (const wait of [...HOME_ROUTE_PENDING_BACKOFF_MS.slice(1), HOME_ROUTE_PENDING_BACKOFF_MS.at(-1) ?? 0]) {
      await vi.advanceTimersByTimeAsync(wait - 1);
      expect(ask).toHaveBeenCalledTimes(asked);
      await vi.advanceTimersByTimeAsync(1);
      asked += 1;
      expect(ask).toHaveBeenCalledTimes(asked);
    }
    await expect(settle(read)).resolves.toEqual(DRAWN);
  });

  it('gives up as unavailable only once a pending overruns the bound the one before named', async () => {
    const boundMs = READ_BOUND_SECS * 1000 + HOME_ROUTE_PENDING_SLACK_MS;
    // The second ask is held past the first answer's bound and its margin.
    const ask = vi
      .fn<() => Promise<ActivityRouteAnswer>>()
      .mockResolvedValueOnce(pending(0))
      .mockImplementationOnce(
        () => new Promise((resolve) => setTimeout(() => resolve(pending(0)), boundMs + 1)),
      );
    await expect(settle(readActivityRoute(ask))).resolves.toEqual({ route: null, reason: 'unavailable' });
    expect(ask).toHaveBeenCalledTimes(2);
  });

  it('gives up as unavailable on a pending that names no bound', async () => {
    const ask = scripted(0, { route: null, reason: 'pending', settles_within_secs: null });
    await expect(settle(readActivityRoute(ask))).resolves.toEqual({ route: null, reason: 'unavailable' });
    expect(ask).toHaveBeenCalledTimes(1);
  });

  it('stops asking once the query is cancelled, mid-backoff too', async () => {
    const controller = new AbortController();
    const ask = vi.fn(async () => {
      controller.abort(new Error('unmounted'));
      return pending(0);
    });
    await expect(settle(readActivityRoute(ask, controller.signal))).rejects.toThrow('unmounted');
    expect(ask).toHaveBeenCalledTimes(1);

    const later = new AbortController();
    const slow = vi.fn(async () => pending(0));
    const read = readActivityRoute(slow, later.signal);
    await vi.advanceTimersByTimeAsync(0);
    expect(slow).toHaveBeenCalledTimes(2);
    later.abort(new Error('left the page'));
    await expect(settle(read)).rejects.toThrow('left the page');
    expect(slow).toHaveBeenCalledTimes(2);
  });
});
