// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The follow-up reads a stale Home activity list earns — on the HOME_STALE_REFETCH_DELAYS_MS schedule, only while the app is in use
// ABOUTME: A fresh answer ends the schedule, the last delay ends it whatever the answer, and a stale answer after that starts another

import React from 'react';
import { act, renderHook } from '@testing-library/react-native';
import {
  QueryClient,
  QueryClientProvider,
  defaultScheduler,
  focusManager,
  notifyManager,
} from '@tanstack/react-query';
import { HOME_STALE_REFETCH_DELAYS_MS, QUERY_KEYS } from '@pierre/shared-constants';
import type { RecentActivitiesResponse } from '@pierre/shared-types';

import { ACTIVITIES, recentResponse } from '../integration/app/helpers/homeFixtures';

const mockGetRecentActivities = jest.fn<Promise<RecentActivitiesResponse>, []>();
jest.mock('../src/services/api', () => ({
  athleteApi: { getRecentActivities: () => mockGetRecentActivities() },
}));

import { useRecentActivities } from '../src/hooks/useHome';

const [FIRST_DELAY, SECOND_DELAY] = HOME_STALE_REFETCH_DELAYS_MS;
/** The whole schedule, first answer to last follow-up. */
const SCHEDULE_MS = HOME_STALE_REFETCH_DELAYS_MS.reduce((sum, delay) => sum + delay, 0);

function newClient() {
  // The app's client does not refetch on focus (QueryProvider), so a read
  // counted here is the hook's own and never React Query's focus refetch.
  return new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0, refetchOnWindowFocus: false } },
  });
}

function renderRecent(client: QueryClient = newClient()) {
  return renderHook(() => useRecentActivities(), {
    wrapper: ({ children }: { children: React.ReactNode }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    ),
  });
}

/** Let the clock run, and whatever falls due in that time settle: the read, its answer, the render after it. */
async function elapse(ms: number) {
  await act(async () => {
    await jest.advanceTimersByTimeAsync(ms);
  });
}

/** Let what is already due settle, without moving the clock. */
const settle = () => elapse(0);

/** Run a whole schedule of stale answers out, from its first answer to its last follow-up's. */
async function runScheduleOut() {
  for (const delay of HOME_STALE_REFETCH_DELAYS_MS) {
    await elapse(delay);
  }
}

/** A read that answers when the test says so. */
function heldAnswer() {
  let answer: (response: RecentActivitiesResponse) => void = () => undefined;
  const promise = new Promise<RecentActivitiesResponse>((resolve) => {
    answer = resolve;
  });
  return { promise, answer };
}

beforeEach(() => {
  // The clock is faked with the timers: an answer is told from the one before
  // it by when it arrived.
  jest.useFakeTimers();
  // React Query batches its notifications on a zero-delay timeout, which a
  // fake clock runs a millisecond late when it is set from inside another
  // timer. Run them inline, so the clock only drives the hook's own delays
  // and each one is measured to the millisecond.
  notifyManager.setScheduler((callback) => callback());
  mockGetRecentActivities.mockReset();
  focusManager.setFocused(undefined);
});

afterEach(() => {
  focusManager.setFocused(undefined);
  notifyManager.setScheduler(defaultScheduler);
  jest.useRealTimers();
});

