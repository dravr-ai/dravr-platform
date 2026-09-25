// ABOUTME: Proves the mobile idle stop actually stops — a polling query goes quiet and a touch revives it
// ABOUTME: Drives the real QueryProvider, useQuery and focusManager, not the constant

import React from 'react';
import { render, waitFor, act, fireEvent, screen } from '@testing-library/react-native';
import { Text, Pressable, AppState, type AppStateStatus } from 'react-native';
import { useQuery, focusManager } from '@tanstack/react-query';
import {
  holdIdleWhileBusy,
  IDLE_STOP_AFTER_MS,
  idleSignal,
  resetIdleAbort,
} from '@pierre/shared-constants';

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

  it('stops polling the moment the app is backgrounded, and resumes on return', async () => {
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

  it('treats an app launched into the background as backgrounded from the start', async () => {
    // A notification action or a background fetch starts the app with no
    // change event at all; nobody is looking until it is opened. A turn it
    // holds is on the same clock as one sent before leaving.
    resetIdleAbort();
    const launched = Object.getOwnPropertyDescriptor(AppState, 'currentState');
    Object.defineProperty(AppState, 'currentState', {
      value: 'background',
      configurable: true,
      writable: true,
    });
    try {
      const queryFn = jest.fn().mockResolvedValue(1);
      render(
        <QueryProvider>
          <PollingScreen queryFn={queryFn} />
        </QueryProvider>,
      );
      await waitFor(() => expect(queryFn).toHaveBeenCalledTimes(1));
      expect(focusManager.isFocused()).toBe(false);

      const inFlight = idleSignal();
      const release = holdIdleWhileBusy();
      await act(async () => {
        jest.advanceTimersByTime(IDLE_STOP_AFTER_MS);
      });
      expect(inFlight.aborted).toBe(true);

      // Opening the app is the return.
      await act(async () => {
        appStateListeners[0]('active');
      });
      expect(focusManager.isFocused()).toBe(true);
      expect(idleSignal().aborted).toBe(false);
      release();
    } finally {
      if (launched) {
        Object.defineProperty(AppState, 'currentState', launched);
      } else {
        delete (AppState as { currentState?: unknown }).currentState;
      }
    }
  });

  it('keeps a turn in flight across a trip to another app shorter than the threshold', async () => {
    // carnet#500: an athlete who leaves for the Strava app to authorize is
    // back in a minute, and the reply the server is still writing must be
    // there when they are. Earlier cases leave the shared controller tripped.
    resetIdleAbort();
    const queryFn = jest.fn().mockResolvedValue(1);
    render(
      <QueryProvider>
        <PollingScreen queryFn={queryFn} />
      </QueryProvider>,
    );
    await waitFor(() => expect(queryFn).toHaveBeenCalledTimes(1));

    const inFlight = idleSignal();
    const release = holdIdleWhileBusy();

    // iOS passes through `inactive` on the way out; neither state drops it.
    await act(async () => {
      appStateListeners[0]('inactive');
      appStateListeners[0]('background');
    });
    expect(focusManager.isFocused()).toBe(false);
    expect(inFlight.aborted).toBe(false);

    await act(async () => {
      jest.advanceTimersByTime(IDLE_STOP_AFTER_MS - 1_000);
    });
    expect(inFlight.aborted).toBe(false);

    await act(async () => {
      appStateListeners[0]('active');
    });
    expect(focusManager.isFocused()).toBe(true);
    expect(inFlight.aborted).toBe(false);

    release();
  });

  it('drops a turn stream once the app has stayed backgrounded for the whole threshold', async () => {
    resetIdleAbort();
    const queryFn = jest.fn().mockResolvedValue(1);
    render(
      <QueryProvider>
        <PollingScreen queryFn={queryFn} />
      </QueryProvider>,
    );
    await waitFor(() => expect(queryFn).toHaveBeenCalledTimes(1));

    const inFlight = idleSignal();
    const release = holdIdleWhileBusy();

    await act(async () => {
      appStateListeners[0]('background');
    });
    await act(async () => {
      jest.advanceTimersByTime(IDLE_STOP_AFTER_MS);
    });
    expect(inFlight.aborted).toBe(true);

    // The return starts a fresh stretch: the next turn is not born aborted.
    await act(async () => {
      appStateListeners[0]('active');
    });
    expect(idleSignal().aborted).toBe(false);
    expect(focusManager.isFocused()).toBe(true);

    release();
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
