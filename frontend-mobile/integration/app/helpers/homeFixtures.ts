// ABOUTME: Server-shaped Home payloads for the mobile specs — a plan around one Thursday, five activities, two routes
// ABOUTME: Mirrors the /api/me/... contract: every key present, None as null, the polyline already trimmed by the server

import type { RouteView } from '@pierre/scene-types';
import type {
  ActivityRouteResponse,
  HomeActivity,
  PlanDay,
  RecentActivitiesResponse,
  TrainingPlanResponse,
  WorkoutPlan,
} from '@pierre/shared-types';

/** The athlete's today in their own zone: a Thursday, so the strip runs Monday 21 to Sunday 27. */
export const TODAY = '2026-09-24';

/** Today's session, with the steps and fuel a strip tap shows. */
export const TEMPO_DAY: PlanDay = {
  date: TODAY,
  sport: 'run',
  workout: 'Tempo run',
  duration_min: 50,
  intensity: 'Z3',
  rest: false,
  steps: [
    { label: 'Warm-up', duration_seconds: 900, target_zone: 'Z1' },
    { label: 'Tempo', duration_seconds: 1200, target_zone: 'Z3', repeat: 2 },
  ],
  fueling: { carbs_g_per_h: 60, fluid_ml_per_h: 500 },
};

/**
 * The plan as `PlanCard` serialises it: a build phase in its third week, the
 * current week with a rest day on Tuesday and nothing at all on Sunday — the
 * gap the page must never draw as rest — and next week folded under it.
 */
export const PLAN: WorkoutPlan = {
  goal_race: { name: 'Harricana 65K', date: '2026-10-18', discipline: 'trail', priority: 'A' },
  phases: [
    {
      kind: 'base',
      start: '2026-08-10',
      end: '2026-09-07',
      weeks: 4,
      purpose: 'Aerobic base',
      intent: 'Easy volume',
      current: false,
    },
    {
      kind: 'build',
      start: '2026-09-07',
      end: '2026-10-05',
      weeks: 4,
      purpose: 'Race specificity',
      intent: 'Threshold and long runs',
      current: true,
    },
  ],
  weeks: [
    {
      week_start: '2026-09-21',
      focus: 'Threshold',
      current: true,
      days: [
        { date: '2026-09-21', sport: 'run', workout: 'Easy run', duration_min: 40, intensity: 'Z2', rest: false },
        { date: '2026-09-22', sport: 'run', workout: 'Rest', intensity: '', rest: true },
        { date: '2026-09-23', sport: 'ride', workout: 'Endurance ride', duration_min: 90, intensity: 'Z2', rest: false },
        TEMPO_DAY,
        { date: '2026-09-25', sport: 'run', workout: 'Easy run', duration_min: 40, intensity: 'Z2', rest: false },
        { date: '2026-09-26', sport: 'run', workout: 'Long run', duration_min: 120, intensity: 'Z2', rest: false },
      ],
    },
    {
      week_start: '2026-09-28',
      focus: 'Race-pace long run',
      current: false,
      days: [
        { date: '2026-09-28', sport: 'run', workout: 'Rest', intensity: '', rest: true },
        { date: '2026-09-29', sport: 'run', workout: 'Hill repeats', duration_min: 60, intensity: 'Z4', rest: false },
      ],
    },
  ],
  weeks_deferred: 2,
};

export const PLAN_RESPONSE: TrainingPlanResponse = { plan: PLAN, today: TODAY };
export const NO_PLAN_RESPONSE: TrainingPlanResponse = { plan: null, today: TODAY };

/** Google's reference polyline: (38.5, -120.2) → (40.7, -120.95) → (43.252, -126.453). */
export const SUMMARY_POLYLINE = '_p~iF~ps|U_ulLnnqC_mqNvxq`@';

/**
 * Five activities, newest first, one of each kind the page draws differently:
 * the latest with a map, a Strava row sketched from its polyline, a row from
 * a provider that sends none (its sketch comes from the stored route), an
 * indoor ride and a strength session, neither of which has anything to draw.
 * Start times sit mid-afternoon UTC so the printed day is the same in any
 * zone a test machine runs in.
 */
