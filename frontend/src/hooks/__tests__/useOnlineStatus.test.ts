// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves the hook reports connectivity on mount and reacts to both transitions
// ABOUTME: A tab that was ALREADY offline when it mounted never hears an event, so mount must read

import { describe, it, expect, afterEach } from 'vitest';
import { renderHook, act } from '@testing-library/react';
import { useOnlineStatus } from '../useOnlineStatus';

function setOnLine(value: boolean) {
  Object.defineProperty(window.navigator, 'onLine', {
    configurable: true,
    get: () => value,
  });
}

afterEach(() => setOnLine(true));

describe('useOnlineStatus', () => {
  it('reports online when the browser is connected', () => {
    setOnLine(true);
    const { result } = renderHook(() => useOnlineStatus());
    expect(result.current).toBe(true);
  });

  it('reports offline on mount when the tab was ALREADY offline', () => {
    // The events only fire on a transition. An installed PWA opened from the
    // home screen with no signal never gets one, so reading the current value
    // at mount is the only thing that catches it — and that launch is exactly
    // the case the service worker makes possible.
    setOnLine(false);
    const { result } = renderHook(() => useOnlineStatus());
    expect(result.current).toBe(false);
  });

  it('follows the connection down and back up', () => {
    setOnLine(true);
    const { result } = renderHook(() => useOnlineStatus());
    expect(result.current).toBe(true);

    act(() => {
      setOnLine(false);
      window.dispatchEvent(new Event('offline'));
    });
    expect(result.current).toBe(false);

    act(() => {
      setOnLine(true);
      window.dispatchEvent(new Event('online'));
    });
    expect(result.current).toBe(true);
  });

  it('stops listening once unmounted', () => {
    setOnLine(true);
    const { result, unmount } = renderHook(() => useOnlineStatus());
    unmount();
    act(() => {
      setOnLine(false);
      window.dispatchEvent(new Event('offline'));
    });
    // Still the last value it saw while mounted — no update after teardown.
    expect(result.current).toBe(true);
  });
});
