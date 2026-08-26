// ABOUTME: Proves the mobile idle stop actually stops — a polling query goes quiet and a touch revives it
// ABOUTME: Drives the real QueryProvider, useQuery and focusManager, not the constant

import React from 'react';
import { render, waitFor, act, fireEvent, screen } from '@testing-library/react-native';
import { Text, Pressable, AppState, type AppStateStatus } from 'react-native';
import { useQuery, focusManager } from '@tanstack/react-query';
import { IDLE_STOP_AFTER_MS } from '@pierre/shared-constants';

jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({ isAuthenticated: true, user: { id: '1' } }),
}));

jest.mock('../src/utils/mmkvStorage', () => ({
  mmkvPersister: {
    persistClient: jest.fn(),
    restoreClient: jest.fn().mockResolvedValue(undefined),
    removeClient: jest.fn(),
  },
  CACHE_TIMES: {
    DEFAULT_STALE_TIME: 60000,
    ACTIVITIES_GC_TIME: 604800000,
    MAX_CACHE_AGE: 604800000,
  },
  clearQueryCache: jest.fn(),
}));

import { QueryProvider } from '../src/providers/QueryProvider';
import { idleSignal } from '../src/services/idleSignal';

/** Shorter than the idle threshold, so several polls land before it fires. */
const POLL_INTERVAL_MS = 30_000;

function PollingScreen({ queryFn }: { queryFn: () => Promise<number> }) {
  const { data } = useQuery({
    queryKey: ['mobile-idle-probe'],
    queryFn,
    refetchInterval: POLL_INTERVAL_MS,
    staleTime: 0,
  });
  return (
    <Pressable testID="tap-target" onPress={() => {}}>
      <Text>{String(data ?? 'pending')}</Text>
    </Pressable>
  );
}

describe('QueryProvider idle contract', () => {
  // The AppState bridge is captured for every test, not just the one that
  // drives it: jest-expo's AppState does not hand back a real subscription,
  // and a half-mocked one would blow up in whichever test unmounted last.
  let appStateListeners: ((status: AppStateStatus) => void)[] = [];
  let appStateSpy: jest.SpyInstance;

  beforeEach(() => {
    jest.useFakeTimers();
    appStateListeners = [];
    appStateSpy = jest
      .spyOn(AppState, 'addEventListener')
      .mockImplementation((_event, listener) => {
        appStateListeners.push(listener as (status: AppStateStatus) => void);
        return { remove: jest.fn() };
      });
  });

  afterEach(() => {
    appStateSpy.mockRestore();
    jest.useRealTimers();
    focusManager.setFocused(undefined);
  });

  /** Fire a touch the way the app's responder-capture wrapper observes one. */
  function touchTheApp() {
    fireEvent(screen.getByTestId('tap-target'), 'startShouldSetResponderCapture', {
      nativeEvent: { touches: [], changedTouches: [], identifier: 1 },
    });
  }

  it('stops the recurring poll once nobody has touched the app, and a touch brings it back', async () => {
    const queryFn = jest.fn().mockResolvedValue(1);
    render(
      <QueryProvider>
        <PollingScreen queryFn={queryFn} />
      </QueryProvider>,
    );

    await waitFor(() => expect(queryFn).toHaveBeenCalledTimes(1));
    await act(async () => {
      jest.advanceTimersByTime(POLL_INTERVAL_MS);
    });
    await waitFor(() => expect(queryFn.mock.calls.length).toBeGreaterThan(1));

    // Cross the idle threshold with no touch at all.
    await act(async () => {
      jest.advanceTimersByTime(IDLE_STOP_AFTER_MS);
    });
    expect(focusManager.isFocused()).toBe(false);

    // From here the poll is silent, however long the app stays open on screen.
    const callsAtIdle = queryFn.mock.calls.length;
    await act(async () => {
      jest.advanceTimersByTime(POLL_INTERVAL_MS * 10);
    });
    expect(queryFn).toHaveBeenCalledTimes(callsAtIdle);

    // A touch anywhere in the app resumes it — the responder-capture wrapper
    // sees every gesture without taking any of them.
    await act(async () => {
      touchTheApp();
    });
    expect(focusManager.isFocused()).toBe(true);

    await act(async () => {
      jest.advanceTimersByTime(POLL_INTERVAL_MS);
    });
    await waitFor(() => expect(queryFn.mock.calls.length).toBeGreaterThan(callsAtIdle));
  });

  it('goes idle the moment the app is backgrounded, without waiting out the threshold', async () => {
    const queryFn = jest.fn().mockResolvedValue(1);
    render(
      <QueryProvider>
        <PollingScreen queryFn={queryFn} />
      </QueryProvider>,
    );
    await waitFor(() => expect(queryFn).toHaveBeenCalledTimes(1));
    expect(appStateListeners).toHaveLength(1);

    await act(async () => {
      appStateListeners[0]('background');
    });
    expect(focusManager.isFocused()).toBe(false);

    const callsAtBackground = queryFn.mock.calls.length;
    await act(async () => {
      jest.advanceTimersByTime(POLL_INTERVAL_MS * 5);
    });
    expect(queryFn).toHaveBeenCalledTimes(callsAtBackground);

    // Returning to the foreground is the interaction that brings it back.
    await act(async () => {
      appStateListeners[0]('active');
    });
    expect(focusManager.isFocused()).toBe(true);
  });

  it('aborts the open turn stream when it goes idle, and hands back a fresh signal', async () => {
    const queryFn = jest.fn().mockResolvedValue(1);
    render(
      <QueryProvider>
        <PollingScreen queryFn={queryFn} />
      </QueryProvider>,
    );
    await waitFor(() => expect(queryFn).toHaveBeenCalledTimes(1));

    const inFlight = idleSignal();
    expect(inFlight.aborted).toBe(false);

    await act(async () => {
      jest.advanceTimersByTime(IDLE_STOP_AFTER_MS);
    });
    expect(inFlight.aborted).toBe(true);

    await act(async () => {
      touchTheApp();
    });
    expect(idleSignal().aborted).toBe(false);
  });
});
