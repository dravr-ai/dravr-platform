// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The activity view's figures, split and lap tables, and pace-or-speed choice, read off a detail answer
// ABOUTME: Pins that a figure the server did not send is left out rather than printed as zero, and pace follows the sport

import { describe, expect, it } from 'vitest';
import { formatDecimal } from '@pierre/chat-utils';
import type { DurationTranslate } from '@pierre/domain-utils';
import type { ActivityDetailResponse } from '@pierre/shared-types';
import en from '../../i18n/src/locales/en/translation.json';
import fr from '../../i18n/src/locales/fr/translation.json';
import de from '../../i18n/src/locales/de/translation.json';
import es from '../../i18n/src/locales/es/translation.json';
import pt from '../../i18n/src/locales/pt/translation.json';
import {
  ACTIVITY_PROMPTS,
  activityFigures,
  activityFirstLine,
  formatClock,
  formatKilometres,
  formatSpeed,
  lapsTable,
  speedForm,
  splitsTable,
  wholeMovingTimeSeconds,
} from '../src/activityView';

const CATALOGUES = { en, fr, de, es, pt } as const;
type Language = keyof typeof CATALOGUES;

/** The app's own catalogue for `language`, its `{{value}}` filled in as i18next fills it. */
function translator(language: Language): DurationTranslate {
  return (key, { value }) => {
    const template = key.split('.').reduce<unknown>(
      (node, part) => (node as Record<string, unknown>)[part],
      CATALOGUES[language],
    );
    if (typeof template !== 'string') throw new Error(`${language} has no ${key}`);
    return template.replace('{{value}}', String(value));
  };
}

function detail(overrides: Partial<ActivityDetailResponse> = {}, sport = 'run'): ActivityDetailResponse {
  return {
    activity: {
      id: 'tempo-10k',
      provider: 'strava',
      name: 'Tempo 10k',
      sport_type: sport,
      start_date: '2026-09-29T10:00:00Z',
      duration_seconds: 2_700,
      distance_meters: 10_000,
      elevation_gain_meters: 64,
      has_gps: true,
      summary_polyline: null,
    },
    average_heart_rate: 158,
    max_heart_rate: 176,
    average_speed_mps: 3.9,
    max_speed_mps: 5.2,
    average_power: 287,
    calories: 712,
    splits: [],
    laps: [],
    conversation_id: null,
    ...overrides,
  };
}

describe('speedForm', () => {
  it('reads a pace on foot, a pace per 100 m in the water, and a speed otherwise', () => {
    expect(speedForm('run')).toBe('pace_km');
    expect(speedForm('trail_running')).toBe('pace_km');
    expect(speedForm('hike')).toBe('pace_km');
    expect(speedForm('swim')).toBe('pace_100m');
    expect(speedForm('ride')).toBe('speed');
    expect(speedForm('Pickleball')).toBe('speed');
  });
});

describe('formatSpeed and formatClock', () => {
  it('prints a pace per km, per 100 m, and a speed in km/h', () => {
    expect(formatSpeed(1000 / 256, 'pace_km', 'en')).toBe('4:16 /km');
    expect(formatSpeed(100 / 112, 'pace_100m', 'en')).toBe('1:52 /100 m');
    expect(formatSpeed(7.9, 'speed', 'en')).toBe('28.4 km/h');
    expect(formatSpeed(0, 'pace_km', 'en')).toBeNull();
  });

  it('prints h:mm:ss from an hour up', () => {
    expect(formatClock(2_700)).toBe('45:00');
    expect(formatClock(3_725)).toBe('1:02:05');
  });
});

