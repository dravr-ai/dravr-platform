// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the one category -> pillar table both clients read
// ABOUTME: Pillars take their pillar, recipes nutrition, casing is ignored, and a category with no pillar answers null

import { describe, expect, it } from 'vitest';
import { coachCategoryPillar } from '../src/coaches';

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

  it('answers null for a key an object carries by inheritance', () => {
    expect(coachCategoryPillar('constructor')).toBeNull();
    expect(coachCategoryPillar('toString')).toBeNull();
  });
});
