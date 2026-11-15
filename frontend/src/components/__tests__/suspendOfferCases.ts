// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The accounts the users row and the user drawer are both asked about, and whether the signed-in admin is offered Suspend on each
// ABOUTME: One table for both specs, so the two surfaces cannot be pinned to different rules

import type { User } from '../../types/api';

/** The signed-in admin, as `useAuth` hands it to the drawer. */
export const VIEWER: User = {
  id: 'admin-1',
  email: 'admin@example.com',
  display_name: 'Signed-in Admin',
  role: 'admin',
  is_admin: true,
  user_status: 'active',
  tier: 'enterprise',
  created_at: '2026-01-01T00:00:00Z',
};

function account(id: string, role: User['role'], status: NonNullable<User['user_status']>): User {
  return {
    id,
    email: `${id}@example.com`,
    display_name: id,
    role,
    is_admin: role === 'admin',
    user_status: status,
    tier: 'starter',
    created_at: '2026-01-01T00:00:00Z',
  };
}

export interface SuspendOfferCase {
  name: string;
  user: User;
  offered: boolean;
}

/**
 * What the row and the drawer offer the signed-in admin (`VIEWER`): every
 * active or pending account but its own, as `POST /api/admin/suspend-user/{id}`
 * takes them from a console session. The server refuses an admin its own
 * suspension, so neither surface offers it; the last active admin is the
 * server's refusal, not a row's. A suspended account is offered Reactivate.
 */
export const SUSPEND_OFFER_CASES: readonly SuspendOfferCase[] = [
  { name: 'an active user', user: account('athlete', 'user', 'active'), offered: true },
  { name: 'a pending user', user: account('pending-user', 'user', 'pending'), offered: true },
  { name: 'another active admin', user: account('admin-2', 'admin', 'active'), offered: true },
  { name: "the signed-in admin's own account", user: VIEWER, offered: false },
  { name: 'a suspended user', user: account('suspended-user', 'user', 'suspended'), offered: false },
  { name: 'a suspended admin', user: account('suspended-admin', 'admin', 'suspended'), offered: false },
];

/**
 * The refusal the server sends for the one admin left, as axios rejects with
 * it: `AppError::invalid_input` is a 400 whose body carries the sentence.
 */
export const LAST_ADMIN_REFUSAL = {
  response: {
    status: 400,
    data: {
      code: 'InvalidInput',
      message: 'Cannot suspend the last active admin',
      timestamp: '2026-11-14T12:00:00Z',
    },
  },
};
