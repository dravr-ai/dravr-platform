// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the shared preferred-calendar hook — when the setting shows, a choice shown at once, a refused one rolled back
// ABOUTME: Red if Connections shows a calendar the server did not record, or hides a recorded one that is no longer connected (carnet#1393)

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { createElement, type ReactNode } from 'react';
import { act, renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { PreferredCalendar } from '@pierre/shared-types';
import { createPreferredCalendarHook, hasCalendarChoice } from '../src/preferredCalendarHook';

const intervals = { provider: 'intervals_icu', name: 'Intervals.icu' };
const garmin = { provider: 'garmin', name: 'Garmin Connect' };

const getPreferredCalendar = vi.fn<() => Promise<PreferredCalendar>>();
const setPreferredCalendar = vi.fn<(provider: string) => Promise<PreferredCalendar>>();
const clearPreferredCalendar = vi.fn<() => Promise<PreferredCalendar>>();
const usePreferredCalendar = createPreferredCalendarHook({
  getPreferredCalendar,
  setPreferredCalendar,
  clearPreferredCalendar,
});

function setup() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client }, children);
  return { client, wrapper };
}

describe('hasCalendarChoice', () => {
  it('shows the setting for several calendars or a recorded one, never for one calendar and no choice', () => {
    expect(hasCalendarChoice(null)).toBe(false);
    expect(hasCalendarChoice({ chosen: null, chosen_connected: false, calendars: [intervals] })).toBe(false);
    expect(hasCalendarChoice({ chosen: null, chosen_connected: false, calendars: [intervals, garmin] })).toBe(true);
    expect(hasCalendarChoice({ chosen: garmin, chosen_connected: false, calendars: [intervals] })).toBe(true);
  });
});

describe('createPreferredCalendarHook', () => {
  beforeEach(() => {
    getPreferredCalendar.mockReset();
    setPreferredCalendar.mockReset();
    clearPreferredCalendar.mockReset();
  });

  it('selects the recorded calendar only while it is connected', async () => {
    getPreferredCalendar.mockResolvedValue({ chosen: garmin, chosen_connected: false, calendars: [intervals] });
    const { wrapper } = setup();
    const { result } = renderHook(() => usePreferredCalendar('intervals_icu'), { wrapper });

    expect(result.current.calendar).toBeNull();
    await waitFor(() => expect(result.current.calendar).not.toBeNull());
    expect(result.current.visible).toBe(true);
    expect(result.current.selected).toBeNull();
  });

  it('shows a choice before the server answers, then settles on what it recorded', async () => {
    getPreferredCalendar.mockResolvedValue({ chosen: null, chosen_connected: false, calendars: [intervals, garmin] });
    let answer: (stored: PreferredCalendar) => void = () => {};
    setPreferredCalendar.mockImplementation(
      () => new Promise<PreferredCalendar>((resolve) => {
        answer = resolve;
      }),
    );
    const { wrapper } = setup();
    const { result } = renderHook(() => usePreferredCalendar('garmin,intervals_icu'), { wrapper });
    await waitFor(() => expect(result.current.calendar).not.toBeNull());

    act(() => result.current.choose('garmin'));
    await waitFor(() => expect(result.current.selected).toBe('garmin'));
    expect(setPreferredCalendar).toHaveBeenCalledWith('garmin');

    act(() => answer({ chosen: garmin, chosen_connected: true, calendars: [intervals, garmin] }));
    await waitFor(() => expect(result.current.isUpdating).toBe(false));
    expect(result.current.selected).toBe('garmin');
    expect(result.current.saveFailed).toBe(false);
  });

  it('forgets the choice with null', async () => {
    getPreferredCalendar.mockResolvedValue({ chosen: intervals, chosen_connected: true, calendars: [intervals, garmin] });
    clearPreferredCalendar.mockResolvedValue({ chosen: null, chosen_connected: false, calendars: [intervals, garmin] });
    const { wrapper } = setup();
    const { result } = renderHook(() => usePreferredCalendar('garmin,intervals_icu'), { wrapper });
    await waitFor(() => expect(result.current.selected).toBe('intervals_icu'));

    act(() => result.current.choose(null));
    await waitFor(() => expect(result.current.selected).toBeNull());
    expect(clearPreferredCalendar).toHaveBeenCalledTimes(1);
    expect(setPreferredCalendar).not.toHaveBeenCalled();
  });

  it('rolls a refused choice back to what the server holds, and says it failed', async () => {
    getPreferredCalendar.mockResolvedValue({ chosen: intervals, chosen_connected: true, calendars: [intervals, garmin] });
    setPreferredCalendar.mockRejectedValue(new Error('400'));
    const { wrapper } = setup();
    const { result } = renderHook(() => usePreferredCalendar('garmin,intervals_icu'), { wrapper });
    await waitFor(() => expect(result.current.selected).toBe('intervals_icu'));

    act(() => result.current.choose('garmin'));
    await waitFor(() => expect(result.current.saveFailed).toBe(true));
    await waitFor(() => expect(result.current.selected).toBe('intervals_icu'));
  });

  it('reads the calendars again when the connected set changes, and only then', async () => {
    getPreferredCalendar.mockResolvedValue({ chosen: null, chosen_connected: false, calendars: [intervals, garmin] });
    const { wrapper } = setup();
    const { result, rerender } = renderHook(({ connected }) => usePreferredCalendar(connected), {
      wrapper,
      initialProps: { connected: 'garmin,intervals_icu' },
    });
    await waitFor(() => expect(result.current.calendar).not.toBeNull());
    const reads = getPreferredCalendar.mock.calls.length;

    rerender({ connected: 'garmin,intervals_icu' });
    expect(getPreferredCalendar).toHaveBeenCalledTimes(reads);

    rerender({ connected: 'intervals_icu' });
    await waitFor(() => expect(getPreferredCalendar).toHaveBeenCalledTimes(reads + 1));
  });
});