describe('the stale follow-up schedule', () => {
  it('has follow-ups to make', () => {
    // Every case below walks the shared schedule; an empty one would pass them all.
    expect(HOME_STALE_REFETCH_DELAYS_MS.length).toBeGreaterThan(1);
  });

  it('asks again after the first delay, and not a moment before', async () => {
    mockGetRecentActivities
      .mockResolvedValueOnce(recentResponse({ stale: true, as_of: '2026-09-23T06:00:00Z' }))
      .mockResolvedValueOnce(recentResponse({ stale: false, as_of: '2026-09-24T09:00:00Z' }));
    const { result } = renderRecent();

    await settle();
    expect(result.current.staleRefetch).toBe('pending');
    expect(result.current.stale).toBe(true);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(1);

    await elapse(FIRST_DELAY - 1);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(1);

    await elapse(1);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(2);
    expect(result.current.staleRefetch).toBe('done');
    expect(result.current.stale).toBe(false);
    expect(result.current.asOf).toBe('2026-09-24T09:00:00Z');
  });

  it('stops at the first answer that is not stale, and shows its rows', async () => {
    const newest = { ...ACTIVITIES[0], id: '9002', name: 'Recovery spin', start_date: '2026-09-23T15:00:00Z' };
    const caughtUp = [newest, ...ACTIVITIES.slice(0, 4)];
    mockGetRecentActivities
      .mockResolvedValueOnce(recentResponse({ stale: true }))
      .mockResolvedValueOnce(recentResponse({ stale: true }))
      .mockResolvedValueOnce(recentResponse({ activities: caughtUp, stale: false }))
      .mockResolvedValue(recentResponse({ stale: true }));
    const { result } = renderRecent();
    await settle();

    await elapse(FIRST_DELAY);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(2);
    // Still stale: a follow-up is still owed, and the old rows stay.
    expect(result.current.staleRefetch).toBe('pending');
    expect(result.current.activities.map((activity) => activity.id)).toEqual(ACTIVITIES.map((a) => a.id));

    await elapse(SECOND_DELAY - 1);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(2);
    await elapse(1);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(3);
    expect(result.current.staleRefetch).toBe('done');
    expect(result.current.stale).toBe(false);
    expect(result.current.activities.map((activity) => activity.id)).toEqual(caughtUp.map((a) => a.id));

    // The rest of the schedule is not walked.
    await elapse(SCHEDULE_MS * 2);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(3);
  });

  it('waits each delay from the answer before it, and stops after the last while every answer is stale', async () => {
    mockGetRecentActivities.mockResolvedValue(recentResponse({ stale: true }));
    const { result } = renderRecent();
    await settle();

    let reads = 1;
    for (const delay of HOME_STALE_REFETCH_DELAYS_MS) {
      expect(result.current.staleRefetch).toBe('pending');
      await elapse(delay - 1);
      expect(mockGetRecentActivities).toHaveBeenCalledTimes(reads);
      await elapse(1);
      reads += 1;
      expect(mockGetRecentActivities).toHaveBeenCalledTimes(reads);
    }

    expect(reads).toBe(1 + HOME_STALE_REFETCH_DELAYS_MS.length);
    // Still stale, and nothing more is owed: the list shows when it last synced.
    expect(result.current.stale).toBe(true);
    expect(result.current.staleRefetch).toBe('done');

    await elapse(SCHEDULE_MS * 2);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(reads);
  });

  it('schedules nothing for a fresh answer', async () => {
    mockGetRecentActivities.mockResolvedValue(recentResponse({ stale: false }));
    const { result } = renderRecent();

    await settle();
    expect(result.current.hasData).toBe(true);
    await elapse(SCHEDULE_MS * 2);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(1);
    expect(result.current.staleRefetch).toBe('none');
  });

  it('cancels the schedule when the screen goes away', async () => {
    mockGetRecentActivities.mockResolvedValue(recentResponse({ stale: true }));
    const { result, unmount } = renderRecent();

    await settle();
    await elapse(FIRST_DELAY);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(2);
    expect(result.current.staleRefetch).toBe('pending');

    unmount();
    await elapse(SCHEDULE_MS * 2);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(2);
  });

  it('counts a follow-up that fails as an answer, and moves on to the next delay', async () => {
    mockGetRecentActivities
      .mockResolvedValueOnce(recentResponse({ stale: true }))
      .mockRejectedValueOnce(new Error('503'))
      .mockResolvedValue(recentResponse({ stale: true }));
    const { result } = renderRecent();
    await settle();

    await elapse(FIRST_DELAY);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(2);
    // The stale rows it followed are still the list, and a follow-up is still owed.
    expect(result.current.isError).toBe(true);
    expect(result.current.hasData).toBe(true);
    expect(result.current.stale).toBe(true);
    expect(result.current.staleRefetch).toBe('pending');

    // Not asked again at once: the failure took the first delay's place.
    await elapse(SECOND_DELAY - 1);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(2);
    await elapse(1);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(3);

    await elapse(SCHEDULE_MS * 2);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(1 + HOME_STALE_REFETCH_DELAYS_MS.length);
    expect(result.current.staleRefetch).toBe('done');
  });

  it('ends on a failure when the last follow-up is the one that fails', async () => {
    mockGetRecentActivities.mockResolvedValueOnce(recentResponse({ stale: true }));
    for (let followUp = 1; followUp < HOME_STALE_REFETCH_DELAYS_MS.length; followUp += 1) {
      mockGetRecentActivities.mockResolvedValueOnce(recentResponse({ stale: true }));
    }
    mockGetRecentActivities.mockRejectedValue(new Error('503'));
    const { result } = renderRecent();
    await settle();

    await runScheduleOut();
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(1 + HOME_STALE_REFETCH_DELAYS_MS.length);
    expect(result.current.staleRefetch).toBe('done');

    await elapse(SCHEDULE_MS * 2);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(1 + HOME_STALE_REFETCH_DELAYS_MS.length);
  });
});

