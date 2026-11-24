// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the class each of the web's three category readers derives from the shared pillar table, and its fallback
// ABOUTME: The pill falls back to primary, the dot to neutral, the avatar to the primary container

import { describe, expect, it } from 'vitest';
import {
  coachCategoryAvatarClass,
  coachCategoryDotClass,
  coachCategoryTone,
} from '../coachCategory';
import { BADGE_TONE } from '../ui/badgeTone';

describe('the readers derive their class from the pillar and keep their own fallback', () => {
  it('draws an admin pill in the pillar tone, or primary', () => {
    expect(coachCategoryTone('training')).toBe('activity');
    expect(coachCategoryTone('custom')).toBe('primary');
  });

  it('fills a store dot with the pillar hue, or neutral outline-variant', () => {
    expect(coachCategoryDotClass('recipes')).toBe('bg-nutrition');
    expect(coachCategoryDotClass('custom')).toBe('bg-outline-variant');
  });

  it('tints an avatar with the pillar pair, or the primary container pair', () => {
    expect(coachCategoryAvatarClass('recovery')).toBe(BADGE_TONE.recovery.tone);
    expect(coachCategoryAvatarClass('custom')).toBe(BADGE_TONE.accent.tone);
  });
});
