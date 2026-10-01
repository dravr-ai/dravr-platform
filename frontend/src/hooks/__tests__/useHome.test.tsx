// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the Home reads — a stale activity answer is followed up on the shared schedule, while focused, until it ends
// ABOUTME: Red if the page polls, asks early, asks an idle tab, runs two schedules, stays stuck after one, or drops the plan's language

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { act, renderHook } from '@testing-library/react';
import type { ReactNode } from 'react';
import { QueryClient, QueryClientProvider, focusManager, notifyManager } from '@tanstack/react-query';
import {
  HOME_ROUTE_PENDING_BACKOFF_MS,
  HOME_ROUTE_UNAVAILABLE_RECHECK_DELAYS_MS,
  HOME_STALE_REFETCH_DELAYS_MS,
} from '@pierre/shared-constants';
import type {
  ActivityRouteAnswer,
  ActivityRouteResponse,
  HomeActivity,
  ProvidersStatusResponse,
  RecentActivitiesResponse,
  TrainingPlanResponse,
} from '@pierre/shared-types';
import { useActivityRoute, useProviderConnection, useRecentActivities, useTrainingPlan } from '../useHome';

const api = vi.hoisted(() => ({
  getRecentActivities:
    vi.fn<(limit?: number, options?: { retry?: boolean }) => Promise<RecentActivitiesResponse>>(),
  getActivityRoute:
    vi.fn<
      (
        provider: string,
        id: string,
        options?: { retry?: boolean; burst?: boolean; signal?: AbortSignal },
      ) => Promise<ActivityRouteAnswer>
    >(),
  getTrainingPlan: vi.fn<(locale?: string) => Promise<TrainingPlanResponse>>(),
  getProvidersStatus: vi.fn<() => Promise<ProvidersStatusResponse>>(),
}));

vi.mock('../../services/api', () => ({
  athleteApi: api,
  providersApi: { getProvidersStatus: api.getProvidersStatus },
}));

function row(id: string, name: string, start_date: string): HomeActivity {
  return {
    id,
    provider: 'garmin',
    name,
    sport_type: 'run',
    start_date,
    duration_seconds: 3600,
    distance_meters: 10200,
    elevation_gain_meters: 85,
    has_gps: true,
    summary_polyline: null,
  };
}

const OLD_ROW = row('act-1', 'Lake loop', '2026-09-22T11:00:00Z');
const NEW_ROW = row('act-2', 'Lunch run', '2026-09-24T12:00:00Z');

const STALE: RecentActivitiesResponse = {
  activities: [OLD_ROW],
  as_of: '2026-09-23T06:00:00Z',
  sync_failure: null,
  stale: true,
};
const FRESH: RecentActivitiesResponse = {
  activities: [NEW_ROW, OLD_ROW],
  as_of: '2026-09-24T12:30:00Z',
  sync_failure: null,
  stale: false,
};

const DELAYS = HOME_STALE_REFETCH_DELAYS_MS;
const WHOLE_SCHEDULE = DELAYS.reduce((sum, delay) => sum + delay, 0);

function wrapper({ children }: { children: ReactNode }) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

/** Let every resolved promise and zero-delay timer run, under act. */
async function flush(ms = 0) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

/** Run a whole schedule of stale answers, checking that each ask waits out its own delay. */
async function runScheduleOut(callsBefore: number) {
  for (const [index, delay] of DELAYS.entries()) {
    await flush(delay - 1);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(callsBefore + index);
    await flush(1);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(callsBefore + index + 1);
  }
}

beforeEach(() => {
  vi.clearAllMocks();
  // The clock is faked with the timers: an answer is told from the one before
  // it by when it arrived, so two answers must never share an instant.
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
  vi.setSystemTime(new Date('2026-09-24T12:00:00Z'));
  // React Query batches its notifications on a zero-delay timeout; run them
  // inline so the fake clock only has to drive the hook's own delays.
  notifyManager.setScheduler((callback) => callback());
  focusManager.setFocused(true);
});

afterEach(() => {
  vi.useRealTimers();
  notifyManager.setScheduler((callback) => setTimeout(callback, 0));
  focusManager.setFocused(undefined);
});