describe('activityFigures', () => {
  it('prints every figure a run carries, pace for its speed and no top speed', () => {
    expect(activityFigures(translator('en'), detail(), 'en').map((figure) => [figure.labelKey, figure.value])).toEqual([
      ['home.activity.figure.distance', '10.00 km'],
      ['home.activity.figure.duration', '45m'],
      ['home.activity.figure.elevationGain', '64 m'],
      ['home.activity.figure.avgPace', '4:16 /km'],
      ['home.activity.figure.avgHeartRate', '158 bpm'],
      ['home.activity.figure.maxHeartRate', '176 bpm'],
      ['home.activity.figure.avgPower', '287 W'],
      ['home.activity.figure.calories', '712 kcal'],
    ]);
  });

  it('prints a ride its speed and top speed', () => {
    const ride = detail({ average_speed_mps: 7.9, max_speed_mps: 15 }, 'ride');
    const values = Object.fromEntries(activityFigures(translator('en'), ride, 'en').map((figure) => [figure.id, figure.value]));
    expect(values.average_speed).toBe('28.4 km/h');
    expect(values.max_speed).toBe('54.0 km/h');
  });

  it('leaves out every figure the server did not send, never printing a zero', () => {
    const bare = detail({
      activity: { ...detail().activity, distance_meters: null, elevation_gain_meters: null, sport_type: 'yoga' },
      average_heart_rate: null,
      max_heart_rate: null,
      average_speed_mps: null,
      max_speed_mps: null,
      average_power: null,
      calories: null,
    });
    expect(activityFigures(translator('en'), bare, 'en').map((figure) => figure.id)).toEqual(['duration']);
  });
});

/** A lap of `meters` covered in `moving` seconds, stopped for `stopped` more. */
function lap(index: number, meters: number, moving: number | null, stopped = 0) {
  return {
    index,
    distance_meters: meters,
    elapsed_time_seconds: (moving ?? 0) + stopped,
    moving_time_seconds: moving,
    elevation_gain_meters: null,
    average_speed_mps: null,
    average_heart_rate: null,
    max_heart_rate: null,
    average_power: null,
  };
}

/** A split of `meters` covered in `moving` seconds. */
function split(index: number, meters: number, moving: number | null) {
  return {
    index,
    distance_meters: meters,
    elapsed_time_seconds: moving ?? 0,
    moving_time_seconds: moving,
    elevation_difference_meters: null,
    average_speed_mps: null,
    average_heart_rate: null,
  };
}

/** The figures by id, for a detail whose provider sent no average speed. */
function figuresWithoutProviderSpeed(overrides: Partial<ActivityDetailResponse>, sport: string) {
  const figures = activityFigures(translator('en'), detail({ average_speed_mps: null, max_speed_mps: null, ...overrides }, sport), 'en');
  return Object.fromEntries(figures.map((figure) => [figure.id, figure]));
}

describe('wholeMovingTimeSeconds', () => {
  it('adds up the laps, which partition the activity, when every lap carries its moving time', () => {
    // 10 km in 45:00; two laps of 5 km moving 20:00 and 21:00.
    expect(wholeMovingTimeSeconds(detail({ laps: [lap(1, 5_000, 1_200, 60), lap(2, 5_000, 1_260, 180)] }))).toBe(
      2_460,
    );
  });

  it('falls back on the splits when no lap carries it', () => {
    const splits = [split(1, 4_000, 1_000), split(2, 4_000, 1_000), split(3, 2_000, 500)];
    expect(wholeMovingTimeSeconds(detail({ laps: [lap(1, 10_000, null)], splits }))).toBe(2_500);
  });

  it('holds none when a segment lacks it, the list stops short of the distance, or it outruns the duration', () => {
    expect(wholeMovingTimeSeconds(detail())).toBeNull();
    expect(wholeMovingTimeSeconds(detail({ laps: [lap(1, 5_000, 1_200), lap(2, 5_000, null)] }))).toBeNull();
    expect(wholeMovingTimeSeconds(detail({ splits: [split(1, 1_000, 250), split(2, 1_000, 250)] }))).toBeNull();
    expect(wholeMovingTimeSeconds(detail({ laps: [lap(1, 10_000, 2_800)] }))).toBeNull();
    const noDistance = detail({ laps: [lap(1, 10_000, 2_400)] });
    expect(
      wholeMovingTimeSeconds({ ...noDistance, activity: { ...noDistance.activity, distance_meters: null } }),
    ).toBeNull();
  });
});

