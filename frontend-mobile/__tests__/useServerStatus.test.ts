// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for useServerStatus foreground-gated /health polling
// ABOUTME: Verifies the interval pauses on background so an idle app stops pinging the backend

import { renderHook, act, waitFor } from '@testing-library/react-native';
import { AppState, type AppStateStatus } from 'react-native';
import { apiClient } from '../src/services/api';
import { useServerStatus } from '../src/hooks/useServerStatus';

jest.mock('../src/services/api', () => ({
  apiClient: { get: jest.fn() },
}));

const POLL_MS = 30_000;

describe('useServerStatus — foreground-gated polling', () => {
  let appStateHandler: (state: AppStateStatus) => void = () => {};

  beforeEach(() => {
    jest.clearAllMocks();
    jest.useFakeTimers();
    (apiClient.get as jest.Mock).mockResolvedValue({ data: 'ok' });
    Object.defineProperty(AppState, 'currentState', {
      configurable: true,
      writable: true,
      value: 'active',
    });
    jest
      .spyOn(AppState, 'addEventListener')
      .mockImplementation((_event: string, handler: (s: AppStateStatus) => void) => {
        appStateHandler = handler;
        return { remove: jest.fn() } as unknown as ReturnType<typeof AppState.addEventListener>;
      });
  });

  afterEach(() => {
    jest.useRealTimers();
    jest.restoreAllMocks();
  });

  const callCount = () => (apiClient.get as jest.Mock).mock.calls.length;

  it('checks on mount and keeps polling while the app is active', async () => {
    renderHook(() => useServerStatus());
    await waitFor(() => expect(callCount()).toBe(1)); // mount check

    await act(async () => {
      jest.advanceTimersByTime(POLL_MS);
    });
    expect(callCount()).toBe(2); // one interval tick
  });

  it('stops polling when the app goes to the background', async () => {
    renderHook(() => useServerStatus());
    await waitFor(() => expect(callCount()).toBe(1));

    act(() => appStateHandler('background'));
    const atBackground = callCount();

    await act(async () => {
      jest.advanceTimersByTime(POLL_MS * 4);
    });
    expect(callCount()).toBe(atBackground); // no pings while backgrounded
  });

  it('resumes with an immediate check and restarts polling on return to active', async () => {
    renderHook(() => useServerStatus());
    await waitFor(() => expect(callCount()).toBe(1));

    act(() => appStateHandler('background'));
    const before = callCount();

    act(() => appStateHandler('active'));
    expect(callCount()).toBe(before + 1); // immediate check on resume

    await act(async () => {
      jest.advanceTimersByTime(POLL_MS);
    });
    expect(callCount()).toBe(before + 2); // polling resumed
  });
});
