// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves which coach groups still wait on their athlete and which invite link a pending row may offer
// ABOUTME: Only a member invite that is active, unexpired and under its limit: never a row for a link that cannot be used

import { describe, expect, it } from 'vitest';
import type { CoachRosterGroup, GroupInvite } from '@pierre/shared-types';
import { groupsAwaitingAthlete, usableMemberInvite } from '../src/coachPendingInvites';

const NOW = new Date('2026-11-13T15:00:00Z');

function invite(overrides: Partial<GroupInvite> = {}): GroupInvite {
  return {
    id: 'inv-1',
    group_id: 'g-1',
    tenant_id: 't-1',
    code: 'ABCD2345',
    kind: 'member',
    created_by: 'coach-1',
    expires_at: '2026-12-13T14:40:00Z',
    max_uses: null,
    use_count: 0,
    is_active: true,
    approves_signup: false,
    created_at: '2026-11-13T14:40:00Z',
    ...overrides,
  };
}

describe('groupsAwaitingAthlete', () => {
  it('keeps only the groups nobody has joined', () => {
    const groups: CoachRosterGroup[] = [
      { id: 'g-1', name: 'Léa', athlete_count: 0 },
      { id: 'g-2', name: 'Marc', athlete_count: 1 },
    ];
    expect(groupsAwaitingAthlete(groups).map((g) => g.id)).toEqual(['g-1']);
  });
});

describe('usableMemberInvite', () => {
  it('offers an active, unexpired member invite under its limit', () => {
    expect(usableMemberInvite([invite()], NOW)?.code).toBe('ABCD2345');
  });

  it('offers none for an expired, deactivated, used-up or coach invite', () => {
    expect(usableMemberInvite([invite({ expires_at: '2026-11-13T14:59:59Z' })], NOW)).toBeNull();
    expect(usableMemberInvite([invite({ is_active: false })], NOW)).toBeNull();
    expect(usableMemberInvite([invite({ max_uses: 1, use_count: 1 })], NOW)).toBeNull();
    expect(usableMemberInvite([invite({ kind: 'coach' })], NOW)).toBeNull();
    expect(usableMemberInvite([], NOW)).toBeNull();
  });

  it('offers an invite that never expires, and the newest when several work', () => {
    expect(usableMemberInvite([invite({ expires_at: null })], NOW)?.code).toBe('ABCD2345');
    const older = invite({ id: 'a', code: 'OLDER234', created_at: '2026-11-10T09:00:00Z' });
    const newer = invite({ id: 'b', code: 'NEWER234', created_at: '2026-11-12T09:00:00Z' });
    expect(usableMemberInvite([older, newer], NOW)?.code).toBe('NEWER234');
  });
});
