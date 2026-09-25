// ABOUTME: Unit tests for the Home page wire parsers — recent activities, one activity's route, the plan for today
// ABOUTME: Feeds each parser the bodies the server sends, then the malformed ones it must refuse rather than half-render

import { describe, it, expect } from 'vitest';
import {
  ACTIVITY_ROUTE_UNAVAILABLE_REASONS,
  parseActivityRouteResponse,
  parseRecentActivitiesResponse,
  parseTrainingPlanResponse,
} from '../src/home';
import { parseWorkoutPlan } from '../src/workout-plan';

/** A copy of `row` with `keys` left out, as a serializer that skips `None` would send it. */
function without(row: Record<string, unknown>, ...keys: string[]): Record<string, unknown> {
  return Object.fromEntries(Object.entries(row).filter(([key]) => !keys.includes(key)));
}

/** A Strava ride as the recent-activities route serializes it: every key present. */
const outdoorRide = {
  id: '12849301144',
  provider: 'strava',
  name: 'Tour du lac Brome',
  sport_type: 'ride',
  start_date: '2026-09-20T12:04:11Z',
  duration_seconds: 13260,
  distance_meters: 92150.4,
  elevation_gain_meters: 811,
  has_gps: true,
  summary_polyline: '_p~iF~ps|U_ulLnnqC_mqNvxq`@',
};

/** A trainer session: no position, no polyline, no distance the provider trusts. */
const indoorRide = {
  id: '12840022031',
  provider: 'strava',
  name: 'Zwift - Watopia',
  sport_type: 'virtual_ride',
  start_date: '2026-09-18T22:30:00Z',
  duration_seconds: 3600,
  distance_meters: null,
  elevation_gain_meters: null,
  has_gps: false,
  summary_polyline: null,
};

/** The plan card as `PlanCard` serializes it: optional fields skipped when absent. */
const planCard = {
  goal_race: { name: 'Big Red', date: '2026-10-18', discipline: 'gravel', priority: 'A' },
  phases: [
    {
      kind: 'build',
      start: '2026-09-07',
      end: '2026-10-05',
      weeks: 4,
      purpose: 'Raise threshold',
      intent: 'Two quality days a week',
      current: true,
    },
  ],
  current_phase_index: 0,
  weeks: [
    {
      week_start: '2026-09-21',
      focus: 'Threshold volume',
      phase_index: 0,
      current: true,
      days: [
        {
          date: '2026-09-24',
          sport: 'run',
          workout: 'Tempo run',
          duration_min: 50,
          intensity: 'Z3',
          rest: false,
          steps: [
            { label: 'Warm-up', duration_seconds: 600, target_zone: 'Z1', repeat: 1 },
            { label: 'Tempo', duration_seconds: 480, target_zone: 'Z3', repeat: 3, repeat_group: 1 },
          ],
          fueling: { carbs_g_per_h: 60, fluid_ml_per_h: 500, carb_source: 'glucose:fructose 1:0.8' },
        },
        { date: '2026-09-25', sport: 'rest', workout: 'Rest', intensity: 'rest', rest: true },
      ],
    },
  ],
  weeks_deferred: 3,
};

describe('parseRecentActivitiesResponse', () => {
  it('reads a list of outdoor and indoor activities with every field intact', () => {
    const parsed = parseRecentActivitiesResponse({
      activities: [outdoorRide, indoorRide],
      as_of: '2026-09-24T07:15:00Z',
      stale: false,
    });
    expect(parsed).not.toBeNull();
    expect(parsed?.activities).toHaveLength(2);
    expect(parsed?.activities[0]).toEqual(outdoorRide);
    expect(parsed?.activities[1].has_gps).toBe(false);
    expect(parsed?.activities[1].summary_polyline).toBeNull();
    expect(parsed?.as_of).toBe('2026-09-24T07:15:00Z');
    expect(parsed?.stale).toBe(false);
  });

  it('reads a never-fetched athlete as an empty list with no as_of', () => {
    expect(parseRecentActivitiesResponse({ activities: [], as_of: null, stale: false })).toEqual({
      activities: [],
      as_of: null,
      stale: false,
    });
  });

  it('reads an omitted nullable as null, so a serializer skipping None means the same thing', () => {
    const sparse = without(indoorRide, 'distance_meters', 'elevation_gain_meters', 'summary_polyline');
    const parsed = parseRecentActivitiesResponse({ activities: [sparse], stale: true });
    expect(parsed?.activities[0].distance_meters).toBeNull();
    expect(parsed?.activities[0].elevation_gain_meters).toBeNull();
    expect(parsed?.activities[0].summary_polyline).toBeNull();
    expect(parsed?.as_of).toBeNull();
    expect(parsed?.stale).toBe(true);
  });

  it('refuses the whole list when one row is malformed', () => {
    const broken = { ...outdoorRide, distance_meters: '92 km' };
    expect(
      parseRecentActivitiesResponse({ activities: [indoorRide, broken], as_of: null, stale: false }),
    ).toBeNull();
  });

  it.each([
    ['a missing id', { ...outdoorRide, id: '' }],
    ['a missing provider', { ...outdoorRide, provider: undefined }],
    ['an unparseable start date', { ...outdoorRide, start_date: 'Saturday' }],
    ['a negative duration', { ...outdoorRide, duration_seconds: -1 }],
    ['a has_gps that is not a boolean', { ...outdoorRide, has_gps: 'yes' }],
    ['a polyline that is not a string', { ...outdoorRide, summary_polyline: [[45.2, -72.4]] }],
  ])('refuses a row with %s', (_label, row) => {
    expect(parseRecentActivitiesResponse({ activities: [row], as_of: null, stale: false })).toBeNull();
  });

  it('refuses a body that is not the response at all', () => {
    expect(parseRecentActivitiesResponse(null)).toBeNull();
    expect(parseRecentActivitiesResponse([outdoorRide])).toBeNull();
    expect(parseRecentActivitiesResponse({ activities: [outdoorRide] })).toBeNull();
    expect(parseRecentActivitiesResponse({ activities: [], as_of: 'yesterday', stale: false })).toBeNull();
  });
});

