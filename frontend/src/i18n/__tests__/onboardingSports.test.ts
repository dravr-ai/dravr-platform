// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the value/label split for onboarding sports — wire value English, label translated
// ABOUTME: The write side was split without the read side, so French prose rendered ", surtout Running."

import { describe, it, expect } from 'vitest';
import {
  ONBOARDING_SPORTS,
  SPORT_LABEL_KEY,
  isOnboardingSport,
} from '@pierre/shared-constants';

/**
 * The bug this guards against was a HALF migration.
 *
 * The chip was changed to render `t(SPORT_LABEL_KEY[sport])` while saving the
 * English value — correct. The read-back on the next screen still interpolated
 * the stored value straight into a translated sentence, so a French athlete
 * who tapped "Course à pied" was told ", surtout Running."
 *
 * A half-split is worse than none: it looks finished.
 */
describe('onboarding sports: the value and the label are different things', () => {
  it('keeps the wire values English', () => {
    // These are stored on the profile and read back by the coach. Translating
    // them would change what gets saved.
    expect([...ONBOARDING_SPORTS]).toEqual([
      'Running',
      'Cycling',
      'Swimming',
      'Triathlon',
      'Strength',
      'Hiking',
    ]);
  });

  it('gives every wire value a label key', () => {
    // A missing entry renders the raw key to the athlete, which is the same
    // class of defect one step further along.
    for (const sport of ONBOARDING_SPORTS) {
      expect(SPORT_LABEL_KEY[sport]).toBeDefined();
      expect(SPORT_LABEL_KEY[sport]).toMatch(/^app\.sport[A-Z]/);
    }
  });

  it('has no label key for a sport that is not a wire value', () => {
    expect(Object.keys(SPORT_LABEL_KEY).sort()).toEqual([...ONBOARDING_SPORTS].sort());
  });

  it('recognises a stored value, and refuses one it did not store', () => {
    expect(isOnboardingSport('Running')).toBe(true);
    // The profile field is free text — the screen lets an athlete type
    // anything — so the read-back must be able to say "not one of mine" and
    // fall back to what they actually wrote.
    expect(isOnboardingSport('Bouldering')).toBe(false);
    expect(isOnboardingSport('running')).toBe(false);
    expect(isOnboardingSport('')).toBe(false);
  });

  it('narrows the type, so a lookup on an unchecked string will not compile', () => {
    const fromServer: string = 'Cycling';
    if (isOnboardingSport(fromServer)) {
      // Only reachable because the guard narrowed it; this line failing to
      // compile is the point of the guard existing.
      expect(SPORT_LABEL_KEY[fromServer]).toBe('app.sportCycling');
    } else {
      throw new Error('guard should have accepted a known wire value');
    }
  });
});
