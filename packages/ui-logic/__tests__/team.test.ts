// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the athlete's team as Home draws it on both clients: assigned agents only, the lead apart, the others by title
// ABOUTME: A lead that is not on the team marks nobody, and the flat order puts the lead first

import { describe, expect, it } from 'vitest';
import type { Agent } from '@pierre/shared-types';
import { orderTeam, teamInOrder } from '../src/team';

function agent(id: string, title: string, assigned: 'yes' | 'no' | 'unset' = 'yes'): Agent {
  return {
    id,
    title,
    description: null,
    system_prompt: '',
    category: 'training',
    tags: [],
    token_count: 0,
    is_favorite: false,
    use_count: 0,
    last_used_at: null,
    created_at: '2026-08-20T10:00:00Z',
    updated_at: '2026-08-20T10:00:00Z',
    is_system: true,
    ...(assigned === 'unset' ? {} : { is_assigned: assigned === 'yes' }),
  };
}

const AGENTS = [
  agent('z', 'Zone Coach'),
  agent('off', 'Aardvark', 'no'),
  agent('unset', 'Unset', 'unset'),
  agent('lead', 'Marathon Coach'),
  agent('a', 'Alpine Coach'),
];

describe('orderTeam', () => {
  it('keeps the assigned agents only, the lead apart and the others by title', () => {
    const { lead, members } = orderTeam(AGENTS, 'lead');
    expect(lead?.id).toBe('lead');
    expect(members.map((member) => member.id)).toEqual(['a', 'z']);
  });

  it('marks nobody when the personal thread has no agent, or one that is not on the team', () => {
    expect(orderTeam(AGENTS, null).lead).toBeNull();
    const offTeam = orderTeam(AGENTS, 'off');
    expect(offTeam.lead).toBeNull();
    expect(offTeam.members.map((member) => member.id)).toEqual(['a', 'lead', 'z']);
  });

  it('answers an empty team for an athlete with none', () => {
    expect(orderTeam([agent('off', 'Aardvark', 'no')], 'off')).toEqual({ lead: null, members: [] });
  });
});

describe('teamInOrder', () => {
  it('reads lead first, then the others', () => {
    expect(teamInOrder(orderTeam(AGENTS, 'lead')).map((member) => member.id)).toEqual(['lead', 'a', 'z']);
  });

  it('is the members alone when nobody leads', () => {
    expect(teamInOrder(orderTeam(AGENTS, null)).map((member) => member.id)).toEqual(['a', 'lead', 'z']);
  });
});
