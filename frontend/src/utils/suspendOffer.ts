// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Which accounts the users row and the user drawer offer to suspend: one rule, so neither offers what the other hides
// ABOUTME: Every account not yet suspended except the signed-in admin's own, as the server takes them (POST /api/admin/suspend-user/{id})

import type { User } from '../types/api';

/**
 * Whether the users row and the user drawer offer Suspend on `user` to the
 * signed-in admin `viewer`.
 *
 * Every active or pending account but the viewer's own: a user, an account
 * still waiting for approval, and another admin. The server takes each of
 * these from a console session (a service token without the super-admin flag
 * is refused an admin's; these surfaces never act as one), and refuses two:
 *
 * - the signed-in admin's own account, as it refuses a self-demotion, so
 *   neither surface offers it;
 * - the last active admin's, which no row can know from here: the server
 *   answers "Cannot suspend the last active admin" and the suspend dialog
 *   shows that sentence.
 *
 * An account already suspended is offered Reactivate instead. A pending
 * account is offered Suspend beside Approve, as the Pending queue
 * (`PendingUsersList`) offers both.
 */
export function offersSuspend(
  user: Pick<User, 'id' | 'user_status'>,
  viewer: Pick<User, 'id'> | null,
): boolean {
  if (user.id === viewer?.id) return false;
  return user.user_status === 'active' || user.user_status === 'pending';
}
