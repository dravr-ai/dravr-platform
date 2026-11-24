// ABOUTME: Unit tests for headless UI hooks (useAsyncAction, useToast)
// ABOUTME: Tests hook state management, callbacks, and accessibility props

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { renderHook, act } from '@testing-library/react';
import { useAsyncAction } from '../src/useAsyncAction';
import { useToast } from '../src/useToast';

describe('useAsyncAction', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('starts in idle state', () => {
    const { result } = renderHook(() =>
      useAsyncAction({ action: async () => 'result' })
    );
    expect(result.current.isIdle).toBe(true);
    expect(result.current.isLoading).toBe(false);
  });

  it('transitions through loading to success', async () => {
    let resolve: (value: string) => void;
    const action = () => new Promise<string>((r) => { resolve = r; });

    const { result } = renderHook(() =>
      useAsyncAction({ action })
    );

    let executePromise: Promise<string | null>;
    act(() => {
      executePromise = result.current.execute();
    });

    expect(result.current.isLoading).toBe(true);

    await act(async () => {
      resolve!('done');
      await executePromise;
    });

    expect(result.current.isSuccess).toBe(true);
    expect(result.current.result).toBe('done');
  });

  it('transitions to error state on failure', async () => {
    const action = async () => { throw new Error('fail'); };
    const onError = vi.fn();

    const { result } = renderHook(() =>
      useAsyncAction({ action, onError })
    );

    await act(async () => {
      await result.current.execute();
    });

    expect(result.current.isError).toBe(true);
    expect(result.current.error).toBeInstanceOf(Error);
    expect(onError).toHaveBeenCalled();
  });

  it('prevents duplicate execution while loading', async () => {
    let callCount = 0;
    const action = async () => { callCount++; return 'ok'; };

    const { result } = renderHook(() =>
      useAsyncAction({ action })
    );

    await act(async () => {
      // Fire two executions in quick succession
      const p1 = result.current.execute();
      const p2 = result.current.execute();
      await Promise.all([p1, p2]);
    });

    expect(callCount).toBe(1);
  });

  it('calls onSuccess callback', async () => {
    const onSuccess = vi.fn();
    const { result } = renderHook(() =>
      useAsyncAction({ action: async () => 42, onSuccess })
    );

    await act(async () => {
      await result.current.execute();
    });

    expect(onSuccess).toHaveBeenCalledWith(42);
  });

  it('resets to idle manually', async () => {
    const { result } = renderHook(() =>
      useAsyncAction({ action: async () => 'ok' })
    );

    await act(async () => {
      await result.current.execute();
    });

    expect(result.current.isSuccess).toBe(true);

    act(() => result.current.reset());
    expect(result.current.isIdle).toBe(true);
    expect(result.current.result).toBeNull();
  });
});

