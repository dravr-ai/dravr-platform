// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the mention and the bind command the post-install hint teaches on both clients
// ABOUTME: A listing with no handle reads as the literal word `handle`, never as an empty name

import { describe, expect, it } from 'vitest';
import { coachAddDraft, coachMention } from '../src/coachDraft';

describe('the post-install hint', () => {
  it('teaches the agent by its handle', () => {
    expect(coachMention('tempo-coach')).toBe('@tempo-coach');
    expect(coachAddDraft('tempo-coach')).toBe('/agent add @tempo-coach');
  });

  it('falls back to the placeholder word when the payload carries no handle', () => {
    expect(coachMention(undefined)).toBe('@handle');
    expect(coachMention('')).toBe('@handle');
    expect(coachAddDraft(undefined)).toBe('/agent add @handle');
  });
});
