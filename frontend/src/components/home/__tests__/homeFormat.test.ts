// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the figures a Home activity row prints, in the athlete's own number notation and duration words
// ABOUTME: Red if a row writes "10.20 km" or "45m 45s" to a French athlete — neither toFixed nor fixed letters follow the language

import { describe, it, expect } from 'vitest';
import { i18n } from '@pierre/i18n';
import type { PlanDay, PlanDayLookup, PlanWeek } from '@pierre/shared-types';
import { activityFigures, planDayRouteDraft } from '../homeFormat';
import { activity } from './homeFixtures';

const tIn = (language: string) => i18n.getFixedT(language);
const t = tIn('en');

describe('activityFigures', () => {
  it('writes the distance with a full stop in English', () => {
    expect(activityFigures(t, activity({ id: 'a' }), 'en', 'metric')).toEqual(['10.20 km', '1h', '+85 m']);
  });

  it('writes the distance with a decimal comma in French, German, Spanish and Portuguese', () => {
    for (const language of ['fr', 'de', 'es', 'pt']) {
      expect(activityFigures(tIn(language), activity({ id: 'a', distance_meters: 42_000 }), language, 'metric')[0]).toBe('42,00 km');
    }
  });

  it('prints metres under a kilometre and leaves out a figure the activity does not carry', () => {
    expect(
      activityFigures(tIn('fr'), activity({ id: 'a', distance_meters: 850, elevation_gain_meters: null }), 'fr', 'metric'),
    ).toEqual(['850 m', '1 h']);
  });

  it.each([
    ['en', '45m 45s', '1h 30m 10s'],
    ['fr', '45 min 45 s', '1 h 30 min 10 s'],
    ['de', '45 Min. 45 Sek.', '1 Std. 30 Min. 10 Sek.'],
    ['es', '45 min 45 s', '1 h 30 min 10 s'],
    ['pt', '45 min 45 s', '1 h 30 min 10 s'],
  ])('writes the time in the words of %s', (language, underAnHour, overAnHour) => {
    const figures = (seconds: number) =>
      activityFigures(tIn(language), activity({ id: 'a', duration_seconds: seconds, distance_meters: null, elevation_gain_meters: null }), language, 'metric');
    expect(figures(2_745)).toEqual([underAnHour]);
    expect(figures(5_410)).toEqual([overAnHour]);
  });
});

describe('planDayRouteDraft', () => {
  const week: PlanWeek = { week_start: '2026-09-21', focus: 'threshold', current: true, days: [] };
  const intervals: PlanDay = {
    date: '2026-09-24',
    sport: 'run',
    workout: '8 × 1 km',
    intensity: 'Z4',
    rest: false,
    steps: [
      { label: 'Warm-up', duration_seconds: 900, distance_meters: 3000, target_zone: 'Z1' },
      { label: 'Interval', duration_seconds: 240, distance_meters: 1000, target_zone: 'Z4', repeat: 8 },
      { label: 'Float', duration_seconds: 120, distance_meters: 250, target_zone: 'Z1', repeat: 8 },
      { label: 'Cool-down', duration_seconds: 600, distance_meters: 2000, target_zone: 'Z1' },
    ],
  };
  const session = (day: PlanDay): PlanDayLookup => ({ kind: 'session', day, week });

  it('carries the session distance when every step has one', () => {
    expect(planDayRouteDraft(t, 'en', '2026-09-24', session(intervals), 'metric')).toBe(
      'Suggest a 15 km route close to where I am for my session on Thursday, September 24: 8 × 1 km',
    );
  });

  it('writes a fractional distance in the notation of the language', () => {
    const day = { ...intervals, steps: [{ label: 'Run', duration_seconds: 3600, distance_meters: 12_540, target_zone: 'Z2' }] };
    expect(planDayRouteDraft(tIn('fr'), 'fr', '2026-09-24', session(day), 'metric')).toBe(
      "Propose-moi un parcours de 12,5 km près d'où je suis pour ma séance du jeudi 24 septembre : 8 × 1 km",
    );
  });

  it('names the distance in miles for an athlete on imperial units (carnet#835)', () => {
    expect(planDayRouteDraft(t, 'en', '2026-09-24', session(intervals), 'imperial')).toBe(
      'Suggest a 9.3 mi route close to where I am for my session on Thursday, September 24: 8 × 1 km',
    );
  });

  it('names no distance for a session set by time', () => {
    const day = { ...intervals, steps: [{ label: 'Tempo', duration_seconds: 1500, target_zone: 'Z3' }] };
    expect(planDayRouteDraft(t, 'en', '2026-09-24', session(day), 'metric')).toBe(
      'Suggest a route close to where I am for my session on Thursday, September 24: 8 × 1 km',
    );
  });

  it('has no draft for a rest day, an uncovered day, or a sport with no route', () => {
    expect(planDayRouteDraft(t, 'en', '2026-09-24', { kind: 'uncovered' }, 'metric')).toBeNull();
    expect(planDayRouteDraft(t, 'en', '2026-09-24', { kind: 'rest', day: { ...intervals, rest: true }, week }, 'metric')).toBeNull();
    expect(planDayRouteDraft(t, 'en', '2026-09-24', session({ ...intervals, sport: 'swim' }), 'metric')).toBeNull();
  });
});
