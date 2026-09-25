// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The one follow-up read a stale Home activity list earns — after the delay, once, only while the app is in use
// ABOUTME: A fresh answer schedules nothing, unmounting cancels the timer, and a second stale answer is never polled

import React from 'react';
import { act, renderHook, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider, focusManager } from '@tanstack/react-query';
import { HOME_STALE_REFETCH_DELAY_MS } from '@pierre/shared-constants';
import type { RecentActivitiesResponse } from '@pierre/shared-types';

import { recentResponse } from '../integration/app/helpers/homeFixtures';

const mockGetRecentActivities = jest.fn<Promise<RecentActivitiesResponse>, []>();
jest.mock('../src/services/api', () => ({
  athleteApi: { getRecentActivities: () => mockGetRecentActivities() },
}));

import { useRecentActivities } from '../src/hooks/useHome';

function renderRecent() {
  // The app's client does not refetch on focus (QueryProvider), so a read
  // counted here is the hook's own and never React Query's focus refetch.
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0, refetchOnWindowFocus: false } },
  });
  return renderHook(() => useRecentActivities(), {
    wrapper: ({ children }: { children: React.ReactNode }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    ),
  });
}

/** Let the timer run out, and whatever it starts settle. */
async function elapse(ms: number) {
  await act(async () => {
    jest.advanceTimersByTime(ms);
  });
}

beforeEach(() => {
  jest.useFakeTimers();
  mockGetRecentActivities.mockReset();
  focusManager.setFocused(undefined);
});

afterEach(() => {
  focusManager.setFocused(undefined);
  jest.useRealTimers();
});

describe('the stale follow-up read', () => {
  it('asks once more after the delay, then shows the second answer', async () => {
    mockGetRecentActivities
      .mockResolvedValueOnce(recentResponse({ stale: true, as_of: '2026-09-23T06:00:00Z' }))
      .mockResolvedValueOnce(recentResponse({ stale: false, as_of: '2026-09-24T09:00:00Z' }));
    const { result } = renderRecent();

    await waitFor(() => expect(result.current.staleRefetch).toBe('pending'));
    expect(result.current.stale).toBe(true);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(1);

    // Not a moment before the delay.
    await elapse(HOME_STALE_REFETCH_DELAY_MS - 1);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(1);

    await elapse(1);
    await waitFor(() => expect(result.current.staleRefetch).toBe('done'));
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(2);
    expect(result.current.stale).toBe(false);
    expect(result.current.asOf).toBe('2026-09-24T09:00:00Z');
  });

  it('never asks a third time when the second answer is still stale', async () => {
    mockGetRecentActivities.mockResolvedValue(recentResponse({ stale: true }));
    const { result } = renderRecent();

    await waitFor(() => expect(result.current.staleRefetch).toBe('pending'));
    await elapse(HOME_STALE_REFETCH_DELAY_MS);
    await waitFor(() => expect(result.current.staleRefetch).toBe('done'));

    await elapse(HOME_STALE_REFETCH_DELAY_MS * 10);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(2);
    expect(result.current.stale).toBe(true);
  });

  it('schedules nothing for a fresh answer', async () => {
    mockGetRecentActivities.mockResolvedValue(recentResponse({ stale: false }));
    const { result } = renderRecent();

    await waitFor(() => expect(result.current.hasData).toBe(true));
    await elapse(HOME_STALE_REFETCH_DELAY_MS * 2);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(1);
    expect(result.current.staleRefetch).toBe('none');
  });

  it('cancels the follow-up when the screen goes away', async () => {
    mockGetRecentActivities.mockResolvedValue(recentResponse({ stale: true }));
    const { result, unmount } = renderRecent();

    await waitFor(() => expect(result.current.staleRefetch).toBe('pending'));
    unmount();
    await elapse(HOME_STALE_REFETCH_DELAY_MS * 2);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(1);
  });

  it('waits for the athlete to come back when the delay runs out in the background', async () => {
    mockGetRecentActivities.mockResolvedValue(recentResponse({ stale: true }));
    const { result } = renderRecent();

    await waitFor(() => expect(result.current.staleRefetch).toBe('pending'));
    // The idle watch expresses backgrounded and idle as "not focused".
    act(() => focusManager.setFocused(false));
    await elapse(HOME_STALE_REFETCH_DELAY_MS * 3);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(1);
    expect(result.current.staleRefetch).toBe('pending');

    await act(async () => {
      focusManager.setFocused(true);
    });
    await waitFor(() => expect(result.current.staleRefetch).toBe('done'));
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(2);
  });
});