describe('the moving time and duration figures', () => {
  it('prints the moving time the laps hold, beside the duration', () => {
    const figures = figuresWithoutProviderSpeed({ laps: [lap(1, 5_000, 1_200), lap(2, 5_000, 1_260)] }, 'run');
    expect(figures.moving_time).toEqual({
      id: 'moving_time',
      labelKey: 'home.activity.figure.movingTime',
      value: '41m',
    });
    expect(figures.duration).toEqual({
      id: 'duration',
      labelKey: 'home.activity.figure.duration',
      value: '45m',
    });
  });

  it('prints only the duration when no moving time is held', () => {
    const figures = figuresWithoutProviderSpeed({}, 'run');
    expect(figures.moving_time).toBeUndefined();
    expect(figures.duration.labelKey).toBe('home.activity.figure.duration');
  });

  // A bare `45:45` beside a distance reads as hours and minutes as easily as
  // minutes and seconds, and French writes neither: the two times are in the
  // athlete's own words, as the Home row above the view writes them.
  it.each<[Language, string, string]>([
    ['en', '41m', '1h 15m 30s'],
    ['fr', '41 min', '1 h 15 min 30 s'],
    ['de', '41 Min.', '1 Std. 15 Min. 30 Sek.'],
    ['es', '41 min', '1 h 15 min 30 s'],
    ['pt', '41 min', '1 h 15 min 30 s'],
  ])('writes the moving time and the duration in %s', (language, moving, duration) => {
    const run = detail({ average_speed_mps: null, max_speed_mps: null, laps: [lap(1, 5_000, 1_200), lap(2, 5_000, 1_260)] });
    const long = { ...run, activity: { ...run.activity, duration_seconds: 4_530 } };
    const values = Object.fromEntries(
      activityFigures(translator(language), long, language).map((figure) => [figure.id, figure.value]),
    );
    expect(values.moving_time).toBe(moving);
    expect(values.duration).toBe(duration);
  });

  // Garmin's `duration` is the timer time, which leaves out every pause; its
  // elapsed time is a separate field the provider does not map. The same
  // `duration_seconds` is elapsed time on Strava and moving time on a
  // scraped detail page, so nothing the view prints may claim which it is.
  it('names a Garmin timer time a duration, and works out no pace "over elapsed" from it', () => {
    const garmin = detail(
      {
        activity: { ...detail().activity, provider: 'garmin', duration_seconds: 2_580 },
        average_speed_mps: null,
        max_speed_mps: null,
      },
      'run',
    );
    const figures = activityFigures(translator('en'), garmin, 'en');
    expect(figures.map((figure) => [figure.id, figure.labelKey, figure.value])).toEqual([
      ['distance', 'home.activity.figure.distance', '10.00 km'],
      ['duration', 'home.activity.figure.duration', '43m'],
      ['elevation_gain', 'home.activity.figure.elevationGain', '64 m'],
      ['average_heart_rate', 'home.activity.figure.avgHeartRate', '158 bpm'],
      ['max_heart_rate', 'home.activity.figure.maxHeartRate', '176 bpm'],
      ['average_power', 'home.activity.figure.avgPower', '287 W'],
      ['calories', 'home.activity.figure.calories', '712 kcal'],
    ]);
    expect(figures.map((figure) => figure.labelKey).join(' ')).not.toMatch(/[Ee]lapsed/);
  });
});