describe('useRecentActivities', () => {
  it('follows a stale answer up until one is fresh, shows its rows, and stops asking', async () => {
    api.getRecentActivities
      .mockResolvedValueOnce(STALE)
      .mockResolvedValueOnce(STALE)
      .mockResolvedValueOnce(FRESH);
    const { result } = renderHook(() => useRecentActivities(), { wrapper });
    await flush();

    expect(result.current.data).toEqual(STALE);
    expect(result.current.refreshing).toBe(true);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(1);

    await flush(DELAYS[0] - 1);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(1);
    await flush(1);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(2);
    // Still stale: the next ask is owed, after the next delay and not before.
    expect(result.current.data).toEqual(STALE);
    expect(result.current.refreshing).toBe(true);

    await flush(DELAYS[1] - 1);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(2);
    await flush(1);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(3);
    expect(result.current.data).toEqual(FRESH);
    expect(result.current.data?.activities.map((activity) => activity.name)).toEqual(['Lunch run', 'Lake loop']);
    expect(result.current.refreshing).toBe(false);

    // The schedule ended on the fresh answer: its remaining delays ask nothing.
    await flush(WHOLE_SCHEDULE * 2);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(3);
  });

  it('stops after the last delay when every answer is stale, and says it is refreshing until then', async () => {
    api.getRecentActivities.mockResolvedValue(STALE);
    const { result } = renderHook(() => useRecentActivities(), { wrapper });
    await flush();
    expect(api.getRecentActivities).toHaveBeenCalledTimes(1);

    for (const [index, delay] of DELAYS.entries()) {
      expect(result.current.refreshing).toBe(true);
      await flush(delay - 1);
      expect(api.getRecentActivities).toHaveBeenCalledTimes(index + 1);
      await flush(1);
      expect(api.getRecentActivities).toHaveBeenCalledTimes(index + 2);
    }
    expect(result.current.refreshing).toBe(false);
    expect(result.current.data).toEqual(STALE);

    // No interval: however long the page stays open, nothing more is asked.
    await flush(WHOLE_SCHEDULE * 3);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(DELAYS.length + 1);
    expect(result.current.refreshing).toBe(false);
  });

  it('never schedules a follow-up for a fresh answer', async () => {
    api.getRecentActivities.mockResolvedValue(FRESH);
    const { result } = renderHook(() => useRecentActivities(), { wrapper });
    await flush();
    await flush(WHOLE_SCHEDULE * 2);

    expect(result.current.refreshing).toBe(false);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(1);
  });

  it('holds an ask that comes due on an idle or hidden tab, sends it on return, and carries on from there', async () => {
    api.getRecentActivities.mockResolvedValue(STALE);
    const { result } = renderHook(() => useRecentActivities(), { wrapper });
    await flush();

    // The idle watch marks an untouched tab unfocused.
    focusManager.setFocused(false);
    await flush(WHOLE_SCHEDULE * 2);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(1);
    expect(result.current.refreshing).toBe(true);

    // React Query's own focus refetch and the held ask share one request.
    await act(async () => {
      focusManager.setFocused(true);
    });
    await flush();
    expect(api.getRecentActivities).toHaveBeenCalledTimes(2);
    expect(result.current.refreshing).toBe(true);

    // The schedule picks up at its second delay, measured from that answer.
    await flush(DELAYS[1] - 1);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(2);
    await flush(1);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(3);

    // Going away again holds the third ask the same way.
    focusManager.setFocused(false);
    await flush(WHOLE_SCHEDULE * 2);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(3);
    await act(async () => {
      focusManager.setFocused(true);
    });
    await flush();
    expect(api.getRecentActivities).toHaveBeenCalledTimes(4);
    expect(result.current.refreshing).toBe(true);
  });

  it('counts a failed follow-up as an answer: it moves to the next delay instead of asking again at once', async () => {
    api.getRecentActivities
      .mockResolvedValueOnce(STALE)
      .mockRejectedValueOnce(new Error('503'))
      .mockResolvedValueOnce(FRESH);
    const { result } = renderHook(() => useRecentActivities(), { wrapper });
    await flush();

    await flush(DELAYS[0]);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(2);
    expect(result.current.isError).toBe(true);
    // The rows the page already had stay, and the next ask is still owed.
    expect(result.current.data).toEqual(STALE);
    expect(result.current.refreshing).toBe(true);

    await flush(DELAYS[1] - 1);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(2);
    await flush(1);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(3);
    expect(result.current.data).toEqual(FRESH);
    expect(result.current.refreshing).toBe(false);
  });

  it('ends the schedule after the last delay when every follow-up fails', async () => {
    api.getRecentActivities.mockResolvedValueOnce(STALE).mockRejectedValue(new Error('503'));
    const { result } = renderHook(() => useRecentActivities(), { wrapper });
    await flush();

    await runScheduleOut(1);
    expect(result.current.refreshing).toBe(false);
    expect(result.current.data).toEqual(STALE);

    await flush(WHOLE_SCHEDULE * 2);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(DELAYS.length + 1);
  });

  it('starts a schedule for a stale answer that arrives after one ran out', async () => {
    api.getRecentActivities.mockResolvedValue(STALE);
    const { result } = renderHook(() => useRecentActivities(), { wrapper });
    await flush();
    await runScheduleOut(1);
    expect(result.current.refreshing).toBe(false);
    const asked = DELAYS.length + 1;

    // The athlete retries a minute on; the server is still refreshing.
    await flush(60_000);
    await act(async () => {
      result.current.refetch();
    });
    await flush();
    expect(api.getRecentActivities).toHaveBeenCalledTimes(asked + 1);
    expect(result.current.refreshing).toBe(true);

    api.getRecentActivities.mockResolvedValue(FRESH);
    await flush(DELAYS[0] - 1);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(asked + 1);
    await flush(1);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(asked + 2);
    expect(result.current.data).toEqual(FRESH);
    expect(result.current.refreshing).toBe(false);

    await flush(WHOLE_SCHEDULE * 2);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(asked + 2);
  });

  it('starts a schedule for a stale answer that arrives after a fresh one ended the last', async () => {
    api.getRecentActivities.mockResolvedValueOnce(STALE).mockResolvedValueOnce(FRESH);
    const { result } = renderHook(() => useRecentActivities(), { wrapper });
    await flush();
    await flush(DELAYS[0]);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(2);
    expect(result.current.refreshing).toBe(false);

    // The cache aged past the freshness window while the page stayed open.
    api.getRecentActivities.mockResolvedValue(STALE);
    await flush(60_000);
    await act(async () => {
      result.current.refetch();
    });
    await flush();
    expect(api.getRecentActivities).toHaveBeenCalledTimes(3);
    expect(result.current.refreshing).toBe(true);

    await runScheduleOut(3);
    expect(result.current.refreshing).toBe(false);
  });

  it('keeps one timer when a stale answer arrives by another route while a schedule runs', async () => {
    api.getRecentActivities.mockResolvedValue(STALE);
    const { result } = renderHook(() => useRecentActivities(), { wrapper });
    await flush();

    const early = 5000;
    await flush(early);
    await act(async () => {
      result.current.refetch();
    });
    await flush();
    expect(api.getRecentActivities).toHaveBeenCalledTimes(2);
    expect(result.current.refreshing).toBe(true);

    // The first follow-up still goes out when its own delay is up.
    await flush(DELAYS[0] - early - 1);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(2);
    await flush(1);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(3);

    // A second schedule started by the early answer would ask `early` into
    // this wait; the only ask is the one the second delay owes.
    await flush(DELAYS[1] - 1);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(3);
    await flush(1);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(4);
  });

  it('restarts the schedule at the retry, which asks the server to refresh past its pause', async () => {
    api.getRecentActivities.mockResolvedValue(STALE);
    const { result } = renderHook(() => useRecentActivities(), { wrapper });
    await flush();
    // Two asks into the schedule: its next ask is a whole third delay away.
    await flush(DELAYS[0]);
    await flush(DELAYS[1]);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(3);
    const intoWait = 10_000;
    await flush(intoWait);

    await act(async () => {
      result.current.retry();
    });
    await flush();
    expect(api.getRecentActivities).toHaveBeenCalledTimes(4);
    expect(api.getRecentActivities).toHaveBeenLastCalledWith(undefined, { retry: true });
    expect(result.current.refreshing).toBe(true);

    // The refresh the retry started is what the page waits on: the next ask
    // is one first delay after the retry, not what was left of the old wait.
    api.getRecentActivities.mockResolvedValue(FRESH);
    await flush(DELAYS[0] - 1);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(4);
    await flush(1);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(5);
    expect(result.current.data).toEqual(FRESH);
    expect(result.current.refreshing).toBe(false);
  });

  it('starts a schedule from a retry made after the last one ended on a paused failure', async () => {
    const paused: RecentActivitiesResponse = {
      ...STALE,
      stale: false,
      sync_failure: {
        provider: 'strava',
        provider_name: 'Strava',
        failed_at: '2026-09-24T11:58:00Z',
        last_synced_at: '2026-09-23T06:00:00Z',
      },
    };
    api.getRecentActivities.mockResolvedValue(paused);
    const { result } = renderHook(() => useRecentActivities(), { wrapper });
    await flush();
    await flush(WHOLE_SCHEDULE);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(1);

    api.getRecentActivities.mockResolvedValueOnce({ ...paused, stale: true }).mockResolvedValue(FRESH);
    await act(async () => {
      result.current.retry();
    });
    await flush();
    expect(result.current.refreshing).toBe(true);
    await flush(DELAYS[0]);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(3);
    expect(result.current.data).toEqual(FRESH);
  });

  it('cancels the pending ask when the page goes away, before the first and in the middle of a schedule', async () => {
    api.getRecentActivities.mockResolvedValue(STALE);
    const first = renderHook(() => useRecentActivities(), { wrapper });
    await flush();
    first.unmount();
    await flush(WHOLE_SCHEDULE * 2);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(1);

    const second = renderHook(() => useRecentActivities(), { wrapper });
    await flush();
    await flush(DELAYS[0]);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(3);
    second.unmount();
    await flush(WHOLE_SCHEDULE * 2);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(3);
  });
});