describe('the schedule while the app is out of use', () => {
  it('holds an ask that falls due in the background until the athlete is back, then carries on', async () => {
    mockGetRecentActivities.mockResolvedValue(recentResponse({ stale: true }));
    const { result } = renderRecent();
    await settle();

    // The idle watch expresses backgrounded and idle as "not focused".
    act(() => focusManager.setFocused(false));
    await elapse(SCHEDULE_MS * 2);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(1);
    expect(result.current.staleRefetch).toBe('pending');

    // Back: the ask that was due happens, once.
    act(() => focusManager.setFocused(true));
    await settle();
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(2);
    expect(result.current.staleRefetch).toBe('pending');

    // The schedule carries on from that answer, at its second delay.
    await elapse(SECOND_DELAY - 1);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(2);
    await elapse(1);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(3);
  });

  it('pauses again each time the app leaves, and never asks twice for one return', async () => {
    mockGetRecentActivities.mockResolvedValue(recentResponse({ stale: true }));
    renderRecent();
    await settle();
    await elapse(FIRST_DELAY);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(2);

    act(() => focusManager.setFocused(false));
    await elapse(SECOND_DELAY);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(2);

    act(() => focusManager.setFocused(true));
    await settle();
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(3);
    // Leaving and coming back again is not a second ask for the same delay.
    act(() => focusManager.setFocused(false));
    act(() => focusManager.setFocused(true));
    await settle();
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(3);
  });

  it('drops the wait for focus when the screen goes away', async () => {
    mockGetRecentActivities.mockResolvedValue(recentResponse({ stale: true }));
    const { unmount } = renderRecent();
    await settle();

    act(() => focusManager.setFocused(false));
    await elapse(FIRST_DELAY);
    unmount();
    act(() => focusManager.setFocused(true));
    await elapse(SCHEDULE_MS);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(1);
  });
});

describe('an answer from outside the schedule', () => {
  it('starts a new schedule when it is stale and the last one has ended', async () => {
    mockGetRecentActivities.mockResolvedValue(recentResponse({ stale: true }));
    const { result } = renderRecent();
    await settle();
    await runScheduleOut();
    const afterFirstSchedule = 1 + HOME_STALE_REFETCH_DELAYS_MS.length;
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(afterFirstSchedule);
    expect(result.current.staleRefetch).toBe('done');

    // Minutes later the athlete pulls to refresh, or comes back to the tab.
    await elapse(SCHEDULE_MS);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(afterFirstSchedule);
    await act(async () => {
      void result.current.refetch();
    });
    await settle();
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(afterFirstSchedule + 1);
    expect(result.current.staleRefetch).toBe('pending');

    // A whole schedule of its own, and then it stops too.
    await elapse(FIRST_DELAY - 1);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(afterFirstSchedule + 1);
    await elapse(1);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(afterFirstSchedule + 2);
    await elapse(SCHEDULE_MS * 2);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(2 * afterFirstSchedule);
    expect(result.current.staleRefetch).toBe('done');
  });

  it('starts a schedule when it is stale and the one before it ended on a fresh answer', async () => {
    mockGetRecentActivities
      .mockResolvedValueOnce(recentResponse({ stale: true }))
      .mockResolvedValueOnce(recentResponse({ stale: false }))
      .mockResolvedValue(recentResponse({ stale: true }));
    const { result } = renderRecent();
    await settle();
    await elapse(FIRST_DELAY);
    expect(result.current.staleRefetch).toBe('done');
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(2);

    await elapse(SCHEDULE_MS);
    await act(async () => {
      void result.current.refetch();
    });
    await settle();
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(3);
    expect(result.current.staleRefetch).toBe('pending');

    await elapse(FIRST_DELAY);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(4);
  });

  it('leaves a running schedule on its one timer when it is stale', async () => {
    mockGetRecentActivities.mockResolvedValue(recentResponse({ stale: true }));
    const { result } = renderRecent();
    await settle();

    // A pull to refresh a third of the way through the first delay.
    const early = Math.floor(FIRST_DELAY / 3);
    await elapse(early);
    await act(async () => {
      void result.current.refetch();
    });
    await settle();
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(2);
    expect(result.current.staleRefetch).toBe('pending');

    // The follow-up falls due when it always would have, and only once.
    await elapse(FIRST_DELAY - early - 1);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(2);
    await elapse(1);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(3);

    // No second timer was set by the pull: the next ask is the schedule's own.
    await elapse(SECOND_DELAY - 1);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(3);
    await elapse(1);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(4);

    await elapse(SCHEDULE_MS * 2);
    // The mount read, the pull, and one follow-up per delay.
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(2 + HOME_STALE_REFETCH_DELAYS_MS.length);
  });

  it('ends a running schedule when it is fresh', async () => {
    mockGetRecentActivities
      .mockResolvedValueOnce(recentResponse({ stale: true }))
      .mockResolvedValue(recentResponse({ stale: false, as_of: '2026-09-24T09:00:00Z' }));
    const { result } = renderRecent();
    await settle();
    expect(result.current.staleRefetch).toBe('pending');

    await elapse(Math.floor(FIRST_DELAY / 3));
    await act(async () => {
      void result.current.refetch();
    });
    await settle();
    expect(result.current.stale).toBe(false);
    expect(result.current.staleRefetch).toBe('done');

    await elapse(SCHEDULE_MS * 2);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(2);
  });

  it('starts nothing when it is a failure', async () => {
    mockGetRecentActivities.mockResolvedValue(recentResponse({ stale: true }));
    const { result } = renderRecent();
    await settle();
    await runScheduleOut();
    const afterSchedule = 1 + HOME_STALE_REFETCH_DELAYS_MS.length;
    expect(result.current.staleRefetch).toBe('done');

    mockGetRecentActivities.mockRejectedValue(new Error('503'));
    await elapse(SCHEDULE_MS);
    await act(async () => {
      void result.current.refetch();
    });
    await settle();
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(afterSchedule + 1);
    expect(result.current.isError).toBe(true);
    expect(result.current.staleRefetch).toBe('done');

    await elapse(SCHEDULE_MS * 2);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(afterSchedule + 1);
  });
});

