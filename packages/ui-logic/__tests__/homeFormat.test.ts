// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests Home's words and figures, shared by both clients — row and volume distances, durations, dates, the plan drafts
// ABOUTME: Read through the app's own catalogues, so a row that printed "10.0 km" on one client and "10.00 km" on the other fails here

import { describe, it, expect } from 'vitest';
import type { CalendarDay, HomeActivity, PlanDay, PlanDayLookup, PlanWeek } from '@pierre/shared-types';
import en from '../../i18n/src/locales/en/translation.json';
import fr from '../../i18n/src/locales/fr/translation.json';
import de from '../../i18n/src/locales/de/translation.json';
import es from '../../i18n/src/locales/es/translation.json';
import pt from '../../i18n/src/locales/pt/translation.json';
import {
  activityNaming,
  activityRowFigures,
  calendarDayLabel,
  civilDayOfMonth,
  civilWeekdayNarrow,
  DRAFT_DATE,
  formatCivilDate,
  formatSyncTime,
  planDayDraft,
  planDayRouteDraft,
  sportLabel,
  volumeMetricText,
  type Translate,
} from '../src/homeFormat';

const CATALOGUES = { en, fr, de, es, pt } as const;
type Language = keyof typeof CATALOGUES;

/** The app's own catalogue for `language`, each `{{name}}` filled in as i18next fills it. */
function translator(language: Language): Translate {
  return (key, options = {}) => {
    const template = key.split('.').reduce<unknown>(
      (node, part) => (node === undefined ? undefined : (node as Record<string, unknown>)[part]),
      CATALOGUES[language],
    );
    if (typeof template !== 'string') {
      if (typeof options.defaultValue === 'string') return options.defaultValue;
      throw new Error(`${language} has no ${key}`);
    }
    return template.replace(/\{\{(\w+)\}\}/g, (_, name: string) => String(options[name]));
  };
}

const t = translator('en');

function activity(overrides: Partial<HomeActivity> = {}): HomeActivity {
  return {
    id: 'ride-1',
    provider: 'strava',
    name: 'Sunday long ride',
    sport_type: 'ride',
    start_date: '2026-09-20T09:00:00Z',
    duration_seconds: 3_600,
    distance_meters: 10_000,
    elevation_gain_meters: 85,
    has_gps: true,
    summary_polyline: null,
    attribution: null,
    ...overrides,
  };
}

describe('activityRowFigures — one distance rule for a row, the volume and the activity view', () => {
  it('prints a 10 km activity with two decimals, as its view does', () => {
    expect(activityRowFigures(t, activity(), 'en', 'metric')).toEqual(['10.00 km', '1h', '+85 m']);
  });

  it('prints metres under a kilometre rather than a fraction of one', () => {
    expect(activityRowFigures(t, activity({ distance_meters: 850, elevation_gain_meters: null }), 'en', 'metric')).toEqual([
      '850 m',
      '1h',
    ]);
  });

  it('writes a decimal comma in French, German, Spanish and Portuguese', () => {
    for (const language of ['fr', 'de', 'es', 'pt'] as const) {
      expect(activityRowFigures(translator(language), activity({ distance_meters: 42_000 }), language, 'metric')[0]).toBe(
        '42,00 km',
      );
    }
  });

  it('prints miles and feet for an athlete on imperial units (carnet#835)', () => {
    expect(activityRowFigures(t, activity({ distance_meters: 92_000, elevation_gain_meters: 850 }), 'en', 'imperial')).toEqual([
      '57.17 mi',
      '1h',
      '+2789 ft',
    ]);
  });

  it.each([
    ['en', '45m 45s', '1h 30m 10s'],
    ['fr', '45 min 45 s', '1 h 30 min 10 s'],
    ['de', '45 Min. 45 Sek.', '1 Std. 30 Min. 10 Sek.'],
    ['es', '45 min 45 s', '1 h 30 min 10 s'],
    ['pt', '45 min 45 s', '1 h 30 min 10 s'],
  ] as const)('writes the time in the words of %s', (language, underAnHour, overAnHour) => {
    const figures = (seconds: number) =>
      activityRowFigures(
        translator(language),
        activity({ duration_seconds: seconds, distance_meters: null, elevation_gain_meters: null }),
        language,
        'metric',
      );
    expect(figures(2_745)).toEqual([underAnHour]);
    expect(figures(5_410)).toEqual([overAnHour]);
  });
});

describe('volumeMetricText', () => {
  it('prints a week distance the way a row prints it, and a week of seconds as a duration in whole minutes', () => {
    expect(volumeMetricText(t, 'distance', 10_000, 'en', 'metric')).toBe('10.00 km');
    expect(volumeMetricText(t, 'distance', 850, 'en', 'metric')).toBe('850 m');
    expect(volumeMetricText(t, 'time', 5_410, 'en', 'metric')).toBe('1h 30m');
  });
});

describe('sportLabel', () => {
  it('names a sport the vocabulary knows, and shows the provider spelling of one it does not', () => {
    expect(sportLabel(t, 'virtual_ride')).toBe('Indoor ride');
    expect(sportLabel(t, 'Underwater Hockey')).toBe('Underwater Hockey');
    expect(sportLabel(translator('fr'), 'run')).toBe('Course');
  });
});