describe('useActivityRoute', () => {
  const UNAVAILABLE: ActivityRouteResponse = { route: null, reason: 'unavailable' };
  const RECHECKS = HOME_ROUTE_UNAVAILABLE_RECHECK_DELAYS_MS;

  beforeEach(() => {
    // React Query follows an answer up on an interval timer.
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'Date'] });
    vi.setSystemTime(new Date('2026-09-24T12:00:00Z'));
  });

  it('asks an unavailable route again after each recheck delay, then stops', async () => {
    api.getActivityRoute.mockResolvedValue(UNAVAILABLE);
    const { result } = renderHook(() => useActivityRoute('strava', 'act-9', true), { wrapper });
    await flush();
    expect(api.getActivityRoute).toHaveBeenCalledTimes(1);

    for (const [index, delay] of RECHECKS.entries()) {
      await flush(delay - 1);
      expect(api.getActivityRoute).toHaveBeenCalledTimes(index + 1);
      await flush(1);
      expect(api.getActivityRoute).toHaveBeenCalledTimes(index + 2);
    }
    // A schedule with an end: an open page asks nothing more.
    await flush(RECHECKS.reduce((sum, delay) => sum + delay, 0) * 4);
    expect(api.getActivityRoute).toHaveBeenCalledTimes(RECHECKS.length + 1);
    expect(result.current.data).toEqual(UNAVAILABLE);
  });

  it('draws the route the server stored after its first answer, and asks nothing more', async () => {
    const drawn: ActivityRouteResponse = {
      route: {
        coordinates: [
          [45.5, -73.6],
          [45.51, -73.61],
        ],
        bounds: { min_latitude: 45.5, max_latitude: 45.51, min_longitude: -73.61, max_longitude: -73.6 },
        elevation_meters: null,
        distances_meters: null,
        climbs: [],
        title: 'Sortie',
        source_tool: 'strava',
      },
      reason: null,
    };
    api.getActivityRoute.mockResolvedValueOnce(UNAVAILABLE).mockResolvedValue(drawn);
    const { result } = renderHook(() => useActivityRoute('strava', 'act-9', true), { wrapper });
    await flush();
    await flush(RECHECKS[0]);
    expect(result.current.data).toEqual(drawn);
    await flush(RECHECKS[1] * 4);
    expect(api.getActivityRoute).toHaveBeenCalledTimes(2);
  });

  // 2026-09-30: a first Home visit's five route reads queued behind one
  // another on the athlete's provider turn, and the reads the server's bound
  // cut off said "could not be loaded". The server now answers those
  // `pending`; the map keeps loading and asks again until the read lands.
  it('keeps loading through pending answers and draws the route the queued read lands', async () => {
    const drawn: ActivityRouteResponse = {
      route: {
        coordinates: [
          [45.5, -73.6],
          [45.51, -73.61],
        ],
        bounds: { min_latitude: 45.5, max_latitude: 45.51, min_longitude: -73.61, max_longitude: -73.6 },
        elevation_meters: null,
        distances_meters: null,
        climbs: [],
        title: 'Sortie',
        source_tool: 'strava',
      },
      reason: null,
    };
    const answers: Array<(response: ActivityRouteAnswer) => void> = [];
    api.getActivityRoute.mockImplementation(
      () =>
        new Promise((resolve) => {
          answers.push(resolve);
        }),
    );
    const { result } = renderHook(() => useActivityRoute('strava', 'act-9', true), { wrapper });
    await flush();
    for (let ask = 0; ask < 3; ask += 1) {
      expect(answers).toHaveLength(ask + 1);
      await act(async () => {
        answers[ask]({ route: null, reason: 'pending', settles_within_secs: 660 });
      });
      // The follow-up waits out its backoff.
      await flush(HOME_ROUTE_PENDING_BACKOFF_MS[ask] ?? 0);
      expect(result.current.data).toBeUndefined();
      expect(result.current.isFetching).toBe(true);
      expect(result.current.isError).toBe(false);
    }
    await act(async () => {
      answers[3](drawn);
    });
    await flush();
    expect(result.current.data).toEqual(drawn);
    expect(result.current.isFetching).toBe(false);
    expect(api.getActivityRoute).toHaveBeenCalledTimes(4);
  });

  // The client used to give up after fourteen `pending` asks (~350 s) and
  // say "could not be loaded" while a read queued behind several slow ones
  // was still coming. It now follows the read for as long as each `pending`
  // says it may take.
  it('keeps loading past any fixed count of pending asks while each answer names a bound', async () => {
    const drawn: ActivityRouteResponse = {
      route: {
        coordinates: [
          [45.5, -73.6],
          [45.51, -73.61],
        ],
        bounds: { min_latitude: 45.5, max_latitude: 45.51, min_longitude: -73.61, max_longitude: -73.6 },
        elevation_meters: null,
        distances_meters: null,
        climbs: [],
        title: 'Sortie',
        source_tool: 'strava',
      },
      reason: null,
    };
    const PENDING_ASKS = 40;
    let asked = 0;
    api.getActivityRoute.mockImplementation(
      () =>
        new Promise((resolve) => {
          asked += 1;
          // The server holds each queued ask up to its 25-second bound.
          setTimeout(
            () => resolve(asked > PENDING_ASKS ? drawn : { route: null, reason: 'pending', settles_within_secs: 1650 }),
            25_000,
          );
        }),
    );
    const { result } = renderHook(() => useActivityRoute('strava', 'act-9', true), { wrapper });
    while (asked <= PENDING_ASKS) {
      await flush(1_000);
      expect(result.current.isError).toBe(false);
      expect(result.current.data).toBeUndefined();
      expect(result.current.isFetching).toBe(true);
    }
    await flush(25_000);
    expect(result.current.data).toEqual(drawn);
    expect(api.getActivityRoute).toHaveBeenCalledTimes(PENDING_ASKS + 1);
  });

  it('says the map could not be loaded once a pending overruns the bound the server named', async () => {
    let asked = 0;
    api.getActivityRoute.mockImplementation(
      () =>
        new Promise((resolve) => {
          asked += 1;
          // The second ask comes back long after the first named 10 s.
          setTimeout(
            () => resolve({ route: null, reason: 'pending', settles_within_secs: 10 }),
            asked === 1 ? 0 : 60_000,
          );
        }),
    );
    const { result } = renderHook(() => useActivityRoute('strava', 'act-9', true), { wrapper });
    await flush(0);
    expect(result.current.data).toBeUndefined();
    await flush(60_000);
    expect(result.current.data).toEqual(UNAVAILABLE);
    expect(api.getActivityRoute).toHaveBeenCalledTimes(2);
  });

  it('marks a Home read as one of its burst, and never the retry', async () => {
    api.getActivityRoute.mockResolvedValue(UNAVAILABLE);
    const { result } = renderHook(() => useActivityRoute('strava', 'act-9', true, { burst: true }), { wrapper });
    await flush();
    expect(api.getActivityRoute).toHaveBeenLastCalledWith('strava', 'act-9', expect.objectContaining({ burst: true }));
    await act(async () => {
      result.current.retry();
    });
    await flush();
    expect(api.getActivityRoute).toHaveBeenLastCalledWith(
      'strava',
      'act-9',
      expect.not.objectContaining({ burst: true }),
    );
  });

  it('reads an activity view map on its own, outside any burst', async () => {
    api.getActivityRoute.mockResolvedValue(UNAVAILABLE);
    renderHook(() => useActivityRoute('strava', 'act-9', true), { wrapper });
    await flush();
    expect(api.getActivityRoute).toHaveBeenCalledWith('strava', 'act-9', expect.objectContaining({ burst: false }));
  });

  // 2026-09-29: a failing scraper had the server store `no_gps` for rides
  // that recorded GPS, and the list said `has_gps: false` with it. The server
  // has since deleted those rows, so the list says `has_gps: true` again —
  // and a tab left open since must ask the route again, not keep the old
  // answer as settled.
  it('asks a held no_gps route again once the list says the activity has GPS again', async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const shared = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    );
    const drawn: ActivityRouteResponse = {
      route: {
        coordinates: [
          [45.5, -73.6],
          [45.51, -73.61],
        ],
        bounds: { min_latitude: 45.5, max_latitude: 45.51, min_longitude: -73.61, max_longitude: -73.6 },
        elevation_meters: null,
        distances_meters: null,
        climbs: [],
        title: 'Sortie',
        source_tool: 'strava',
      },
      reason: null,
    };
    api.getActivityRoute.mockResolvedValueOnce({ route: null, reason: 'no_gps' }).mockResolvedValue(drawn);
    const { result, rerender } = renderHook(
      ({ hasGps }: { hasGps: boolean }) => useActivityRoute('strava', 'act-9', hasGps),
      { wrapper: shared, initialProps: { hasGps: true } },
    );
    await flush();
    expect(result.current.data).toEqual({ route: null, reason: 'no_gps' });

    rerender({ hasGps: false });
    await flush();
    rerender({ hasGps: true });
    await flush();
    expect(api.getActivityRoute).toHaveBeenCalledTimes(2);
    expect(result.current.data).toEqual(drawn);
  });

  it('retries past a stored unavailable answer, and says a read is in flight meanwhile', async () => {
    api.getActivityRoute.mockResolvedValueOnce(UNAVAILABLE);
    const { result } = renderHook(() => useActivityRoute('strava', 'act-9', true), { wrapper });
    await flush();
    let answer: (response: ActivityRouteAnswer) => void = () => undefined;
    api.getActivityRoute.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          answer = resolve;
        }),
    );

    await act(async () => {
      result.current.retry();
    });
    expect(api.getActivityRoute).toHaveBeenLastCalledWith('strava', 'act-9', expect.objectContaining({ retry: true }));
    expect(result.current.isFetching).toBe(true);
    await act(async () => {
      answer(UNAVAILABLE);
    });
    await flush();
    expect(result.current.isFetching).toBe(false);
  });
});