describe('a follow-up that falls due during another read', () => {
  it('joins the read in flight rather than cancelling it', async () => {
    const held = heldAnswer();
    mockGetRecentActivities
      .mockResolvedValueOnce(recentResponse({ stale: true }))
      .mockReturnValueOnce(held.promise)
      .mockResolvedValue(recentResponse({ stale: true }));
    const { result } = renderRecent();
    await settle();

    // A pull to refresh that is still on the wire when the first delay runs out.
    await elapse(FIRST_DELAY - 1);
    await act(async () => {
      void result.current.refetch();
    });
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(2);
    await elapse(1);
    // Cancelling the pull would have put a third read on the wire.
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(2);

    // Its answer counts as the ask's own: the schedule moves to its second delay.
    await act(async () => {
      held.answer(recentResponse({ stale: true }));
    });
    await settle();
    expect(result.current.staleRefetch).toBe('pending');
    await elapse(SECOND_DELAY - 1);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(2);
    await elapse(1);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(3);
  });
});

describe('an answer the cache already holds', () => {
  const RESTORED_AT = Date.parse('2026-09-24T08:00:00Z');

  beforeEach(() => {
    jest.setSystemTime(RESTORED_AT + 60 * 60 * 1000);
  });

  it('is followed up when it is stale, on one schedule with the read that follows it', async () => {
    const client = newClient();
    // What the persister restores: an answer from an hour ago that said stale.
    client.setQueryData(QUERY_KEYS.home.recentActivities(), recentResponse({ stale: true }), {
      updatedAt: RESTORED_AT,
    });
    mockGetRecentActivities.mockResolvedValue(recentResponse({ stale: true }));
    const { result } = renderRecent(client);

    await settle();
    // The mount read, since the restored answer is old.
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(1);
    expect(result.current.staleRefetch).toBe('pending');

    // One timer between the restored answer and the read after it.
    await elapse(FIRST_DELAY);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(2);
    await elapse(SCHEDULE_MS * 2);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(1 + HOME_STALE_REFETCH_DELAYS_MS.length);
    expect(result.current.staleRefetch).toBe('done');
  });

  it('starts a schedule when it was fresh and the read after it is stale', async () => {
    const client = newClient();
    client.setQueryData(QUERY_KEYS.home.recentActivities(), recentResponse({ stale: false }), {
      updatedAt: RESTORED_AT,
    });
    mockGetRecentActivities.mockResolvedValue(recentResponse({ stale: true }));
    const { result } = renderRecent(client);

    await settle();
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(1);
    expect(result.current.stale).toBe(true);
    expect(result.current.staleRefetch).toBe('pending');

    await elapse(FIRST_DELAY);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(2);
  });
});
