// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves who lands on the pending coach Home, who keeps their own Home beside it, and that its words exist in five catalogues
// ABOUTME: The routing reads coach access and the onboarding answer alone; every new key sits under humanCoach, where the human coach is named

import { describe, expect, it } from 'vitest';
import de from '../../i18n/src/locales/de/translation.json';
import en from '../../i18n/src/locales/en/translation.json';
import es from '../../i18n/src/locales/es/translation.json';
import fr from '../../i18n/src/locales/fr/translation.json';
import pt from '../../i18n/src/locales/pt/translation.json';
import { COACH_SETUP_KEYS, coachTrainsToo, landsOnPendingCoachHome } from '../src/coach-setup';

function lookup(catalogue: unknown, key: string): unknown {
  return key.split('.').reduce<unknown>(
    (node, part) => (node && typeof node === 'object' ? (node as Record<string, unknown>)[part] : undefined),
    catalogue,
  );
}

describe('landsOnPendingCoachHome', () => {
  it('lands a coach without coach access on the coach-shaped Home', () => {
    expect(landsOnPendingCoachHome({ coachesOthers: true, managesRoster: false })).toBe(true);
  });

  it('leaves a coach who holds coach access to their coach Home, and an athlete to theirs', () => {
    expect(landsOnPendingCoachHome({ coachesOthers: true, managesRoster: true })).toBe(false);
    expect(landsOnPendingCoachHome({ coachesOthers: false, managesRoster: false })).toBe(false);
  });
});

describe('coachTrainsToo', () => {
  it('reads a coach who answered "I coach others" alone as not training', () => {
    expect(coachTrainsToo([{ step_id: 'parq', status: 'not_applicable' }])).toBe(false);
    expect(coachTrainsToo([{ step_id: 'about_you', status: 'not_applicable' }])).toBe(false);
  });

  it('reads any other coach as training too, so no athlete nudge is taken from them', () => {
    expect(coachTrainsToo([])).toBe(true);
    expect(coachTrainsToo(undefined)).toBe(true);
    expect(coachTrainsToo([{ step_id: 'parq', status: 'skipped' }])).toBe(true);
    expect(coachTrainsToo([{ step_id: 'parq', status: 'complete' }])).toBe(true);
  });
});

describe('pending coach Home words', () => {
  const keys = Object.values(COACH_SETUP_KEYS);

  it.each([
    ['en', en],
    ['fr', fr],
    ['es', es],
    ['de', de],
    ['pt', pt],
  ])('resolves every key in the %s catalogue', (_locale, catalogue) => {
    for (const key of keys) {
      expect(typeof lookup(catalogue, key), key).toBe('string');
    }
  });

  it('keeps every new key under humanCoach.pendingHome (ADR-026)', () => {
    const fresh = keys.filter((key) => key.includes('pendingHome'));
    expect(fresh.length).toBeGreaterThan(0);
    for (const key of fresh) {
      expect(key.startsWith('humanCoach.pendingHome.')).toBe(true);
    }
  });
});
