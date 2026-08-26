// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
// ABOUTME: Proves the idle stop actually stops — a polling query goes quiet and comes back on interaction
// ABOUTME: Drives the real useQuery + focusManager path, not the constant, because the constant proves nothing

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { renderHook, waitFor, act } from '@testing-library/react';
import React from 'react';
import { QueryClient, QueryClientProvider, useQuery, focusManager } from '@tanstack/react-query';
import { IDLE_STOP_AFTER_MS, QUERY_FOCUS_POLICY } from '@pierre/shared-constants';
import { useIdleWatch } from '../useIdleWatch';
import { holdIdleWhileBusy, idleSignal, resetIdleAbort } from '../../services/api/idleSignal';

/** Shorter than the idle threshold, so several polls land before it fires. */
const POLL_INTERVAL_MS = 30_000;

function wrapper(client: QueryClient) {
  return function Wrapper({ children }: { children: React.ReactNode }) {
    return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
  };
}

/** A polling query plus the idle watch, exactly as the app mounts them. */
function usePollingScreen(queryFn: () => Promise<number>) {
  useIdleWatch();
  return useQuery({
    queryKey: ['idle-watch-probe'],
    queryFn,
    refetchInterval: POLL_INTERVAL_MS,
  });
}

describe('useIdleWatch', () => {
  let client: QueryClient;

  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    client = new QueryClient({
      defaultOptions: { queries: { ...QUERY_FOCUS_POLICY, retry: false, gcTime: Infinity } },
    });
  });

  afterEach(() => {
    client.clear();
    focusManager.setFocused(undefined);
    vi.useRealTimers();
  });

  it('stops the recurring poll once nobody has touched the tab, and resumes on interaction', async () => {
    const queryFn = vi.fn().mockResolvedValue(1);
    const { unmount } = renderHook(() => usePollingScreen(queryFn), {
      wrapper: wrapper(client),
    });

    // The mount fetch, then a poll: the interval is live while somebody is here.
    await waitFor(() => expect(queryFn).toHaveBeenCalledTimes(1));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(POLL_INTERVAL_MS);
    });
    await waitFor(() => expect(queryFn).toHaveBeenCalledTimes(2));

    // Cross the idle threshold with no interaction at all.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(IDLE_STOP_AFTER_MS);
    });
    expect(focusManager.isFocused()).toBe(false);

    // From here the poll is silent, however long the tab stays open. Ten more
    // intervals — five minutes of wall clock — and not one request.
    const callsAtIdle = queryFn.mock.calls.length;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(POLL_INTERVAL_MS * 10);
    });
    expect(queryFn).toHaveBeenCalledTimes(callsAtIdle);

    // The athlete comes back. The next interval fires again.
    await act(async () => {
      window.dispatchEvent(new Event('pointermove'));
    });
    expect(focusManager.isFocused()).toBe(true);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(POLL_INTERVAL_MS);
    });
    await waitFor(() => expect(queryFn.mock.calls.length).toBeGreaterThan(callsAtIdle));

    unmount();
  });

  it('keeps polling while the athlete is reading — an interaction resets the deadline', async () => {
    const queryFn = vi.fn().mockResolvedValue(1);
    const { unmount } = renderHook(() => usePollingScreen(queryFn), {
      wrapper: wrapper(client),
    });
    await waitFor(() => expect(queryFn).toHaveBeenCalledTimes(1));

    // Scroll every two minutes for twelve minutes. Well past the threshold in
    // total elapsed time, never close to it between interactions.
    for (let i = 0; i < 6; i += 1) {
      await act(async () => {
        await vi.advanceTimersByTimeAsync(2 * 60 * 1000);
        window.dispatchEvent(new Event('scroll'));
      });
    }

    expect(focusManager.isFocused()).toBe(true);
    const callsBefore = queryFn.mock.calls.length;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(POLL_INTERVAL_MS);
    });
    expect(queryFn.mock.calls.length).toBeGreaterThan(callsBefore);

    unmount();
  });

  it('goes idle immediately when the tab is hidden, without waiting out the threshold', async () => {
    const queryFn = vi.fn().mockResolvedValue(1);
    const visibility = vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden');
    const { unmount } = renderHook(() => usePollingScreen(queryFn), {
      wrapper: wrapper(client),
    });
    await waitFor(() => expect(queryFn).toHaveBeenCalledTimes(1));

    await act(async () => {
      document.dispatchEvent(new Event('visibilitychange'));
    });
    expect(focusManager.isFocused()).toBe(false);

    const callsAtHide = queryFn.mock.calls.length;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(POLL_INTERVAL_MS * 4);
    });
    expect(queryFn).toHaveBeenCalledTimes(callsAtHide);

    visibility.mockReturnValue('visible');
    await act(async () => {
      document.dispatchEvent(new Event('visibilitychange'));
    });
    expect(focusManager.isFocused()).toBe(true);

    visibility.mockRestore();
    unmount();
  });

  it('aborts the open turn stream when it goes idle, and hands back a fresh signal', async () => {
    const queryFn = vi.fn().mockResolvedValue(1);
    const { unmount } = renderHook(() => usePollingScreen(queryFn), {
      wrapper: wrapper(client),
    });
    await waitFor(() => expect(queryFn).toHaveBeenCalledTimes(1));

    // The signal a turn sent right now would ride.
    const inFlight = idleSignal();
    expect(inFlight.aborted).toBe(false);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(IDLE_STOP_AFTER_MS);
    });
    expect(inFlight.aborted).toBe(true);

    // A turn sent after the athlete returns must not be born aborted.
    await act(async () => {
      window.dispatchEvent(new Event('keydown'));
    });
    expect(idleSignal().aborted).toBe(false);

    unmount();
  });

  it('hands focus tracking back to React Query when it unmounts', async () => {
    const queryFn = vi.fn().mockResolvedValue(1);
    const { unmount } = renderHook(() => usePollingScreen(queryFn), {
      wrapper: wrapper(client),
    });
    await waitFor(() => expect(queryFn).toHaveBeenCalledTimes(1));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(IDLE_STOP_AFTER_MS);
    });
    expect(focusManager.isFocused()).toBe(false);

    unmount();
    // A pinned `false` would leave every later screen convinced the tab is
    // gone. Unmounting must restore the browser's own answer.
    expect(focusManager.isFocused()).toBe(true);
  });


  it('does not abort a turn the athlete is waiting for', async () => {
    // The regression: a tool-heavy turn that outruns the threshold was aborted
    // and its tokens thrown away, because "no pointer movement" was read as
    // "nobody is here" while the athlete was watching for the answer.
    // Earlier cases in this file leave the shared controller tripped; a turn
    // sent after the athlete returns rides a fresh one.
    resetIdleAbort();
    const client = new QueryClient({ defaultOptions: { queries: QUERY_FOCUS_POLICY } });
    const queryFn = vi.fn(async () => 1);
    renderHook(() => usePollingScreen(queryFn), { wrapper: wrapper(client) });
    await waitFor(() => expect(queryFn).toHaveBeenCalled());

    const signalBefore = idleSignal();
    const release = holdIdleWhileBusy();

    // Well past the threshold, with the turn still streaming and nobody
    // touching anything.
    await act(async () => {
      vi.advanceTimersByTime(IDLE_STOP_AFTER_MS * 3);
    });

    expect(signalBefore.aborted).toBe(false);
    expect(focusManager.isFocused()).toBe(true);

    // Once the turn finishes, the threshold starts measuring the quiet after
    // it -- so the client still goes idle, just not mid-answer.
    release();
    await act(async () => {
      vi.advanceTimersByTime(IDLE_STOP_AFTER_MS + 1_000);
    });
    expect(focusManager.isFocused()).toBe(false);
    expect(idleSignal().aborted).toBe(true);
  });

});
