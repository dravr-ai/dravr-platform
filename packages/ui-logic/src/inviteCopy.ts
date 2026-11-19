// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The words of a group invite on both clients — the card, the line through onboarding, the promise, the Home strip (carnet#1649, #1650)
// ABOUTME: Names the inviter when they have a name, and says what joining shares only about the group's coach, never the inviter

import type { GroupInviteKind, GroupInvitePreviewStatus } from '@pierre/shared-types';

/** What the words of an invite are chosen from: the card's or the hold's fields. */
export interface InvitePeople {
  kind: GroupInviteKind;
  /** Who sent the link; null when they have no display name */
  inviter_name: string | null;
  group_name: string;
  /** Whether the group has a human coach */
  has_coach: boolean;
  /** That coach's name; null without a coach or a name */
  coach_name: string | null;
}

/** One line to render: an i18n key and the values `t` fills it with. */
export interface InviteCopyLine {
  key: string;
  values: Record<string, string>;
}

const line = (key: string, values: Record<string, string> = {}): InviteCopyLine => ({ key, values });

/**
 * The card's heading: « Jeremy B. t'invite », or the group's name in it when
 * the inviter has no name. A coach seat reads as an invite to coach.
 */
export function inviteCardTitle(invite: InvitePeople): InviteCopyLine {
  const asCoach = invite.kind === 'coach';
  if (invite.inviter_name === null) {
    return line(asCoach ? 'humanCoach.inviteCard.titleCoachNoName' : 'humanCoach.inviteCard.titleNoName', {
      group: invite.group_name,
    });
  }
  return line(asCoach ? 'humanCoach.inviteCard.titleCoach' : 'humanCoach.inviteCard.title', {
    name: invite.inviter_name,
  });
}

/**
 * Whether the card shows the group's name under its heading: a nameless
 * inviter's heading already names the group.
 */
export function inviteCardNamesGroupBelow(invite: InvitePeople): boolean {
  return invite.inviter_name !== null;
}

/**
 * What joining shares, said about the group's human coach (ADR-002), never
 * the inviter: the coach by name, the group's coach when they have none, and
 * a group with no coach says so instead of promising anyone sees anything. A
 * coach seat says what its coach will see.
 */
export function inviteShares(invite: InvitePeople): InviteCopyLine {
  if (invite.kind === 'coach') return line('humanCoach.inviteCard.sharesCoach');
  if (!invite.has_coach) return line('humanCoach.noneMember');
  if (invite.coach_name === null) return line('humanCoach.inviteCard.sharesUnnamedCoach');
  return line('humanCoach.inviteCard.shares', { name: invite.coach_name });
}

/** Why a dead link no longer works, asking its sender for a new one. */
export function inviteDeadLink(
  status: Exclude<GroupInvitePreviewStatus, 'live'>,
  inviterName: string | null,
): InviteCopyLine {
  const base = { expired: 'expired', used_up: 'usedUp', inactive: 'inactive' }[status];
  return inviterName === null
    ? line(`humanCoach.inviteCard.${base}NoName`)
    : line(`humanCoach.inviteCard.${base}`, { name: inviterName });
}

/** The thin line naming the invite above the onboarding steps. */
export function inviteJourneyLine(invite: Pick<InvitePeople, 'inviter_name' | 'group_name'>): InviteCopyLine {
  return invite.inviter_name === null
    ? line('humanCoach.inviteJourney.lineNoName', { group: invite.group_name })
    : line('humanCoach.inviteJourney.line', { name: invite.inviter_name, group: invite.group_name });
}

/**
 * What the waiting screen promises: the join once the address is confirmed,
 * or once an administrator approves the account when sign-up is reviewed.
 */
export function inviteJourneyPromise(
  invite: Pick<InvitePeople, 'inviter_name' | 'group_name'>,
  reviewed: boolean,
): InviteCopyLine {
  const base = reviewed ? 'reviewPromise' : 'confirmEmailPromise';
  return invite.inviter_name === null
    ? line(`humanCoach.inviteJourney.${base}NoName`, { group: invite.group_name })
    : line(`humanCoach.inviteJourney.${base}`, { name: invite.inviter_name, group: invite.group_name });
}

/** The Home strip's sentence: who invites the athlete, to which group. */
export function joinStripTitle(invite: InvitePeople): InviteCopyLine {
  const suffix = invite.kind === 'coach' ? 'titleCoach' : 'title';
  return invite.inviter_name === null
    ? line(`humanCoach.joinStrip.${suffix}NoName`, { group: invite.group_name })
    : line(`humanCoach.joinStrip.${suffix}`, { name: invite.inviter_name, group: invite.group_name });
}

/** The Home strip's one filled button. */
export function joinStripJoin(invite: Pick<InvitePeople, 'inviter_name'>): InviteCopyLine {
  return invite.inviter_name === null
    ? line('humanCoach.joinStrip.joinNoName')
    : line('humanCoach.joinStrip.join', { name: invite.inviter_name });
}

/** The Home strip's word for a full group, offering a retry. */
export function joinStripFull(invite: Pick<InvitePeople, 'inviter_name'>): InviteCopyLine {
  return invite.inviter_name === null
    ? line('humanCoach.joinStrip.fullNoName')
    : line('humanCoach.joinStrip.full', { name: invite.inviter_name });
}
