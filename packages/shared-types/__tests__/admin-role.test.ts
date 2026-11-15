// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the one question both clients ask about an account: does it hold the admin role
// ABOUTME: `role` decides alone; the `is_admin` flag sent beside it is never read, whichever way it disagrees

import { describe, it, expect } from 'vitest';
import { hasAdminRole, type User } from '../src/auth';

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
