// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the one category -> pillar table on the web and the fallback each of its three readers keeps
// ABOUTME: Pillars take their pillar, recipes nutrition, casing is ignored; the pill falls back to primary, the dot to neutral, the avatar to the primary container

import { describe, expect, it } from 'vitest';
import {
  coachCategoryAvatarClass,
  coachCategoryDotClass,
  coachCategoryPillar,
  coachCategoryTone,
} from '../coachCategory';

describe('coachCategoryPillar', () => {
  it.each([
    ['training', 'activity'],
    ['nutrition', 'nutrition'],
    ['recipes', 'nutrition'],
    ['recovery', 'recovery'],
    ['mobility', 'mobility'],
    ['Training', 'activity'],
    ['MOBILITY', 'mobility'],
  ])('places %s under %s', (category, pillar) => {
    expect(coachCategoryPillar(category)).toBe(pillar);
  });

  it.each(['analysis', 'custom', 'strength', ''])('places %s under no pillar', (category) => {
    expect(coachCategoryPillar(category)).toBeNull();
  });
});

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
    expect(coachCategoryAvatarClass('recovery')).toBe('bg-recovery/15 text-on-recovery-container');
    expect(coachCategoryAvatarClass('custom')).toBe('bg-primary-container text-on-primary-container');
  });
});
