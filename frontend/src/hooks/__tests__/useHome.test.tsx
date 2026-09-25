// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the Home reads — a stale activity answer is asked for exactly once more, after the delay, while focused
// ABOUTME: Red if the page polls, asks before the delay, asks an idle tab, or sends the plan read without the app's language

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { act, renderHook } from '@testing-library/react';
import type { ReactNode } from 'react';
import { QueryClient, QueryClientProvider, focusManager, notifyManager } from '@tanstack/react-query';
import { HOME_STALE_REFETCH_DELAY_MS } from '@pierre/shared-constants';
import type { RecentActivitiesResponse, TrainingPlanResponse } from '@pierre/shared-types';
import { useRecentActivities, useTrainingPlan } from '../useHome';

const api = vi.hoisted(() => ({
  getRecentActivities: vi.fn<() => Promise<RecentActivitiesResponse>>(),
  getActivityRoute: vi.fn(),
  getTrainingPlan: vi.fn<(locale?: string) => Promise<TrainingPlanResponse>>(),
}));

vi.mock('../../services/api', () => ({
  athleteApi: api,
  providersApi: { getProvidersStatus: vi.fn() },
}));

const STALE: RecentActivitiesResponse = { activities: [], as_of: '2026-09-23T06:00:00Z', stale: true };
const FRESH: RecentActivitiesResponse = { activities: [], as_of: '2026-09-24T08:15:00Z', stale: false };

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

beforeEach(() => {
  vi.clearAllMocks();
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
  // React Query batches its notifications on a zero-delay timeout; run them
  // inline so the fake clock only has to drive the hook's own delay.
  notifyManager.setScheduler((callback) => callback());
  focusManager.setFocused(true);
});

afterEach(() => {
  vi.useRealTimers();
  notifyManager.setScheduler((callback) => setTimeout(callback, 0));
  focusManager.setFocused(undefined);
});

describe('useRecentActivities', () => {
  it('asks exactly once more after the delay, and stops there even when the answer is still stale', async () => {
    api.getRecentActivities.mockResolvedValue(STALE);
    const { result } = renderHook(() => useRecentActivities(), { wrapper });
    await flush();

    expect(result.current.data).toEqual(STALE);
    expect(result.current.refreshing).toBe(true);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(1);

    await flush(HOME_STALE_REFETCH_DELAY_MS - 1);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(1);

    await flush(1);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(2);
    expect(result.current.refreshing).toBe(false);

    // No interval: however long the page stays open, nothing more is asked.
    await flush(HOME_STALE_REFETCH_DELAY_MS * 10);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(2);
  });

  it('shows the fresh answer the second read brings', async () => {
    api.getRecentActivities.mockResolvedValueOnce(STALE).mockResolvedValueOnce(FRESH);
    const { result } = renderHook(() => useRecentActivities(), { wrapper });
    await flush();
    await flush(HOME_STALE_REFETCH_DELAY_MS);

    expect(result.current.data).toEqual(FRESH);
    expect(result.current.refreshing).toBe(false);
  });

  it('never schedules a second read for a fresh answer', async () => {
    api.getRecentActivities.mockResolvedValue(FRESH);
    const { result } = renderHook(() => useRecentActivities(), { wrapper });
    await flush();
    await flush(HOME_STALE_REFETCH_DELAY_MS * 2);

    expect(result.current.refreshing).toBe(false);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(1);
  });

  it('waits for an idle or hidden tab to come back before asking, and asks once when it does', async () => {
    api.getRecentActivities.mockResolvedValue(STALE);
    const { result } = renderHook(() => useRecentActivities(), { wrapper });
    await flush();

    // The idle watch marks an untouched tab unfocused.
    focusManager.setFocused(false);
    await flush(HOME_STALE_REFETCH_DELAY_MS * 3);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(1);
    expect(result.current.refreshing).toBe(true);

    // React Query's own focus refetch and the delayed read share one request.
    await act(async () => {
      focusManager.setFocused(true);
    });
    await flush();
    expect(api.getRecentActivities).toHaveBeenCalledTimes(2);
    expect(result.current.refreshing).toBe(false);
  });

  it('cancels the pending read when the page goes away', async () => {
    api.getRecentActivities.mockResolvedValue(STALE);
    const { unmount } = renderHook(() => useRecentActivities(), { wrapper });
    await flush();
    unmount();

    await flush(HOME_STALE_REFETCH_DELAY_MS * 2);
    expect(api.getRecentActivities).toHaveBeenCalledTimes(1);
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