describe('the average pace or speed without the provider\'s figure', () => {
  // 10 km in 45:00, 41:00 moving when the laps hold it.
  const movingLaps = { laps: [lap(1, 5_000, 1_200), lap(2, 5_000, 1_260)] };

  it.each(['run', 'trail_running', 'walk', 'hike'])(
    'works out a %s pace per km over its moving time when it is held',
    (sport) => {
      const average = figuresWithoutProviderSpeed(movingLaps, sport).average_speed;
      expect(average).toEqual({
        id: 'average_speed',
        labelKey: 'home.activity.figure.avgPace',
        value: '4:06 /km',
      });
    },
  );

  it.each(['run', 'trail_running', 'walk', 'ride'])(
    'works out no %s average over the duration alone, whose meaning differs by provider',
    (sport) => {
      expect(figuresWithoutProviderSpeed({}, sport).average_speed).toBeUndefined();
    },
  );

  it('works out a ride\'s speed in km/h over its moving time', () => {
    const ride = {
      activity: { ...detail().activity, sport_type: 'ride', distance_meters: 40_000, duration_seconds: 5_400 },
      laps: [lap(1, 20_000, 2_400), lap(2, 20_000, 2_400)],
    };
    expect(figuresWithoutProviderSpeed(ride, 'ride').average_speed).toEqual({
      id: 'average_speed',
      labelKey: 'home.activity.figure.avgSpeed',
      value: '30.0 km/h',
    });
    // No top speed is made up: only the provider's is printed.
    expect(figuresWithoutProviderSpeed(ride, 'ride').max_speed).toBeUndefined();
  });

  it('keeps the provider\'s own figure when it sent one', () => {
    const figures = Object.fromEntries(activityFigures(translator('en'), detail(), 'en').map((figure) => [figure.id, figure]));
    expect(figures.average_speed.labelKey).toBe('home.activity.figure.avgPace');
    expect(figures.average_speed.value).toBe('4:16 /km');
  });

  it('prints none without a distance', () => {
    const noDistance = { activity: { ...detail().activity, distance_meters: null }, ...movingLaps };
    expect(figuresWithoutProviderSpeed(noDistance, 'run').average_speed).toBeUndefined();
  });
});

describe('splitsTable and lapsTable', () => {
  it('formats each split, and drops a column no split fills', () => {
    const table = splitsTable(
      detail({
        splits: [
          {
            index: 1,
            distance_meters: 1_000,
            elapsed_time_seconds: 262,
            moving_time_seconds: 260,
            elevation_difference_meters: 4.2,
            average_speed_mps: 1000 / 260,
            average_heart_rate: null,
          },
          {
            index: 2,
            distance_meters: 1_000,
            elapsed_time_seconds: 251,
            moving_time_seconds: null,
            elevation_difference_meters: -2.6,
            average_speed_mps: 1000 / 251,
            average_heart_rate: null,
          },
        ],
      }),
      'en',
    );
    expect(table).not.toBeNull();
    expect(table?.speedLabelKey).toBe('home.activity.column.pace');
    expect(table?.rows).toEqual([
      { index: 1, distance: '1.00 km', time: '4:20', speed: '4:20 /km', heartRate: null, elevation: '+4 m' },
      { index: 2, distance: '1.00 km', time: '4:11', speed: '4:11 /km', heartRate: null, elevation: '-3 m' },
    ]);
    expect(table?.hasHeartRate).toBe(false);
    expect(table?.hasElevation).toBe(true);
  });

  // A provider rounds a split's speed to the centimetre per second: 1 km in
  // 5:25 arrives as 3.07 m/s, which is 5:26 /km. The row printed "5:25" beside
  // "5:26 /km" until the pace was worked out from the time the row shows.
  it('prints a pace that agrees with the moving time beside it', () => {
    const split = {
      index: 1,
      distance_meters: 1_000,
      elapsed_time_seconds: 330,
      moving_time_seconds: 325,
      elevation_difference_meters: null,
      average_speed_mps: 3.07,
      average_heart_rate: null,
    };
    const row = splitsTable(detail({ splits: [split] }), 'fr')?.rows[0];
    expect(row?.time).toBe('5:25');
    expect(row?.speed).toBe('5:25 /km');
    // Without a moving time, the elapsed time counts the stops, so the
    // provider's own speed is the one to print.
    const elapsedOnly = splitsTable(detail({ splits: [{ ...split, moving_time_seconds: null }] }), 'fr')?.rows[0];
    expect(elapsedOnly?.time).toBe('5:30');
    expect(elapsedOnly?.speed).toBe('5:26 /km');
  });

  it('has no table without splits, and none for a lone lap that is the whole activity', () => {
    const lap = {
      index: 1,
      distance_meters: 10_000,
      elapsed_time_seconds: 2_700,
      moving_time_seconds: null,
      elevation_gain_meters: 64,
      average_speed_mps: 3.9,
      average_heart_rate: 158,
      max_heart_rate: 176,
      average_power: 287,
    };
    expect(splitsTable(detail(), 'en')).toBeNull();
    expect(lapsTable(detail({ laps: [lap] }), 'en')).toBeNull();
    expect(lapsTable(detail({ laps: [lap, { ...lap, index: 2 }] }), 'en')?.rows).toHaveLength(2);
  });
});

