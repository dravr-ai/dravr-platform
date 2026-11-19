// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests that an opened group invite is kept until its /group join turn is answered (registre#1648)
// ABOUTME: Covers a reload after the join was queued, the answered join, another tab's invite and an expired one

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, renderHook } from '@testing-library/react';
import { usePendingInvite } from '../usePendingInvite';
import { PENDING_INVITE_KEY, PENDING_INVITE_TTL_MS, writePendingInvite } from '../../utils/pendingInvite';

describe('usePendingInvite', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.sessionStorage.clear();
    window.history.replaceState(null, '', '/');
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('takes the code from an invite link, stores it and cleans the address bar', () => {
    window.history.replaceState(null, '', '/groups/join/WELCOME2026');

    const { result } = renderHook(() => usePendingInvite());

    expect(result.current.pendingInviteCode).toBe('WELCOME2026');
    expect(window.location.pathname).toBe('/');
    expect(window.localStorage.getItem(PENDING_INVITE_KEY)).toContain('WELCOME2026');
  });

  it('keeps the invite across a reload after the join was queued but before it was answered', () => {
    window.history.replaceState(null, '', '/groups/join/WELCOME2026');
    const firstLoad = renderHook(() => usePendingInvite());

    // The dashboard dispatched the join: this page does not send it again…
    act(() => firstLoad.result.current.onInviteCodeConsumed());
    expect(firstLoad.result.current.pendingInviteCode).toBeNull();

    // …and the page reloads before the reply.
    firstLoad.unmount();
    const reload = renderHook(() => usePendingInvite());

    expect(reload.result.current.pendingInviteCode).toBe('WELCOME2026');
  });

  it('forgets the invite once its join is answered, so a reload sends nothing', () => {
    window.history.replaceState(null, '', '/groups/join/WELCOME2026');
    const firstLoad = renderHook(() => usePendingInvite());

    act(() => firstLoad.result.current.onInviteCodeConsumed());
    act(() => firstLoad.result.current.onInviteJoinAnswered());

    expect(firstLoad.result.current.pendingInviteCode).toBeNull();
    expect(window.localStorage.getItem(PENDING_INVITE_KEY)).toBeNull();

    firstLoad.unmount();
    const reload = renderHook(() => usePendingInvite());
    expect(reload.result.current.pendingInviteCode).toBeNull();
  });

  it('picks up the invite another tab opened, as the email-confirmation tab does', () => {
    // The invite tab stored it; this tab opened on the confirmation link.
    writePendingInvite('FROM-INVITE-TAB');
    window.history.replaceState(null, '', '/verify-email');

    const { result } = renderHook(() => usePendingInvite());

    expect(result.current.pendingInviteCode).toBe('FROM-INVITE-TAB');
    // The confirmation link keeps its own path.
    expect(window.location.pathname).toBe('/verify-email');
  });

  it('drops its invite when another tab answers the join', () => {
    writePendingInvite('WELCOME2026');
    const { result } = renderHook(() => usePendingInvite());
    expect(result.current.pendingInviteCode).toBe('WELCOME2026');

    act(() => {
      window.localStorage.removeItem(PENDING_INVITE_KEY);
      window.dispatchEvent(new StorageEvent('storage', { key: PENDING_INVITE_KEY, newValue: null }));
    });

    expect(result.current.pendingInviteCode).toBeNull();
  });

  it('ignores an expired invite', () => {
    const openedAt = Date.UTC(2026, 10, 18, 9, 0, 0);
    writePendingInvite('STALE', openedAt);
    vi.useFakeTimers({ toFake: ['Date'] });
    vi.setSystemTime(openedAt + PENDING_INVITE_TTL_MS + 1);

    const { result } = renderHook(() => usePendingInvite());

    expect(result.current.pendingInviteCode).toBeNull();
    expect(window.localStorage.getItem(PENDING_INVITE_KEY)).toBeNull();
  });
});