export const ACTIVITIES: HomeActivity[] = [
  {
    id: '9001',
    provider: 'strava',
    name: 'Long ride',
    sport_type: 'ride',
    start_date: '2026-09-20T15:00:00Z',
    duration_seconds: 13265,
    distance_meters: 92000,
    elevation_gain_meters: 850,
    has_gps: true,
    summary_polyline: SUMMARY_POLYLINE,
  },
  {
    id: '9000',
    provider: 'strava',
    name: 'Tempo Thursday',
    sport_type: 'run',
    start_date: '2026-09-17T15:00:00Z',
    duration_seconds: 3012,
    distance_meters: 10200,
    elevation_gain_meters: 45,
    has_gps: true,
    summary_polyline: SUMMARY_POLYLINE,
  },
  {
    id: 'i77',
    provider: 'intervals_icu',
    name: 'Morning trail',
    sport_type: 'trail_run',
    start_date: '2026-09-16T15:00:00Z',
    duration_seconds: 4500,
    distance_meters: 12500,
    elevation_gain_meters: 410,
    has_gps: true,
    summary_polyline: null,
  },
  {
    id: '8999',
    provider: 'strava',
    name: 'Zwift — Watopia',
    sport_type: 'virtual_ride',
    start_date: '2026-09-15T15:00:00Z',
    duration_seconds: 3600,
    distance_meters: 30000,
    elevation_gain_meters: null,
    has_gps: false,
    summary_polyline: null,
  },
  {
    id: '8998',
    provider: 'strava',
    name: 'Gym',
    sport_type: 'weight_training',
    start_date: '2026-09-14T15:00:00Z',
    duration_seconds: 2700,
    distance_meters: null,
    elevation_gain_meters: null,
    has_gps: false,
    summary_polyline: null,
  },
];

export function recentResponse(overrides: Partial<RecentActivitiesResponse> = {}): RecentActivitiesResponse {
  return { activities: ACTIVITIES, as_of: '2026-09-24T08:30:00Z', stale: false, ...overrides };
}

/** The latest activity's stored route, the shape the chat map already draws. */
export const LATEST_ROUTE: RouteView = {
  coordinates: [
    [45.5, -73.6],
    [45.51, -73.59],
    [45.52, -73.58],
    [45.53, -73.57],
  ],
  bounds: { min_latitude: 45.5, max_latitude: 45.53, min_longitude: -73.6, max_longitude: -73.57 },
  elevation_meters: null,
  distances_meters: [0, 1200, 2400, 3600],
  climbs: [],
  title: 'Long ride',
  source_tool: 'strava',
};

/** The intervals.icu row's stored route: no polyline came with the row, so its sketch is drawn from these. */
export const TRAIL_ROUTE: RouteView = {
  coordinates: [
    [46.1, -74.2],
    [46.12, -74.18],
    [46.13, -74.15],
  ],
  bounds: { min_latitude: 46.1, max_latitude: 46.13, min_longitude: -74.2, max_longitude: -74.15 },
  elevation_meters: null,
  distances_meters: null,
  climbs: [],
  title: 'Morning trail',
  source_tool: 'intervals_icu',
};

export const LATEST_ROUTE_RESPONSE: ActivityRouteResponse = { route: LATEST_ROUTE, reason: null };
export const TRAIL_ROUTE_RESPONSE: ActivityRouteResponse = { route: TRAIL_ROUTE, reason: null };

/** A connected Strava, as `GET /api/providers` lists it. */
export const PROVIDERS_CONNECTED = {
  providers: [
    {
      provider: 'strava',
      display_name: 'Strava',
      requires_oauth: true,
      connected: true,
      needs_reauth: false,
      capabilities: ['activities'],
    },
  ],
};

/** The same list with nothing connected. */
export const PROVIDERS_NONE = {
  providers: [{ ...PROVIDERS_CONNECTED.providers[0], connected: false }],
};
