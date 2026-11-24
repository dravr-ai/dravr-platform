// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the one group invite link rule: the link built for a code on an origin, and the code read back from its path
// ABOUTME: What one side builds the other reads, an escaped code included; any other path names no code

import { describe, expect, it } from 'vitest';
import { groupInviteCodeFromPath, groupInviteLink } from '../src/invite-link';

describe('groupInviteLink', () => {
  it('builds the join link on the origin it is given', () => {
    expect(groupInviteLink('https://app.dravr.ai', 'ABCD2345')).toBe('https://app.dravr.ai/groups/join/ABCD2345');
    expect(groupInviteLink('http://localhost:3000', 'ABCD2345')).toBe('http://localhost:3000/groups/join/ABCD2345');
  });

  it('escapes a code, and the path it builds reads back as that code', () => {
    const link = groupInviteLink('https://app.dravr.ai', 'AB CD/é');
    expect(link).toBe('https://app.dravr.ai/groups/join/AB%20CD%2F%C3%A9');
    expect(groupInviteCodeFromPath(new URL(link).pathname)).toBe('AB CD/é');
  });
});

describe('groupInviteCodeFromPath', () => {
  it('reads the code a /groups/join/<code> path names, decoded', () => {
    expect(groupInviteCodeFromPath('/groups/join/MRT2026X')).toBe('MRT2026X');
    expect(groupInviteCodeFromPath('/groups/join/AB%20CD')).toBe('AB CD');
  });

  it('names no code for a malformed escape, a longer path, an empty code or any other path', () => {
    expect(groupInviteCodeFromPath('/groups/join/%E0%A4%A')).toBeNull();
    expect(groupInviteCodeFromPath('/groups/join/MRT2026X/extra')).toBeNull();
    expect(groupInviteCodeFromPath('/groups/join/')).toBeNull();
    expect(groupInviteCodeFromPath('/groups/join/%20')).toBeNull();
    expect(groupInviteCodeFromPath('/groups/join')).toBeNull();
    expect(groupInviteCodeFromPath('/')).toBeNull();
  });
});
