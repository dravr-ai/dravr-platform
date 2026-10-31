// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins that a closed sign-in window aborts the request it left in flight (carnet#1300)
// ABOUTME: Red if closing, unmounting or starting a new request leaves the old one running

import { describe, expect, it } from 'vitest';
import { renderHook } from '@testing-library/react';
import { useAbortOnClose } from '../src/useAbortOnClose';

describe('useAbortOnClose', () => {
  it('aborts the request in flight when the window closes', () => {
    const { result, rerender } = renderHook(({ open }) => useAbortOnClose(open), {
      initialProps: { open: true },
    });
    const signal = result.current();
    expect(signal.aborted).toBe(false);

    rerender({ open: false });

    expect(signal.aborted).toBe(true);
  });

  it('aborts the request in flight when the window unmounts', () => {
    const { result, unmount } = renderHook(() => useAbortOnClose(true));
    const signal = result.current();

    unmount();

    expect(signal.aborted).toBe(true);
  });

  it('aborts the earlier request when the window starts another', () => {
    const { result } = renderHook(() => useAbortOnClose(true));
    const first = result.current();

    const second = result.current();

    expect(first.aborted).toBe(true);
    expect(second.aborted).toBe(false);
  });

  it('leaves a request started after a reopen running', () => {
    const { result, rerender } = renderHook(({ open }) => useAbortOnClose(open), {
      initialProps: { open: true },
    });
    result.current();
    rerender({ open: false });
    rerender({ open: true });

    const signal = result.current();

    expect(signal.aborted).toBe(false);
  });
});
