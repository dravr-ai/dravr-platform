// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the figures a Home activity row prints, in the athlete's own number notation and duration words
// ABOUTME: Red if a row writes "10.20 km" or "45m 45s" to a French athlete — neither toFixed nor fixed letters follow the language

import { describe, it, expect } from 'vitest';
import { i18n } from '@pierre/i18n';
import { activityFigures } from '../homeFormat';
import { activity } from './homeFixtures';

const tIn = (language: string) => i18n.getFixedT(language);
const t = tIn('en');

describe('activityFigures', () => {
  it('writes the distance with a full stop in English', () => {
    expect(activityFigures(t, activity({ id: 'a' }), 'en')).toEqual(['10.20 km', '1h', '+85 m']);
  });

  it('writes the distance with a decimal comma in French, German, Spanish and Portuguese', () => {
    for (const language of ['fr', 'de', 'es', 'pt']) {
      expect(activityFigures(tIn(language), activity({ id: 'a', distance_meters: 42_000 }), language)[0]).toBe('42,00 km');
    }
  });

  it('prints metres under a kilometre and leaves out a figure the activity does not carry', () => {
    expect(
      activityFigures(tIn('fr'), activity({ id: 'a', distance_meters: 850, elevation_gain_meters: null }), 'fr'),
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
      activityFigures(tIn(language), activity({ id: 'a', duration_seconds: seconds, distance_meters: null, elevation_gain_meters: null }), language);
    expect(figures(2_745)).toEqual([underAnHour]);
    expect(figures(5_410)).toEqual([overAnHour]);
  });
});
