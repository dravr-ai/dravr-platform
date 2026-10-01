// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home tab's words and figures — civil dates never shifted by the device zone, durations in each language's units, the chat drafts
// ABOUTME: Real i18next strings in English and French, so a draft that dropped its date or sport would fail here

import { i18n } from '@pierre/i18n';

import { ACTIVITIES, TEMPO_DAY } from '../integration/app/helpers/homeFixtures';
import {
  activityNaming,
  activityFigures,
  civilDayOfMonth,
  civilLongDate,
  civilWeekdayNarrow,
  planDayDraft,
  sportLabel,
} from '../src/screens/home/homeFormat';

const t = (key: string, options?: Record<string, unknown>) => i18n.t(key, options);
const tIn = (language: string) => (key: string, options?: Record<string, unknown>) =>
  i18n.getFixedT(language)(key, options);
const tFr = tIn('fr');

describe('civil dates', () => {
  it('names a plan day on the calendar, in the app language', () => {
    expect(civilLongDate('2026-09-24', 'en')).toBe('Thursday, September 24');
    expect(civilLongDate('2026-09-23', 'fr')).toBe('mercredi 23 septembre');
    expect(civilWeekdayNarrow('2026-09-21', 'en')).toBe('M');
    expect(civilDayOfMonth('2026-09-07')).toBe(7);
  });

  it('never moves a civil date to the day before, whatever the device zone', () => {
    // Midnight UTC of the 1st is still the 31st west of Greenwich; the date
    // is formatted on the calendar, not at an instant.
    expect(civilLongDate('2026-10-01', 'en')).toBe('Thursday, October 1');
  });

  it('refuses a string that is not a date', () => {
    expect(() => civilLongDate('24/09/2026', 'en')).toThrow(RangeError);
  });
});

describe('durations and figures', () => {
  it('prints only the figures the provider reported', () => {
    expect(activityFigures(t, ACTIVITIES[0], 'en')).toEqual(['92.0 km', '3h 41m 5s', '+850 m']);
    expect(activityFigures(t, ACTIVITIES[3], 'en')).toEqual(['30.0 km', '1h']);
    expect(activityFigures(t, ACTIVITIES[4], 'en')).toEqual(['45m']);
  });

  // `toFixed` writes a full stop in every language; a French athlete read
  // "92.0 km" until the row's figures went through Intl.NumberFormat.
  it('writes the distance in the athlete\'s notation, a decimal comma in French', () => {
    expect(activityFigures(tFr, ACTIVITIES[0], 'fr')).toEqual(['92,0 km', '3 h 41 min 5 s', '+850 m']);
    expect(activityFigures(tIn('de'), ACTIVITIES[3], 'de')[0]).toBe('30,0 km');
  });

  // The row printed "45m 45s" in every language; each writes its own units.
  it.each([
    ['en', '45m 45s', '1h 30m 10s'],
    ['fr', '45 min 45 s', '1 h 30 min 10 s'],
    ['de', '45 Min. 45 Sek.', '1 Std. 30 Min. 10 Sek.'],
    ['es', '45 min 45 s', '1 h 30 min 10 s'],
    ['pt', '45 min 45 s', '1 h 30 min 10 s'],
  ])('writes the duration in the words of %s', (language, underAnHour, overAnHour) => {
    const figures = (seconds: number) =>
      activityFigures(
        tIn(language),
        { ...ACTIVITIES[4], duration_seconds: seconds, distance_meters: null, elevation_gain_meters: null },
        language,
      );
    expect(figures(2_745)).toEqual([underAnHour]);
    expect(figures(5_410)).toEqual([overAnHour]);
  });

  it('names a sport the vocabulary knows, and shows the provider spelling of one it does not', () => {
    expect(sportLabel(t, 'virtual_ride')).toBe('Indoor ride');
    expect(sportLabel(t, 'Underwater Hockey')).toBe('Underwater Hockey');
    // A plan day's sport goes through the same label, so a French athlete reads Course.
    expect(sportLabel(tFr, 'run')).toBe('Course');
  });
});

describe('chat drafts', () => {
  it('asks about a session with its date and workout', () => {
    expect(planDayDraft(t, TEMPO_DAY, 'en')).toBe('Walk me through my session on Thursday, September 24: Tempo run');
    expect(planDayDraft(tFr, TEMPO_DAY, 'fr')).toContain('jeudi 24 septembre');
    expect(planDayDraft(tFr, TEMPO_DAY, 'fr')).toContain('Tempo run');
  });

  it('asks why a rest day is one', () => {
    const rest = { ...TEMPO_DAY, date: '2026-09-22', workout: 'Rest', rest: true };
    expect(planDayDraft(t, rest, 'en')).toBe('Why is Tuesday, September 22 a rest day in my plan?');
  });

  it('names an activity by its title and day, and by its sport when it has no title', () => {
    expect(activityNaming(t, ACTIVITIES[0], 'en')).toEqual({ name: ACTIVITIES[0].name, date: 'Sunday, September 20' });
    expect(activityNaming(tFr, { ...ACTIVITIES[0], name: '  ' }, 'fr')).toEqual({
      name: 'Vélo',
      date: 'dimanche 20 septembre',
    });
  });
});
