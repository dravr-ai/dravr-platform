// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Wire-shaped fixtures for the Home tests — a plan card around Thursday 2026-09-24 and five cached activities
// ABOUTME: Built as the server serializes them, so a component test starts from the contract rather than a convenience shape

import type { HomeActivity, RecentActivitiesResponse, WorkoutPlan } from '@pierre/shared-types';
import type { RouteView } from '@pierre/scene-types';

/** The athlete's today in every Home test: a Thursday, week 2 of the build phase. */
export const TODAY = '2026-09-24';

/**
 * The current week holds a session on Thursday (today), a rest day on
 * Friday (tomorrow), and no entry at all for Saturday — the gap the page
 * must say is uncovered, never rest. Next week carries its own focus.
 */
export function homePlan(overrides: Partial<WorkoutPlan> = {}): WorkoutPlan {
  return {
    goal_race: { name: 'Montreal Marathon', date: '2026-11-22', discipline: 'run_marathon', priority: 'A' },
    season_start: '2026-08-31',
    season_end: '2026-11-23',
    phases: [
      {
        kind: 'base',
        start: '2026-08-31',
        end: '2026-09-14',
        weeks: 2,
        purpose: 'aerobic base',
        intent: 'easy volume',
        current: false,
      },
      {
        kind: 'build',
        start: '2026-09-14',
        end: '2026-10-26',
        weeks: 6,
        purpose: 'raise the ceiling',
        intent: 'two hard days, the rest easy',
        current: true,
      },
    ],
    current_phase_index: 1,
    weeks: [
      {
        week_start: '2026-09-21',
        focus: 'threshold volume',
        phase_index: 1,
        current: true,
        days: [
          { date: '2026-09-21', sport: 'run', workout: 'Easy run', duration_min: 40, intensity: 'Z2', rest: false },
          { date: '2026-09-22', sport: 'run', workout: 'Strides', duration_min: 35, intensity: 'Z2', rest: false },
          { date: '2026-09-23', sport: 'rest', workout: '', intensity: '', rest: true },
          {
            date: '2026-09-24',
            sport: 'run',
            workout: 'Tempo run',
            duration_min: 50,
            intensity: 'threshold',
            rest: false,
            steps: [
              { label: 'Warm-up', duration_seconds: 900, target_zone: 'Z1' },
              { label: 'Tempo', duration_seconds: 1500, target_zone: 'Z3' },
              { label: 'Cool-down', duration_seconds: 600, target_zone: 'Z1' },
            ],
            fueling: { carbs_g_per_h: 40, fluid_ml_per_h: 500 },
          },
          { date: '2026-09-25', sport: 'rest', workout: '', intensity: '', rest: true },
          { date: '2026-09-27', sport: 'ride', workout: 'Long ride', duration_min: 180, intensity: 'Z2', rest: false },
        ],
      },
      {
        week_start: '2026-09-28',
        focus: 'absorb the block',
        phase_index: 1,
        current: false,
        days: [
          { date: '2026-09-28', sport: 'run', workout: 'Recovery jog', duration_min: 30, intensity: 'Z1', rest: false },
        ],
      },
    ],
    weeks_deferred: 4,
    ...overrides,
  };
}

/** A few points around Mont Royal, as the route endpoint's coordinates. */
export const ROUTE_COORDINATES: Array<[number, number]> = [
  [45.5, -73.6],
  [45.51, -73.61],
  [45.52, -73.63],
  [45.53, -73.62],
];

/** The chat's `RouteView`, as the route endpoint answers it for the latest activity. */
export function routeView(title: string): RouteView {
  return {
    coordinates: ROUTE_COORDINATES,
    bounds: { min_latitude: 45.5, max_latitude: 45.53, min_longitude: -73.63, max_longitude: -73.6 },
    elevation_meters: null,
    distances_meters: [0, 1400, 3100, 4500],
    climbs: [],
    title,
    source_tool: 'strava',
  };
}

/**
 * Google's documented example polyline — (38.5, -120.2), (40.7, -120.95),
 * (43.252, -126.453) — standing in for a server-trimmed summary polyline.
 */
export const SAMPLE_POLYLINE = '_p~iF~ps|U_ulLnnqC_mqNvxq`@';

export function activity(overrides: Partial<HomeActivity> & Pick<HomeActivity, 'id'>): HomeActivity {
  return {
    provider: 'strava',
    name: 'Morning run',
    sport_type: 'run',
    start_date: '2026-09-23T11:00:00Z',
    duration_seconds: 3600,
    distance_meters: 10200,
    elevation_gain_meters: 85,
    has_gps: true,
    summary_polyline: null,
    ...overrides,
  };
}

/**
 * Five activities, newest first: the latest outdoors with a stored route, one
 * with a summary polyline, one recorded with GPS but no polyline (its sketch
 * comes from the route endpoint), one indoor, and one from another provider.
 */
export function fiveActivities(): HomeActivity[] {
  return [
    activity({
      id: 'act-5',
      name: 'Long ride',
      sport_type: 'ride',
      start_date: '2026-09-20T13:00:00Z',
      duration_seconds: 13260,
      distance_meters: 92400,
      elevation_gain_meters: 820,
    }),
    activity({ id: 'act-4', name: 'Tempo Tuesday', summary_polyline: SAMPLE_POLYLINE, start_date: '2026-09-18T11:00:00Z' }),
    activity({ id: 'act-3', name: 'Hill repeats', start_date: '2026-09-16T11:00:00Z' }),
    activity({
      id: 'act-2',
      name: 'Trainer spin',
      sport_type: 'virtual_ride',
      has_gps: false,
      distance_meters: 30000,
      elevation_gain_meters: null,
      start_date: '2026-09-15T22:00:00Z',
    }),
    activity({
      id: 'act-1',
      provider: 'garmin',
      name: 'Lake loop',
      start_date: '2026-09-14T11:00:00Z',
      summary_polyline: null,
      has_gps: false,
    }),
  ];
}

export function recentResponse(overrides: Partial<RecentActivitiesResponse> = {}): RecentActivitiesResponse {
  return {
    activities: fiveActivities(),
    as_of: '2026-09-24T08:15:00Z',
    stale: false,
    ...overrides,
  };
}
