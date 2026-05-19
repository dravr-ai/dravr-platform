// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { renderHook, act } from '@testing-library/react';
import { describe, it, expect, beforeEach, vi, type MockInstance } from 'vitest';
import { useIsMobile, useIsTablet, useIsDesktop } from '../useBreakpoint';

interface ListenerMap {
  change: Array<(event: { matches: boolean }) => void>;
}

interface FakeMediaQueryList {
  matches: boolean;
  media: string;
  addEventListener: (kind: 'change', cb: (event: { matches: boolean }) => void) => void;
  removeEventListener: (kind: 'change', cb: (event: { matches: boolean }) => void) => void;
  fire: (matches: boolean) => void;
}

function makeMatchMediaStub(initial: Record<string, boolean>): {
  spy: MockInstance;
  fire: (query: string, matches: boolean) => void;
} {
  const stores = new Map<string, FakeMediaQueryList>();

  const factory = (query: string): FakeMediaQueryList => {
    if (stores.has(query)) {
      const existing = stores.get(query);
      if (existing) {
        return existing;
      }
    }
    const listeners: ListenerMap = { change: [] };
    const mql: FakeMediaQueryList = {
      matches: initial[query] ?? false,
      media: query,
      addEventListener: (kind, cb) => {
        if (kind === 'change') listeners.change.push(cb);
      },
      removeEventListener: (kind, cb) => {
        if (kind !== 'change') return;
        const idx = listeners.change.indexOf(cb);
        if (idx >= 0) listeners.change.splice(idx, 1);
      },
      fire: (matches: boolean) => {
        mql.matches = matches;
        listeners.change.forEach((cb) => cb({ matches }));
      },
    };
    stores.set(query, mql);
    return mql;
  };

  const spy = vi.spyOn(window, 'matchMedia').mockImplementation((query: string) => {
    return factory(query) as unknown as MediaQueryList;
  });

  return {
    spy,
    fire: (query: string, matches: boolean) => {
      const mql = stores.get(query);
      if (mql) mql.fire(matches);
    },
  };
}

describe('useBreakpoint hooks', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it('useIsMobile returns true when (max-width: 767px) matches', () => {
    makeMatchMediaStub({ '(max-width: 767px)': true });
    const { result } = renderHook(() => useIsMobile());
    expect(result.current).toBe(true);
  });

  it('useIsMobile returns false above 767px', () => {
    makeMatchMediaStub({ '(max-width: 767px)': false });
    const { result } = renderHook(() => useIsMobile());
    expect(result.current).toBe(false);
  });

  it('useIsTablet matches 768..1023px range', () => {
    makeMatchMediaStub({ '(min-width: 768px) and (max-width: 1023px)': true });
    const { result } = renderHook(() => useIsTablet());
    expect(result.current).toBe(true);
  });

  it('useIsDesktop matches >=1024px', () => {
    makeMatchMediaStub({ '(min-width: 1024px)': true });
    const { result } = renderHook(() => useIsDesktop());
    expect(result.current).toBe(true);
  });

  it('reacts to media query changes', () => {
    const stub = makeMatchMediaStub({ '(max-width: 767px)': false });
    const { result } = renderHook(() => useIsMobile());
    expect(result.current).toBe(false);
    act(() => {
      stub.fire('(max-width: 767px)', true);
    });
    expect(result.current).toBe(true);
  });
});
