// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Coach Home's invited-but-not-joined rows on both clients — which groups still wait on their athlete, and the invite link that waits
// ABOUTME: A row only for a group nobody has joined that holds a member invite still usable: never "invited" for a link that cannot be used

import type { CoachRosterGroup, GroupInvite } from '@pierre/shared-types';

/** The words of a pending-invite row, every one under `coachHome.*`. */
export const COACH_PENDING_INVITE_KEYS = {
  status: 'coachHome.pendingInvite.status',
  why: 'coachHome.pendingInvite.why',
  whyNoExpiry: 'coachHome.pendingInvite.whyNoExpiry',
  copyAgain: 'coachHome.pendingInvite.copyAgain',
  copied: 'coachHome.pendingInvite.copied',
  copyFailed: 'coachHome.pendingInvite.copyFailed',
  after: 'coachHome.pendingInvite.after',
} as const;

/** The coach's groups nobody has joined yet: the ones whose invite may still be waiting. */
export function groupsAwaitingAthlete(groups: readonly CoachRosterGroup[]): CoachRosterGroup[] {
  return groups.filter((group) => group.athlete_count === 0);
}

/**
 * The member invite a group's athlete can still join with, or `null`: active,
 * not past its expiry at `now`, under its use limit. The newest wins, as the
 * one the coach handed out last. A coach invite is not the athlete's.
 */
export function usableMemberInvite(invites: readonly GroupInvite[], now: Date): GroupInvite | null {
  const usable = invites.filter(
    (invite) =>
      invite.kind === 'member' &&
      invite.is_active &&
      (invite.expires_at === null || new Date(invite.expires_at).getTime() > now.getTime()) &&
      (invite.max_uses === null || invite.use_count < invite.max_uses),
  );
  usable.sort((a, b) => b.created_at.localeCompare(a.created_at));
  return usable[0] ?? null;
}
