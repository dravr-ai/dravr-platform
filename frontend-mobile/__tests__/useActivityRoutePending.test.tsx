// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home route query follows the server's `pending` answers — a read queued behind others keeps loading, then draws
// ABOUTME: Red if a queued read surfaces as "pending" data or as a map that could not be loaded, if the retry loses its flag, or the burst flag leaks

import React from 'react';
import { act, renderHook, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { HOME_ROUTE_PENDING_BACKOFF_MS } from '@pierre/shared-constants';
import type { ActivityRouteAnswer } from '@pierre/shared-types';

import { TRAIL_ROUTE_RESPONSE } from '../integration/app/helpers/homeFixtures';

type RouteOptions = { retry?: boolean; burst?: boolean; signal?: AbortSignal };

const mockGetActivityRoute = jest.fn<Promise<ActivityRouteAnswer>, [string, string, RouteOptions | undefined]>();
jest.mock('../src/services/api', () => ({
  athleteApi: {
    getActivityRoute: (provider: string, id: string, options?: RouteOptions) =>
      mockGetActivityRoute(provider, id, options),
  },
}));

import { useActivityRoute } from '../src/hooks/useHome';

const PENDING: ActivityRouteAnswer = { route: null, reason: 'pending', settles_within_secs: 660 };

function renderRoute(burst = false) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  return renderHook(() => useActivityRoute('strava', 'morning-trail-run', true, { burst }), { wrapper });
}

/** An api whose every ask waits until the test answers it. */
function heldAnswers() {
  const answers: Array<(answer: ActivityRouteAnswer) => void> = [];
  mockGetActivityRoute.mockImplementation(
    () =>
      new Promise((resolve) => {
        answers.push(resolve);
      }),
  );
  return answers;
}

beforeEach(() => {
  mockGetActivityRoute.mockReset();
  jest.useFakeTimers();
  jest.setSystemTime(new Date('2026-09-30T12:00:00Z'));
});

afterEach(() => {
  jest.useRealTimers();
});

/** Advance the fake clock by `ms`, letting every promise it releases run. */
async function advance(ms: number) {
  await act(async () => {
    await jest.advanceTimersByTimeAsync(ms);
  });
}

describe('useActivityRoute and pending answers', () => {
  // 2026-09-30: a first visit's queued route reads answered "unavailable"
  // and the screen said three of five maps could not be loaded.
  it('keeps loading through pending answers and draws the route the queued read lands', async () => {
    const answers = heldAnswers();
    const { result } = renderRoute();
    await act(async () => {});

    for (let ask = 0; ask < 3; ask += 1) {
      expect(answers).toHaveLength(ask + 1);
      await act(async () => {
        answers[ask](PENDING);
      });
      // The follow-up waits out its backoff.
      await advance(HOME_ROUTE_PENDING_BACKOFF_MS[ask] ?? 0);
      expect(result.current.route).toBeNull();
      expect(result.current.reason).toBeNull();
      expect(result.current.isError).toBe(false);
      expect(result.current.isFetching).toBe(true);
    }

    await act(async () => {
      answers[3](TRAIL_ROUTE_RESPONSE);
    });
    await waitFor(() => expect(result.current.route).toEqual(TRAIL_ROUTE_RESPONSE.route));
    expect(result.current.reason).toBeNull();
    expect(result.current.isFetching).toBe(false);
    expect(mockGetActivityRoute).toHaveBeenCalledTimes(4);
  });

  // The query used to give up after fourteen `pending` asks (~350 s) and the
  // screen said the map could not be loaded while a read queued behind
  // several slow ones was still coming.
  it('keeps loading past any fixed count of pending asks while each answer names a bound', async () => {
    const PENDING_ASKS = 40;
    let asked = 0;
    mockGetActivityRoute.mockImplementation(
      () =>
        new Promise((resolve) => {
          asked += 1;
          // The server holds each queued ask up to its 25-second bound.
          setTimeout(() => resolve(asked > PENDING_ASKS ? TRAIL_ROUTE_RESPONSE : PENDING), 25_000);
        }),
    );
    const { result } = renderRoute(true);
    while (asked <= PENDING_ASKS) {
      await advance(1_000);
      expect(result.current.route).toBeNull();
      expect(result.current.reason).toBeNull();
      expect(result.current.isError).toBe(false);
      expect(result.current.isFetching).toBe(true);
    }
    await advance(25_000);
    await advance(100);
    expect(result.current.route).toEqual(TRAIL_ROUTE_RESPONSE.route);
    expect(mockGetActivityRoute).toHaveBeenCalledTimes(PENDING_ASKS + 1);
  });

  it('says the map could not be loaded once a pending overruns the bound the server named', async () => {
    let asked = 0;
    mockGetActivityRoute.mockImplementation(
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
    const { result } = renderRoute();
    await advance(0);
    expect(result.current.reason).toBeNull();
    await advance(60_000);
    await advance(100);
    expect(result.current.reason).toBe('unavailable');
    expect(mockGetActivityRoute).toHaveBeenCalledTimes(2);
  });

  it('marks a Home read as one of its burst, an activity screen read not, and never the retry', async () => {
    mockGetActivityRoute.mockResolvedValue({ route: null, reason: 'unavailable' });
    const home = renderRoute(true);
    await advance(0);
    expect(mockGetActivityRoute).toHaveBeenLastCalledWith(
      'strava',
      'morning-trail-run',
      expect.objectContaining({ burst: true }),
    );
    await act(async () => {
      home.result.current.retry();
    });
    await advance(0);
    expect(mockGetActivityRoute.mock.lastCall?.[2]).toEqual(expect.objectContaining({ retry: true }));
    expect(mockGetActivityRoute.mock.lastCall?.[2]?.burst).toBeUndefined();
    home.unmount();

    mockGetActivityRoute.mockClear();
    renderRoute(false);
    await advance(0);
    expect(mockGetActivityRoute).toHaveBeenLastCalledWith(
      'strava',
      'morning-trail-run',
      expect.objectContaining({ burst: false }),
    );
  });

  it('asks every follow-up of a retry as the retry', async () => {
    mockGetActivityRoute.mockResolvedValueOnce({ route: null, reason: 'unavailable' });
    const { result } = renderRoute();
    await waitFor(() => expect(result.current.reason).toBe('unavailable'));

    mockGetActivityRoute.mockResolvedValueOnce(PENDING).mockResolvedValueOnce(TRAIL_ROUTE_RESPONSE);
    await act(async () => {
      result.current.retry();
    });
    await waitFor(() => expect(result.current.route).toEqual(TRAIL_ROUTE_RESPONSE.route));
    expect(mockGetActivityRoute).toHaveBeenCalledTimes(3);
    for (const call of mockGetActivityRoute.mock.calls.slice(1)) {
      expect(call[2]).toEqual(expect.objectContaining({ retry: true }));
    }
  });
});
