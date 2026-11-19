// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves the invite's words name the inviter only when they have a name, and say what joining shares only about the coach
// ABOUTME: A coachless group never promises anyone sees anything; a nameless sender is never replaced by a stand-in (carnet#1649, #1650)

import { describe, expect, it } from 'vitest';
import {
  inviteCardNamesGroupBelow,
  inviteCardTitle,
  inviteDeadLink,
  inviteJourneyLine,
  inviteJourneyPromise,
  inviteShares,
  joinStripFull,
  joinStripJoin,
  joinStripTitle,
  type InvitePeople,
} from '../src/inviteCopy';

function invite(overrides: Partial<InvitePeople> = {}): InvitePeople {
  return {
    kind: 'member',
    inviter_name: 'Jeremy B.',
    group_name: 'Morning Riders',
    has_coach: true,
    coach_name: 'Jeremy B.',
    ...overrides,
  };
}

describe('inviteCopy', () => {
  it('names the inviter in the heading, and the group under it', () => {
    expect(inviteCardTitle(invite())).toEqual({ key: 'humanCoach.inviteCard.title', values: { name: 'Jeremy B.' } });
    expect(inviteCardNamesGroupBelow(invite())).toBe(true);
  });

  it('puts the group in the heading when the inviter has no name, never a stand-in', () => {
    const nameless = invite({ inviter_name: null });
    expect(inviteCardTitle(nameless)).toEqual({
      key: 'humanCoach.inviteCard.titleNoName',
      values: { group: 'Morning Riders' },
    });
    expect(inviteCardNamesGroupBelow(nameless)).toBe(false);
    expect(inviteCardTitle(invite({ kind: 'coach', inviter_name: null })).key).toBe(
      'humanCoach.inviteCard.titleCoachNoName',
    );
  });

  it('says what joining shares about the coach, not the inviter', () => {
    expect(inviteShares(invite({ coach_name: 'Ana L.' }))).toEqual({
      key: 'humanCoach.inviteCard.shares',
      values: { name: 'Ana L.' },
    });
  });

  it('promises no sharing in a group with no coach', () => {
    expect(inviteShares(invite({ has_coach: false, coach_name: null })).key).toBe('humanCoach.noneMember');
  });

  it('speaks of the group coach when the coach has no name', () => {
    expect(inviteShares(invite({ coach_name: null })).key).toBe('humanCoach.inviteCard.sharesUnnamedCoach');
  });

  it('tells a coach seat what its coach sees', () => {
    expect(inviteShares(invite({ kind: 'coach', has_coach: false })).key).toBe('humanCoach.inviteCard.sharesCoach');
  });

  it('asks the sender for a new link, by name when there is one', () => {
    expect(inviteDeadLink('expired', 'Jeremy B.')).toEqual({
      key: 'humanCoach.inviteCard.expired',
      values: { name: 'Jeremy B.' },
    });
    expect(inviteDeadLink('used_up', null)).toEqual({ key: 'humanCoach.inviteCard.usedUpNoName', values: {} });
    expect(inviteDeadLink('inactive', null).key).toBe('humanCoach.inviteCard.inactiveNoName');
  });

  it('words the journey and its promise with or without a name', () => {
    expect(inviteJourneyLine(invite()).key).toBe('humanCoach.inviteJourney.line');
    expect(inviteJourneyLine(invite({ inviter_name: null }))).toEqual({
      key: 'humanCoach.inviteJourney.lineNoName',
      values: { group: 'Morning Riders' },
    });
    expect(inviteJourneyPromise(invite(), true).key).toBe('humanCoach.inviteJourney.reviewPromise');
    expect(inviteJourneyPromise(invite({ inviter_name: null }), false).key).toBe(
      'humanCoach.inviteJourney.confirmEmailPromiseNoName',
    );
  });

  it('words the Home strip with or without a name', () => {
    expect(joinStripTitle(invite({ kind: 'coach' })).key).toBe('humanCoach.joinStrip.titleCoach');
    expect(joinStripTitle(invite({ inviter_name: null })).key).toBe('humanCoach.joinStrip.titleNoName');
    expect(joinStripJoin(invite({ inviter_name: null })).key).toBe('humanCoach.joinStrip.joinNoName');
    expect(joinStripFull(invite()).values).toEqual({ name: 'Jeremy B.' });
  });
});
