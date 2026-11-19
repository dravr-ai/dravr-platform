// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the stored group invite: shared by every tab through localStorage, with an expiry
// ABOUTME: Covers another tab's invite, an expired or unreadable entry, a refused storage and the per-tab entry it replaces

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  PENDING_INVITE_KEY,
  PENDING_INVITE_TTL_MS,
  clearPendingInvite,
  onPendingInviteClearedElsewhere,
  readPendingInvite,
  writePendingInvite,
} from '../pendingInvite';

const NOW = Date.UTC(2026, 10, 19, 9, 0, 0);

describe('pending group invite storage', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.sessionStorage.clear();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('reads back the invite written, in localStorage where every tab sees it', () => {
    writePendingInvite('WELCOME2026', NOW);

    expect(readPendingInvite(NOW + 1000)).toBe('WELCOME2026');
    expect(JSON.parse(window.localStorage.getItem(PENDING_INVITE_KEY) ?? 'null')).toEqual({
      code: 'WELCOME2026',
      expiresAt: NOW + PENDING_INVITE_TTL_MS,
    });
    expect(window.sessionStorage.getItem(PENDING_INVITE_KEY)).toBeNull();
  });

  it('picks up an invite another tab stored', () => {
    // What the invite tab left before the athlete opened the confirmation
    // link in this one.
    window.localStorage.setItem(
      PENDING_INVITE_KEY,
      JSON.stringify({ code: 'FROM-OTHER-TAB', expiresAt: NOW + 60_000 }),
    );

    expect(readPendingInvite(NOW)).toBe('FROM-OTHER-TAB');
  });

  it('ignores and removes an expired invite', () => {
    writePendingInvite('STALE', NOW);

    expect(readPendingInvite(NOW + PENDING_INVITE_TTL_MS)).toBeNull();
    expect(window.localStorage.getItem(PENDING_INVITE_KEY)).toBeNull();
  });

  it('ignores and removes an entry it cannot read', () => {
    window.localStorage.setItem(PENDING_INVITE_KEY, 'not json');
    expect(readPendingInvite(NOW)).toBeNull();
    expect(window.localStorage.getItem(PENDING_INVITE_KEY)).toBeNull();

    window.localStorage.setItem(PENDING_INVITE_KEY, JSON.stringify({ code: 'NO-EXPIRY' }));
    expect(readPendingInvite(NOW)).toBeNull();
  });

  it('moves a per-tab invite left by the earlier version into localStorage', () => {
    window.sessionStorage.setItem(PENDING_INVITE_KEY, 'OLD-TAB-CODE');

    expect(readPendingInvite(NOW)).toBe('OLD-TAB-CODE');
    expect(window.sessionStorage.getItem(PENDING_INVITE_KEY)).toBeNull();
    expect(readPendingInvite(NOW + 1000)).toBe('OLD-TAB-CODE');
  });

  it('forgets the invite once cleared', () => {
    writePendingInvite('WELCOME2026', NOW);
    clearPendingInvite();
    expect(readPendingInvite(NOW)).toBeNull();
  });

  it('answers no invite, and throws nothing, when storage is refused', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new DOMException('denied', 'SecurityError');
    });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new DOMException('denied', 'SecurityError');
    });
    vi.spyOn(Storage.prototype, 'removeItem').mockImplementation(() => {
      throw new DOMException('denied', 'SecurityError');
    });

    expect(() => writePendingInvite('WELCOME2026', NOW)).not.toThrow();
    expect(readPendingInvite(NOW)).toBeNull();
    expect(() => clearPendingInvite()).not.toThrow();
  });

  it('reports another tab clearing the invite, and nothing else', () => {
    const onCleared = vi.fn();
    const unsubscribe = onPendingInviteClearedElsewhere(onCleared);

    window.dispatchEvent(new StorageEvent('storage', { key: 'other-key', newValue: null }));
    window.dispatchEvent(new StorageEvent('storage', { key: PENDING_INVITE_KEY, newValue: '{}' }));
    expect(onCleared).not.toHaveBeenCalled();

    window.dispatchEvent(new StorageEvent('storage', { key: PENDING_INVITE_KEY, newValue: null }));
    expect(onCleared).toHaveBeenCalledTimes(1);

    unsubscribe();
    window.dispatchEvent(new StorageEvent('storage', { key: PENDING_INVITE_KEY, newValue: null }));
    expect(onCleared).toHaveBeenCalledTimes(1);
  });
});