describe('useToast', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('starts with empty toast queue', () => {
    const { result } = renderHook(() => useToast());
    expect(result.current.toasts).toHaveLength(0);
  });

  it('adds a toast', () => {
    const { result } = renderHook(() => useToast());

    act(() => {
      result.current.addToast({ message: 'Hello' });
    });

    expect(result.current.toasts).toHaveLength(1);
    expect(result.current.toasts[0].message).toBe('Hello');
    expect(result.current.toasts[0].type).toBe('info');
  });

  it('provides convenience methods', () => {
    const { result } = renderHook(() => useToast());

    act(() => {
      result.current.showInfo('Info');
      result.current.showSuccess('Success');
      result.current.showWarning('Warning');
      result.current.showError('Error');
    });

    expect(result.current.toasts).toHaveLength(4);
    expect(result.current.toasts[0].type).toBe('info');
    expect(result.current.toasts[1].type).toBe('success');
    expect(result.current.toasts[2].type).toBe('warning');
    expect(result.current.toasts[3].type).toBe('error');
  });

  it('removes a toast by id', () => {
    const { result } = renderHook(() => useToast());

    let toastId: string;
    act(() => {
      toastId = result.current.showInfo('Removable');
    });

    expect(result.current.toasts).toHaveLength(1);

    act(() => {
      result.current.removeToast(toastId);
    });

    expect(result.current.toasts).toHaveLength(0);
  });

  it('clears all toasts', () => {
    const { result } = renderHook(() => useToast());

    act(() => {
      result.current.showInfo('A');
      result.current.showInfo('B');
      result.current.showInfo('C');
    });

    expect(result.current.toasts).toHaveLength(3);

    act(() => result.current.clearAll());
    expect(result.current.toasts).toHaveLength(0);
  });

  it('respects maxToasts limit', () => {
    const { result } = renderHook(() => useToast({ maxToasts: 2 }));

    act(() => {
      result.current.showInfo('First');
      result.current.showInfo('Second');
      result.current.showInfo('Third');
    });

    expect(result.current.toasts).toHaveLength(2);
    // FIFO: first should be evicted
    expect(result.current.toasts[0].message).toBe('Second');
    expect(result.current.toasts[1].message).toBe('Third');
  });

  it('auto-dismisses after duration', () => {
    const { result } = renderHook(() => useToast({ defaultDuration: 1000 }));

    act(() => {
      result.current.showInfo('Temporary');
    });

    expect(result.current.toasts).toHaveLength(1);

    act(() => {
      vi.advanceTimersByTime(1000);
    });

    expect(result.current.toasts).toHaveLength(0);
  });

  it('uses longer duration for error toasts', () => {
    const { result } = renderHook(() => useToast({ defaultDuration: 1000 }));

    act(() => {
      result.current.showError('Error toast');
    });

    // Error uses max(defaultDuration, 5000) = 5000
    act(() => {
      vi.advanceTimersByTime(1000);
    });
    expect(result.current.toasts).toHaveLength(1);

    act(() => {
      vi.advanceTimersByTime(4000);
    });
    expect(result.current.toasts).toHaveLength(0);
  });

  it('shows one toast for a burst sharing a dedupe key, and returns its id each time', () => {
    const { result } = renderHook(() => useToast());

    const ids: string[] = [];
    // One tick, as five parallel refusals arrive: no render between the adds.
    act(() => {
      for (let i = 0; i < 5; i += 1) {
        ids.push(result.current.addToast({ message: 'Garmin', type: 'error', dedupeKey: 'auth:garmin' }));
      }
      ids.push(result.current.addToast({ message: 'Strava', type: 'error', dedupeKey: 'auth:strava' }));
    });

    expect(result.current.toasts.map(toast => toast.message)).toEqual(['Garmin', 'Strava']);
    expect(new Set(ids.slice(0, 5)).size).toBe(1);
    expect(ids[5]).not.toBe(ids[0]);
  });

  it('shows a keyed toast again once the first is gone', () => {
    const { result } = renderHook(() => useToast({ defaultDuration: 1000 }));

    let first = '';
    act(() => {
      first = result.current.addToast({ message: 'Garmin', dedupeKey: 'auth:garmin' });
    });
    act(() => result.current.removeToast(first));
    act(() => {
      result.current.addToast({ message: 'Garmin', dedupeKey: 'auth:garmin' });
    });
    expect(result.current.toasts).toHaveLength(1);

    // Auto-dismissed releases the key as well.
    act(() => {
      vi.advanceTimersByTime(1000);
    });
    act(() => {
      result.current.addToast({ message: 'Garmin', dedupeKey: 'auth:garmin' });
    });
    expect(result.current.toasts).toHaveLength(1);

    // And so does being pushed out of a full queue.
    const { result: small } = renderHook(() => useToast({ maxToasts: 1 }));
    act(() => {
      small.current.addToast({ message: 'Garmin', dedupeKey: 'auth:garmin' });
      small.current.addToast({ message: 'Other' });
      small.current.addToast({ message: 'Garmin again', dedupeKey: 'auth:garmin' });
    });
    expect(small.current.toasts.map(toast => toast.message)).toEqual(['Garmin again']);
  });
});