describe('parseActivityRouteResponse', () => {
  const route = {
    coordinates: [
      [45.2051, -72.4102],
      [45.2074, -72.4151],
      [45.2113, -72.4188],
    ],
    bounds: { min_latitude: 45.2051, max_latitude: 45.2113, min_longitude: -72.4188, max_longitude: -72.4102 },
    elevation_meters: [212, 219.5, 231],
    distances_meters: [480, 925.3, 1440.8],
    climbs: [{ start_index: 0, end_index: 2, avg_gradient: 3.6, category: null }],
    title: 'Tour du lac Brome',
    source_tool: 'strava',
  };

  it('reads a drawn route unchanged', () => {
    const parsed = parseActivityRouteResponse({ route, reason: null });
    expect(parsed?.reason).toBeNull();
    expect(parsed?.route?.coordinates).toHaveLength(3);
    expect(parsed?.route?.bounds.max_latitude).toBe(45.2113);
    expect(parsed?.route?.elevation_meters).toEqual([212, 219.5, 231]);
    expect(parsed?.route?.climbs[0].avg_gradient).toBe(3.6);
    expect(parsed?.route?.title).toBe('Tour du lac Brome');
    expect(parsed?.route?.source_tool).toBe('strava');
  });

  it('reads omitted series as null, the way the chat track serializes them', () => {
    const bare = without(route, 'elevation_meters', 'distances_meters');
    const parsed = parseActivityRouteResponse({ route: { ...bare, climbs: [] }, reason: null });
    expect(parsed?.route?.elevation_meters).toBeNull();
    expect(parsed?.route?.distances_meters).toBeNull();
  });

  it.each(ACTIVITY_ROUTE_UNAVAILABLE_REASONS.map((reason) => [reason]))(
    'reads the %s refusal as no route',
    (reason) => {
      expect(parseActivityRouteResponse({ route: null, reason })).toEqual({ route: null, reason });
    },
  );

  it('refuses a body that sets both, neither, or an unknown reason', () => {
    expect(parseActivityRouteResponse({ route, reason: 'no_gps' })).toBeNull();
    expect(parseActivityRouteResponse({ route: null, reason: null })).toBeNull();
    expect(parseActivityRouteResponse({ route: null, reason: 'indoor' })).toBeNull();
  });

  it('refuses a route whose parallel series or climbs do not fit the track', () => {
    expect(
      parseActivityRouteResponse({ route: { ...route, elevation_meters: [212, 219.5] }, reason: null }),
    ).toBeNull();
    expect(
      parseActivityRouteResponse({
        route: { ...route, climbs: [{ start_index: 1, end_index: 3, avg_gradient: 4, category: '4' }] },
        reason: null,
      }),
    ).toBeNull();
  });

  it('refuses a single point or an out-of-range coordinate', () => {
    expect(
      parseActivityRouteResponse({ route: { ...route, coordinates: [[45.2, -72.4]] }, reason: null }),
    ).toBeNull();
    expect(
      parseActivityRouteResponse({
        route: { ...route, coordinates: [[45.2, -72.4], [95, -72.4], [45.21, -72.41]] },
        reason: null,
      }),
    ).toBeNull();
  });
});

describe('parseTrainingPlanResponse', () => {
  it('reads an active plan and the athlete-local today', () => {
    const parsed = parseTrainingPlanResponse({ plan: planCard, today: '2026-09-24' });
    expect(parsed?.today).toBe('2026-09-24');
    expect(parsed?.plan?.goal_race.name).toBe('Big Red');
    expect(parsed?.plan?.weeks[0].days[0].steps?.[1].repeat_group).toBe(1);
    expect(parsed?.plan?.weeks[0].days[0].fueling?.carb_source).toBe('glucose:fructose 1:0.8');
    expect(parsed?.plan?.weeks[0].days[1].rest).toBe(true);
    expect(parsed?.plan?.weeks_deferred).toBe(3);
  });

  it('reads an explicit null plan as no active plan', () => {
    expect(parseTrainingPlanResponse({ plan: null, today: '2026-09-24' })).toEqual({
      plan: null,
      today: '2026-09-24',
    });
  });

  it('refuses an omitted plan rather than offering to build one the athlete may already have', () => {
    expect(parseTrainingPlanResponse({ today: '2026-09-24' })).toBeNull();
  });

  it('refuses a malformed plan or a today that is not a civil date', () => {
    expect(parseTrainingPlanResponse({ plan: { weeks: [] }, today: '2026-09-24' })).toBeNull();
    expect(parseTrainingPlanResponse({ plan: null, today: '2026-09-24T00:00:00Z' })).toBeNull();
    expect(parseTrainingPlanResponse({ plan: null })).toBeNull();
  });

  it('agrees with the chat block parser about what a plan is', () => {
    expect(parseWorkoutPlan(JSON.stringify(planCard))).toEqual(planCard);
    expect(parseWorkoutPlan(JSON.stringify({ weeks: [] }))).toBeNull();
    expect(parseWorkoutPlan('not json')).toBeNull();
    expect(parseWorkoutPlan(undefined)).toBeNull();
  });
});
