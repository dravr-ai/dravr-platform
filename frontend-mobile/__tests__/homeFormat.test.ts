// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home tab's words and figures — civil dates never shifted by the device zone, watch-style durations, the chat drafts
// ABOUTME: Real i18next strings in English and French, so a draft that dropped its date or sport would fail here

import { i18n } from '@pierre/i18n';

import { ACTIVITIES, TEMPO_DAY } from '../integration/app/helpers/homeFixtures';
import {
  activityDraft,
  activityFigures,
  civilDayOfMonth,
  civilLongDate,
  civilWeekdayNarrow,
  planDayDraft,
  sportLabel,
} from '../src/screens/home/homeFormat';

const t = (key: string, options?: Record<string, unknown>) => i18n.t(key, options);
const tFr = (key: string, options?: Record<string, unknown>) => i18n.getFixedT('fr')(key, options);

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
    expect(activityFigures(ACTIVITIES[0])).toEqual(['92.0 km', '3h 41m 5s', '+850 m']);
    expect(activityFigures(ACTIVITIES[3])).toEqual(['30.0 km', '1h']);
    expect(activityFigures(ACTIVITIES[4])).toEqual(['45m']);
  });

  it('names a sport the vocabulary knows, and shows the provider spelling of one it does not', () => {
    expect(sportLabel(t, 'virtual_ride')).toBe('Indoor ride');
    expect(sportLabel(t, 'Underwater Hockey')).toBe('Underwater Hockey');
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

  it('asks about an activity with its day and sport', () => {
    expect(activityDraft(t, ACTIVITIES[0], 'en')).toBe('Analyze my activity from Sunday, September 20 (Ride)');
    expect(activityDraft(tFr, ACTIVITIES[0], 'fr')).toBe('Analyse mon activité du dimanche 20 septembre (Vélo)');
  });
});
