// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Admin console client for coach-seat invitations — issue, list, revoke the email-bound invite carrying coach access (carnet#1524)
// ABOUTME: Super-admin only on the server; same session-authenticated axios and AdminResponse envelope as coachAccessAdmin.ts

import { axios } from './client';

/** One coach-seat invitation neither redeemed nor revoked. */
export interface CoachSeatInvitation {
  email: string;
  invited_by: string | null;
  /** The inviting operator's address, while their account exists. */
  invited_by_email: string | null;
  created_at: string;
  expires_at: string;
  /** The link no longer works: send it again. */
  expired: boolean;
}

/** What asking for a coach-seat invitation did. */
export type CoachSeatInviteResult =
  | { outcome: 'invited'; message: string; email: string; expiresAt: string }
  | {
      /** The address has an account: grant it coach access directly instead. */
      outcome: 'account_exists';
      message: string;
      email: string;
      userId: string;
      displayName: string | null;
      managesRoster: boolean;
    };

export const coachSeatInvitationsApi = {
  /** Every outstanding coach-seat invitation, oldest first, expired ones included. */
  async list(): Promise<CoachSeatInvitation[]> {
    const response = await axios.get('/api/admin/coach-seat-invitations');
    // A body without the list is a failed read, never an empty queue.
    const invitations: unknown = response.data?.data?.invitations;
    if (!Array.isArray(invitations)) {
      throw new Error('Malformed coach-seat invitation listing');
    }
    return invitations as CoachSeatInvitation[];
  },

  /** Invite `email` with a coach seat; sending again replaces the earlier link. */
  async invite(email: string): Promise<CoachSeatInviteResult> {
    const response = await axios.post('/api/admin/coach-seat-invitations', { email });
    const data = response.data?.data ?? {};
    if (data.outcome === 'account_exists') {
      return {
        outcome: 'account_exists',
        message: response.data.message,
        email: data.email,
        userId: data.user_id,
        displayName: data.display_name ?? null,
        managesRoster: data.manages_roster === true,
      };
    }
    return { outcome: 'invited', message: response.data.message, email: data.email, expiresAt: data.expires_at };
  },

  /** Revoke the coach-seat invitation `email` holds, so its link stops working. */
  async revoke(email: string): Promise<{ message: string; revoked: boolean }> {
    const response = await axios.delete(`/api/admin/coach-seat-invitations/${encodeURIComponent(email)}`);
    return { message: response.data.message, revoked: response.data.data?.revoked === true };
  },
};
