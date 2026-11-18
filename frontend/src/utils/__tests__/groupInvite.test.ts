// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the tab's carry of a /groups/join/<code> link: read from the path, kept across reloads, dropped by code
// ABOUTME: A browser refusing storage keeps the code for the page's life instead of losing it (carnet#1648)

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  captureGroupInviteLink,
  dropCarriedGroupInvite,
  groupInviteCodeFromPath,
  readCarriedGroupInvite,
} from '../groupInvite';

describe('group invite carry', () => {
  beforeEach(() => {
    sessionStorage.clear();
    window.history.replaceState(null, '', '/');
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('reads the code a /groups/join/<code> path names, decoded', () => {
    expect(groupInviteCodeFromPath('/groups/join/MRT2026X')).toBe('MRT2026X');
    expect(groupInviteCodeFromPath('/groups/join/AB%20CD')).toBe('AB CD');
    expect(groupInviteCodeFromPath('/groups/join/%E0%A4%A')).toBeNull();
    expect(groupInviteCodeFromPath('/groups/join/MRT2026X/extra')).toBeNull();
    expect(groupInviteCodeFromPath('/')).toBeNull();
  });

  it('carries the link code for the tab and takes the link out of the address bar', () => {
    window.history.replaceState(null, '', '/groups/join/MRT2026X');

    captureGroupInviteLink();

    expect(window.location.pathname).toBe('/');
    // What a reload of the tab reads back.
    expect(sessionStorage.getItem('dravr.pending_invite_code')).toBe('MRT2026X');
    expect(readCarriedGroupInvite()).toBe('MRT2026X');
  });

  it('leaves the carry alone on a page that is not an invite link', () => {
    sessionStorage.setItem('dravr.pending_invite_code', 'MRT2026X');
    window.history.replaceState(null, '', '/register');

    captureGroupInviteLink();

    expect(window.location.pathname).toBe('/register');
    expect(readCarriedGroupInvite()).toBe('MRT2026X');
  });

  it('drops only the code the server answered for, keeping a newer link', () => {
    window.history.replaceState(null, '', '/groups/join/NEWER');
    captureGroupInviteLink();

    dropCarriedGroupInvite('OLDER');
    expect(readCarriedGroupInvite()).toBe('NEWER');

    dropCarriedGroupInvite('NEWER');
    expect(readCarriedGroupInvite()).toBeNull();
  });

  it('keeps the code for the page when the browser refuses storage', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new DOMException('denied', 'SecurityError');
    });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new DOMException('denied', 'SecurityError');
    });
    window.history.replaceState(null, '', '/groups/join/MRT2026X');

    captureGroupInviteLink();

    expect(window.location.pathname).toBe('/');
    expect(readCarriedGroupInvite()).toBe('MRT2026X');
    dropCarriedGroupInvite('MRT2026X');
    expect(readCarriedGroupInvite()).toBeNull();
  });
});
