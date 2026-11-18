// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the one question both clients ask about an account: does it hold the admin role, and does the web draw its console
// ABOUTME: `role` decides alone; the `is_admin` flag sent beside it is never read, whichever way it disagrees

import { describe, it, expect } from 'vitest';
import { actsAsAdmin, hasAdminRole, type User } from '../src/auth';

describe('hasAdminRole', () => {
  it('answers from the role', () => {
    expect(hasAdminRole({ role: 'admin' })).toBe(true);
    expect(hasAdminRole({ role: 'user' })).toBe(false);
  });

  it('ignores an is_admin flag that disagrees with the role, both ways', () => {
    const demotedFlag = { role: 'admin', is_admin: false } as const;
    const staleFlag = { role: 'user', is_admin: true } as const;
    expect(hasAdminRole(demotedFlag)).toBe(true);
    expect(hasAdminRole(staleFlag)).toBe(false);
  });

  it('refuses a signed-out viewer and a role the server no longer issues', () => {
    expect(hasAdminRole(null)).toBe(false);
    expect(hasAdminRole(undefined)).toBe(false);
    // What a browser cached before the roles were folded: a value the type no
    // longer admits, and the helper gives it no alias.
    const cachedBeforeTheFold = { role: 'super_admin' } as unknown as Pick<User, 'role'>;
    expect(hasAdminRole(cachedBeforeTheFold)).toBe(false);
  });
});

describe('actsAsAdmin', () => {
  it('draws the console for an admin in the admin mode, and before any mode is chosen', () => {
    expect(actsAsAdmin({ role: 'admin' }, 'admin')).toBe(true);
    expect(actsAsAdmin({ role: 'admin' }, null)).toBe(true);
    expect(actsAsAdmin({ role: 'admin' }, undefined)).toBe(true);
  });

  it('draws the athlete app for an admin who chose it', () => {
    expect(actsAsAdmin({ role: 'admin' }, 'athlete')).toBe(false);
  });

  it('never draws the console for an athlete, whatever mode a browser kept', () => {
    expect(actsAsAdmin({ role: 'user' }, 'admin')).toBe(false);
    expect(actsAsAdmin({ role: 'user' }, null)).toBe(false);
    expect(actsAsAdmin(null, 'admin')).toBe(false);
  });
});