describe('figures in the athlete\'s notation', () => {
  // `toFixed` writes a full stop in every language; a French athlete read
  // "42.00 km" and "29.7 km/h" until the figures went through Intl.
  it('writes a French decimal comma in every figure and cell', () => {
    const ride = detail(
      {
        activity: { ...detail().activity, sport_type: 'ride', distance_meters: 42_000, elevation_gain_meters: 1_234.4 },
        average_speed_mps: 29.7 / 3.6,
        max_speed_mps: 15,
        splits: [
          {
            index: 1,
            distance_meters: 1_500,
            elapsed_time_seconds: 180,
            moving_time_seconds: 180,
            elevation_difference_meters: -2.6,
            average_speed_mps: 1_500 / 180,
            average_heart_rate: 151,
          },
        ],
      },
      'ride',
    );
    const values = Object.fromEntries(activityFigures(translator('fr'), ride, 'fr').map((figure) => [figure.id, figure.value]));
    expect(values.distance).toBe('42,00 km');
    expect(values.average_speed).toBe('29,7 km/h');
    expect(values.max_speed).toBe('54,0 km/h');
    expect(values.elevation_gain).toBe('1234 m');
    expect(splitsTable(ride, 'fr')?.rows).toEqual([
      { index: 1, distance: '1,50 km', time: '3:00', speed: '30,0 km/h', heartRate: '151 bpm', elevation: '-3 m' },
    ]);
  });

  it('keeps the full stop in English and German its comma', () => {
    expect(formatKilometres(42_000, 'en')).toBe('42.00 km');
    expect(formatKilometres(42_000, 'de')).toBe('42,00 km');
    expect(formatKilometres(850, 'fr')).toBe('850 m');
    expect(formatSpeed(29.7 / 3.6, 'speed', 'pt')).toBe('29,7 km/h');
    expect(formatDecimal(1234.5, 1, 'es')).toBe('1234,5');
  });
});

describe('ACTIVITY_PROMPTS', () => {
  it('offers the four questions, each with a label and a sentence', () => {
    expect(ACTIVITY_PROMPTS.map((prompt) => prompt.id)).toEqual(['analyze', 'compare', 'recovery', 'adjust']);
    for (const prompt of ACTIVITY_PROMPTS) {
      expect(prompt.labelKey).toMatch(/^home\.activity\.prompt\.\w+Label$/);
      expect(prompt.textKey).toMatch(/^home\.activity\.prompt\.\w+Text$/);
    }
  });
});

describe('activityFirstLine', () => {
  const naming = { name: 'Tempo 10k', date: 'mardi 29 septembre' };
  /** The French catalogue's sentence, filled as i18next fills it. */
  const t = (_key: string, options: { name: string; date: string; question: string }) =>
    (fr.home.activity.askAbout as string)
      .replace('{{name}}', options.name)
      .replace('{{date}}', options.date)
      .replace('{{question}}', options.question);

  it('names the activity around a typed question', () => {
    expect(activityFirstLine(t, naming, 'Mon allure était-elle régulière ?')).toBe(
      'À propos de mon activité « Tempo 10k » du mardi 29 septembre : Mon allure était-elle régulière ?'
    );
  });

  it('sends a slash command as typed, so it stays a command', () => {
    expect(activityFirstLine(t, naming, '/plan')).toBe('/plan');
  });
});
