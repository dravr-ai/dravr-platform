// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Admin console coach-seat invitations — invite a recruited coach with their seat, see who is invited, re-send or revoke (carnet#1524)
// ABOUTME: Mounted in the coach-access queue; an address with an account is offered the direct grant instead of a link it could never redeem

import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { formatDate } from '@pierre/chat-utils';
import { adminApi, coachSeatInvitationsApi } from '../services/api';
import type { CoachSeatInvitation, CoachSeatInviteResult } from '../services/api/coachSeatInvitations';
import { QUERY_KEYS } from '../constants/queryKeys';
import { Badge, Button, EmptyState, Input } from './ui';
import { ADMIN_LOCALE } from '../utils/adminLocale';

/** An address that already has an account: the direct grant is offered instead. */
type ExistingAccount = Extract<CoachSeatInviteResult, { outcome: 'account_exists' }>;

/**
 * The coach-access decision ADR-018 leaves to a super-admin, taken before the
 * coach signs up: an email-bound, single-use link that creates their account
 * with coach access and the coach role, so they meet no role question and no
 * request. Each invited coach waits here as an "invited" row with its expiry
 * until they redeem it; the link can be sent again (it replaces the earlier
 * one) or revoked.
 */
export default function CoachSeatInvitations() {
  const queryClient = useQueryClient();
  const [email, setEmail] = useState('');
  const [result, setResult] = useState<{ message: string; ok: boolean } | null>(null);
  const [existing, setExisting] = useState<ExistingAccount | null>(null);

  const {
    data: invitations = [],
    isLoading,
    error,
    refetch,
  } = useQuery<CoachSeatInvitation[]>({
    queryKey: QUERY_KEYS.adminUsers.coachSeatInvitations(),
    queryFn: () => coachSeatInvitationsApi.list(),
  });

  const invalidate = () => {
    void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.adminUsers.coachSeatInvitations() });
  };

  const inviteMutation = useMutation({
    mutationFn: (address: string) => coachSeatInvitationsApi.invite(address),
    onSuccess: (data) => {
      setResult({ message: data.message, ok: true });
      setExisting(data.outcome === 'account_exists' ? data : null);
      if (data.outcome === 'invited') setEmail('');
      invalidate();
    },
    onError: (err: unknown) => {
      setExisting(null);
      setResult({ message: errorMessage(err, 'Could not send the invitation'), ok: false });
    },
  });

  const revokeMutation = useMutation({
    mutationFn: (address: string) => coachSeatInvitationsApi.revoke(address),
    onSuccess: (data) => {
      setExisting(null);
      setResult({ message: data.message, ok: true });
      invalidate();
    },
    onError: (err: unknown) => {
      setResult({ message: errorMessage(err, 'Could not revoke the invitation'), ok: false });
    },
  });

  // The existing direct grant, the same decision as "May coach a group" in the user drawer.
  const grantMutation = useMutation({
    mutationFn: (account: ExistingAccount) => adminApi.setUserManagesRoster(account.userId, true),
    onSuccess: (_data, account) => {
      setExisting(null);
      setResult({ message: `${account.email} now has coach access`, ok: true });
      void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.adminUsers.list() });
    },
    onError: (err: unknown) => {
      setResult({ message: errorMessage(err, 'Could not grant coach access'), ok: false });
    },
  });

  const busy = inviteMutation.isPending || revokeMutation.isPending || grantMutation.isPending;

  return (
    <div className="mt-6 border-t border-outline-variant pt-5" data-testid="coach-seat-invitations">
      <h3 className="text-base font-semibold text-on-surface">Invite a coach with their seat</h3>
      <p className="mt-1 max-w-prose text-sm text-on-surface-variant">
        For a coach the team recruited. The link creates their account with coach access, so they skip the role
        question and the request. It works once, for this address only, and only an address with no account can
        be invited.
      </p>

      <form
        className="mt-3 flex flex-col gap-3 sm:flex-row"
        onSubmit={(e) => {
          e.preventDefault();
          if (email.trim()) inviteMutation.mutate(email.trim());
        }}
      >
        <div className="flex-1">
          <Input
            type="email"
            placeholder="coach@example.com"
            aria-label="Coach's email address"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            required
            data-testid="coach-seat-invite-email"
          />
        </div>
        <Button type="submit" disabled={!email.trim() || busy} data-testid="coach-seat-invite-send">
          {inviteMutation.isPending ? 'Sending…' : 'Send invitation'}
        </Button>
      </form>

      {result && (
        <p
          role="status"
          className={`mt-3 text-sm ${result.ok ? 'text-on-surface-variant' : 'text-error'}`}
          data-testid="coach-seat-invite-result"
        >
          {result.message}
        </p>
      )}
      {existing && !existing.managesRoster && (
        <div className="mt-2" data-testid="coach-seat-invite-existing">
          <Button
            size="sm"
            variant="outline"
            onClick={() => grantMutation.mutate(existing)}
            disabled={busy}
            data-testid="coach-seat-invite-grant"
          >
            {grantMutation.isPending ? 'Granting…' : `Grant coach access to ${existing.email}`}
          </Button>
        </div>
      )}

      <div className="mt-5">
        <p className="text-sm font-medium text-on-surface">Invited ({invitations.length})</p>
        {isLoading ? (
          <EmptyState>Loading invitations…</EmptyState>
        ) : error ? (
          <EmptyState action={{ label: 'Retry', onClick: () => void refetch() }}>
            Failed to load coach-seat invitations.
          </EmptyState>
        ) : invitations.length === 0 ? (
          <EmptyState>No coach is invited right now.</EmptyState>
        ) : (
          <ul className="divide-y divide-outline-variant">
            {invitations.map((invitation) => (
              <li
                key={invitation.email}
                className="flex flex-col gap-3 py-3 sm:flex-row sm:items-start sm:justify-between"
                data-testid={`coach-seat-invitation-${invitation.email}`}
              >
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <p className="truncate font-medium text-on-surface">{invitation.email}</p>
                    <Badge variant={invitation.expired ? 'secondary' : 'info'}>
                      {invitation.expired ? 'expired' : 'invited'}
                    </Badge>
                  </div>
                  <p className="text-xs text-outline">
                    {invitation.expired ? 'Expired' : 'Expires'} {formatDate(invitation.expires_at, ADMIN_LOCALE)} ·
                    Sent {formatDate(invitation.created_at, ADMIN_LOCALE)} by{' '}
                    {invitation.invited_by_email ?? 'an unattributed token'}
                  </p>
                </div>
                <div className="flex shrink-0 gap-2">
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() => inviteMutation.mutate(invitation.email)}
                    disabled={busy}
                    title="Sends a new link and stops the earlier one"
                  >
                    Send again
                  </Button>
                  <Button
                    size="sm"
                    variant="outline"
                    className="border-error/50 text-error hover:bg-error/10"
                    onClick={() => revokeMutation.mutate(invitation.email)}
                    disabled={busy}
                  >
                    Revoke
                  </Button>
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

/** Surface the server's own refusal (a spent budget, a malformed address) over a generic line. */
function errorMessage(err: unknown, fallback: string): string {
  const response = (err as { response?: { data?: { message?: string; error?: string } } })?.response;
  return response?.data?.message ?? response?.data?.error ?? fallback;
}