describe('civil dates', () => {
  it('names a plan day on the calendar, never moved to the day before by the device zone', () => {
    expect(formatCivilDate('2026-09-24', 'en', DRAFT_DATE)).toBe('Thursday, September 24');
    expect(formatCivilDate('2026-10-01', 'en', DRAFT_DATE)).toBe('Thursday, October 1');
    expect(formatCivilDate('2026-09-23', 'fr', DRAFT_DATE)).toBe('mercredi 23 septembre');
    expect(civilWeekdayNarrow('2026-09-21', 'en')).toBe('M');
    expect(civilDayOfMonth('2026-09-07')).toBe(7);
  });

  it('gives back a string that is not a calendar day unchanged rather than a wrong date', () => {
    expect(formatCivilDate('24/09/2026', 'en', DRAFT_DATE)).toBe('24/09/2026');
  });

  it('reads the sync time on a 24-hour clock', () => {
    expect(formatSyncTime('2026-09-20T00:05:00Z', 'en').replace(/\s/g, ' ')).not.toMatch(/24:05|12:05 AM/);
  });
});

describe('the chat drafts a plan day opens', () => {
  const week: PlanWeek = { week_start: '2026-09-21', focus: 'threshold', current: true, days: [] };
  const intervals: PlanDay = {
    date: '2026-09-24',
    sport: 'run',
    workout: '8 × 1 km',
    intensity: 'Z4',
    duration_min: 55,
    rest: false,
    steps: [
      { label: 'Warm-up', duration_seconds: 900, distance_meters: 3000, target_zone: 'Z1' },
      { label: 'Interval', duration_seconds: 240, distance_meters: 1000, target_zone: 'Z4', repeat: 8 },
      { label: 'Float', duration_seconds: 120, distance_meters: 250, target_zone: 'Z1', repeat: 8 },
      { label: 'Cool-down', duration_seconds: 600, distance_meters: 2000, target_zone: 'Z1' },
    ],
  };
  const session = (day: PlanDay): PlanDayLookup => ({ kind: 'session', day, week });

  it('asks about a session with its date and workout, and why a rest day is one', () => {
    expect(planDayDraft(t, 'en', '2026-09-24', session(intervals))).toBe(
      'Walk me through my session on Thursday, September 24: 8 × 1 km',
    );
    expect(planDayDraft(t, 'en', '2026-09-22', { kind: 'rest', day: { ...intervals, rest: true }, week })).toBe(
      'Why is Tuesday, September 22 a rest day in my plan?',
    );
  });

  it('has no draft for a day the plan does not cover', () => {
    expect(planDayDraft(t, 'en', '2026-09-24', { kind: 'uncovered' })).toBeNull();
  });

  it('carries the session distance in the route draft when every step has one', () => {
    expect(planDayRouteDraft(t, 'en', '2026-09-24', session(intervals), 'metric')).toBe(
      'Suggest a 15 km route close to where I am for my session on Thursday, September 24: 8 × 1 km',
    );
    expect(planDayRouteDraft(t, 'en', '2026-09-24', session(intervals), 'imperial')).toBe(
      'Suggest a 9.3 mi route close to where I am for my session on Thursday, September 24: 8 × 1 km',
    );
  });

  it('writes a fractional route distance in the notation of the language', () => {
    const day = { ...intervals, steps: [{ label: 'Run', duration_seconds: 3600, distance_meters: 12_540, target_zone: 'Z2' }] };
    expect(planDayRouteDraft(translator('fr'), 'fr', '2026-09-24', session(day), 'metric')).toBe(
      "Propose-moi un parcours de 12,5 km près d'où je suis pour ma séance du jeudi 24 septembre : 8 × 1 km",
    );
  });

  it('names no distance for a session set by time, and has no route draft where there is no route', () => {
    const timed = { ...intervals, steps: [{ label: 'Tempo', duration_seconds: 1500, target_zone: 'Z3' }] };
    expect(planDayRouteDraft(t, 'en', '2026-09-24', session(timed), 'metric')).toBe(
      'Suggest a route close to where I am for my session on Thursday, September 24: 8 × 1 km',
    );
    expect(planDayRouteDraft(t, 'en', '2026-09-24', { kind: 'uncovered' }, 'metric')).toBeNull();
    expect(planDayRouteDraft(t, 'en', '2026-09-24', { kind: 'rest', day: { ...intervals, rest: true }, week }, 'metric')).toBeNull();
    expect(planDayRouteDraft(t, 'en', '2026-09-24', session({ ...intervals, sport: 'swim' }), 'metric')).toBeNull();
  });

  it('calendarDayLabel says the session with its sport, minutes and intensity', () => {
    const day: CalendarDay = { date: '2026-09-24', activities: [], plan: session(intervals), beforeHistory: false };
    const label = calendarDayLabel(t, 'Thursday 24', day, '2026-09-21');
    expect(label).toBe('Thursday 24 — 8 × 1 km · Run · 55 min · Z4');
  });

  it('names an activity by its title and day, and by its sport when it has no title', () => {
    expect(activityNaming(t, activity(), 'en')).toEqual({ name: 'Sunday long ride', date: 'Sunday, September 20' });
    expect(activityNaming(translator('fr'), activity({ name: '  ' }), 'fr')).toEqual({
      name: 'Vélo',
      date: 'dimanche 20 septembre',
    });
  });
});
