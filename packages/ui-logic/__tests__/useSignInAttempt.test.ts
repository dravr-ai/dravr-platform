// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins that a closed sign-in window cancels its attempt on the server, by the id its steps sent (carnet#1300)
// ABOUTME: Red if a close mid-login sends no cancel, cancels the wrong id, reuses an id, or cancels an idle window

import { describe, expect, it, vi } from 'vitest';
import { renderHook } from '@testing-library/react';
import { useSignInAttempt } from '../src/useSignInAttempt';

/** Ids `attempt-1`, `attempt-2`, … in minting order. */
function minter() {
  let n = 0;
  return vi.fn(() => {
    n += 1;
    return `attempt-${n}`;
  });
}

function setup(initial: { open: boolean; active: boolean }) {
  const mintAttemptId = minter();
  const cancel = vi.fn().mockResolvedValue({ cancelled: true });
  const hook = renderHook(
    ({ open, active }) => useSignInAttempt(open, { active, mintAttemptId, cancel }),
    { initialProps: initial },
  );
  return { ...hook, cancel, mintAttemptId };
}

describe('useSignInAttempt', () => {
  it('cancels the attempt its steps named when the window closes mid-login', () => {
    const { result, rerender, cancel } = setup({ open: true, active: false });
    const attemptId = result.current();
    expect(attemptId).toBe('attempt-1');

    rerender({ open: true, active: true });
    rerender({ open: false, active: true });

    expect(cancel).toHaveBeenCalledTimes(1);
    expect(cancel).toHaveBeenCalledWith(attemptId);
    expect(result.current()).toBeUndefined();
  });

  it('cancels when the window unmounts mid-login', () => {
    const { result, unmount, cancel } = setup({ open: true, active: true });
    const attemptId = result.current();

    unmount();

    expect(cancel).toHaveBeenCalledWith(attemptId);
  });

  it('sends nothing when the window closes with nothing running', () => {
    const { rerender, unmount, cancel } = setup({ open: true, active: false });

    rerender({ open: false, active: false });
    unmount();

    expect(cancel).not.toHaveBeenCalled();
  });

  it('names every reopening with a fresh id and cancels each one once', () => {
    const { result, rerender, unmount, cancel } = setup({ open: true, active: true });
    const first = result.current();
    rerender({ open: false, active: true });

    rerender({ open: true, active: true });
    const second = result.current();
    rerender({ open: false, active: true });
    unmount();

    expect(second).not.toBe(first);
    expect(cancel.mock.calls).toEqual([[first], [second]]);
  });

  it('swallows a cancel that fails', async () => {
    const mintAttemptId = minter();
    const cancel = vi.fn().mockRejectedValue(new Error('offline'));
    const { rerender } = renderHook(
      ({ open }) => useSignInAttempt(open, { active: true, mintAttemptId, cancel }),
      { initialProps: { open: true } },
    );

    rerender({ open: false });
    await Promise.resolve();

    expect(cancel).toHaveBeenCalledTimes(1);
  });

  it('starts a new attempt in the same opening once the server ended the last one, and cancels only the new one', () => {
    const { result, rerender, cancel } = setup({ open: true, active: true });
    const first = result.current();
    result.current.renew();
    const second = result.current();
    expect(second).not.toBe(first);
    expect(cancel).not.toHaveBeenCalled();

    rerender({ open: false, active: true });
    expect(cancel.mock.calls).toEqual([[second]]);
  });

  it('starts nothing while the window is closed', () => {
    const { result, rerender, mintAttemptId } = setup({ open: true, active: false });
    rerender({ open: false, active: false });
    const minted = mintAttemptId.mock.calls.length;
    result.current.renew();
    expect(result.current()).toBeUndefined();
    expect(mintAttemptId.mock.calls.length).toBe(minted);
  });
});