describe('useTrainingPlan', () => {
  it("asks for the plan in the language the app renders", async () => {
    api.getTrainingPlan.mockResolvedValue({ plan: null, today: '2026-09-24' });
    const { result } = renderHook(() => useTrainingPlan(), { wrapper });
    await flush();

    expect(api.getTrainingPlan).toHaveBeenCalledExactlyOnceWith('en');
    expect(result.current.data).toEqual({ plan: null, today: '2026-09-24' });
  });
});

describe('useProviderConnection', () => {
  function status(
    provider: string,
    display_name: string,
    connected: boolean,
    needs_reauth: boolean,
  ): ProvidersStatusResponse['providers'][number] {
    return {
      provider,
      display_name,
      requires_oauth: true,
      connected,
      needs_reauth,
      capabilities: ['activities'],
      consent_required: false,
    };
  }

  it('names the connected providers that have to be reconnected, and only those', async () => {
    api.getProvidersStatus.mockResolvedValue({
      providers: [
        status('strava', 'Strava', true, false),
        status('garmin', 'Garmin', true, true),
        // The flag means nothing on a provider that is not connected.
        status('whoop', 'WHOOP', false, true),
        status('coros', 'COROS', true, true),
      ],
    });
    const { result } = renderHook(() => useProviderConnection(), { wrapper });
    expect(result.current).toEqual({ loaded: false, connected: false, needsReconnect: [] });
    await flush();

    expect(result.current).toEqual({ loaded: true, connected: true, needsReconnect: ['Garmin', 'COROS'] });
  });

  it('names a provider once when its mirror and native cards both need reconnecting', async () => {
    api.getProvidersStatus.mockResolvedValue({
      providers: [status('strava', 'Strava', true, true), status('sciotte', 'Strava', true, true)],
    });
    const { result } = renderHook(() => useProviderConnection(), { wrapper });
    await flush();

    expect(result.current.needsReconnect).toEqual(['Strava']);
  });

  it('names nobody when every connection is healthy', async () => {
    api.getProvidersStatus.mockResolvedValue({ providers: [status('strava', 'Strava', true, false)] });
    const { result } = renderHook(() => useProviderConnection(), { wrapper });
    await flush();

    expect(result.current).toEqual({ loaded: true, connected: true, needsReconnect: [] });
  });
});
