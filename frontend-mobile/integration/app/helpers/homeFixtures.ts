// ABOUTME: Server-shaped Home payloads for the mobile specs — a plan around one Thursday, a training status, five activities, their routes, one's view
// ABOUTME: Mirrors the /api/me/... contract: every key present, None as null, the polyline already trimmed by the server

import type { RouteView } from '@pierre/scene-types';
import { addCivilDays } from '@pierre/shared-types';
import type {
  ActivityDetailResponse,
  ActivityRouteResponse,
  CalendarResponse,
  HomeActivity,
  PlanDay,
  RecentActivitiesResponse,
  TrainingPlanResponse,
  TrainingStatusResponse,
  TrainingVolumeResponse,
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

/**
 * The training status of an athlete mid-block: three days of trend that cross
 * the zero line, a load ratio, and no lighter day called for.
 */
export const STATUS_RESPONSE: TrainingStatusResponse = {
  today: TODAY,
  form: { band: 'heavy_block', pct_of_fitness: -22 },
  trend: [
    { date: '2026-09-22', band: 'productive', pct_of_fitness: -12 },
    { date: '2026-09-23', band: 'fresh', pct_of_fitness: 6 },
    { date: TODAY, band: 'heavy_block', pct_of_fitness: -22 },
  ],
  load_ratio: { ratio: 1.37, acute_days: 7, chronic_days: 28 },
  recovery_days: 0,
};

/** What the server answers when the stored history cannot stand behind today. */
export const THIN_STATUS_RESPONSE: TrainingStatusResponse = {
  today: TODAY,
  form: null,
  trend: [],
  load_ratio: null,
  recovery_days: null,
};

/**
 * Three weeks of volume ending with the week of {@link TODAY}: runs, a quiet
 * week, then a run week with a ride.
 */
export const VOLUME_RESPONSE: TrainingVolumeResponse = {
  today: TODAY,
  weeks: [
    {
      week_start: '2026-09-07',
      sports: [{ sport_type: 'run', activities: 3, distance_meters: 21_000, duration_seconds: 6_300, elevation_gain_meters: 60 }],
    },
    { week_start: '2026-09-14', sports: [] },
    {
      week_start: '2026-09-21',
      sports: [
        { sport_type: 'ride', activities: 1, distance_meters: 40_000, duration_seconds: 7_230, elevation_gain_meters: 450 },
        { sport_type: 'run', activities: 2, distance_meters: 15_000, duration_seconds: 5_400, elevation_gain_meters: 80 },
      ],
    },
  ],
};

/** Google's reference polyline: (38.5, -120.2) → (40.7, -120.95) → (43.252, -126.453). */
export const SUMMARY_POLYLINE = '_p~iF~ps|U_ulLnnqC_mqNvxq`@';

/**
 * Five activities, newest first, one of each kind the page draws differently:
 * the latest with a map, a Strava row sketched from its polyline, a row from
 * a provider whose list carries no position (its sketch comes from the route
 * read), an indoor ride whose route was read once and held no GPS — the one
 * row that says `has_gps: false` — and a strength session whose route was
 * never read, so it says `has_gps: true` and the route read is what answers
 * that there is no track. Start times sit mid-afternoon UTC so the printed
 * day is the same in any zone a test machine runs in.
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
    attribution: null,
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
    attribution: null,
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
    attribution: null,
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
    attribution: null,
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
    has_gps: true,
    summary_polyline: null,
    attribution: null,
  },
];

export function recentResponse(overrides: Partial<RecentActivitiesResponse> = {}): RecentActivitiesResponse {
  return { activities: ACTIVITIES, as_of: '2026-09-24T08:30:00Z', sync_failure: null, stale: false, ...overrides };
}

/** The latest activity's stored route, the shape the chat map already draws — untitled, as the server sends it. */
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
  title: null,
  source_tool: 'strava',
};

/** The intervals.icu row's route: no polyline came with the row, so its sketch is drawn from these. */
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
  title: null,
  source_tool: 'intervals_icu',
};

export const LATEST_ROUTE_RESPONSE: ActivityRouteResponse = { route: LATEST_ROUTE, reason: null };
export const TRAIL_ROUTE_RESPONSE: ActivityRouteResponse = { route: TRAIL_ROUTE, reason: null };
/** The strength session's route read: the recording held no GPS. */
export const NO_GPS_ROUTE_RESPONSE: ActivityRouteResponse = { route: null, reason: 'no_gps' };

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

/**
 * A Strava whose session died, served by two backends that carry one name,
 * beside a Garmin whose session died too, a healthy intervals.icu, and a
 * disconnected COROS whose flag means nothing: two names to reconnect.
 */
export const PROVIDERS_RECONNECT = {
  providers: [
    { ...PROVIDERS_CONNECTED.providers[0], needs_reauth: true },
    { ...PROVIDERS_CONNECTED.providers[0], provider: 'sciotte', requires_oauth: false, needs_reauth: true },
    { ...PROVIDERS_CONNECTED.providers[0], provider: 'garmin', display_name: 'Garmin', needs_reauth: true },
    { ...PROVIDERS_CONNECTED.providers[0], provider: 'intervals_icu', display_name: 'Intervals.icu' },
    {
      ...PROVIDERS_CONNECTED.providers[0],
      provider: 'coros',
      display_name: 'COROS',
      connected: false,
      needs_reauth: true,
    },
  ],
};

/**
 * Every connected provider flagged: a Garmin scrape session and its Strava
 * mirror whose sessions died, and a disconnected COROS. Nothing connected
 * still syncs.
 */
export const PROVIDERS_ONLY_FLAGGED = {
  providers: [
    { ...PROVIDERS_CONNECTED.providers[0], provider: 'sciotte', requires_oauth: false, needs_reauth: true },
    { ...PROVIDERS_CONNECTED.providers[0], provider: 'garmin', display_name: 'Garmin', needs_reauth: true },
    {
      ...PROVIDERS_CONNECTED.providers[0],
      provider: 'coros',
      display_name: 'COROS',
      connected: false,
      needs_reauth: false,
    },
  ],
};

/**
 * `GET /api/me/activities/strava/9000` — Tempo Thursday's view: the figures
 * the cache holds for it and two kilometre splits. No power was recorded, so
 * none is sent.
 */
export const TEMPO_DETAIL_RESPONSE: ActivityDetailResponse = {
  activity: ACTIVITIES[1],
  average_heart_rate: 158,
  max_heart_rate: 176,
  average_speed_mps: 1000 / 295,
  max_speed_mps: 5.1,
  average_power: null,
  calories: 712,
  splits: [
    {
      index: 1,
      distance_meters: 1000,
      elapsed_time_seconds: 300,
      moving_time_seconds: 298,
      elevation_difference_meters: 6,
      average_speed_mps: 1000 / 298,
      average_heart_rate: 151,
    },
    {
      index: 2,
      distance_meters: 1000,
      elapsed_time_seconds: 290,
      moving_time_seconds: null,
      elevation_difference_meters: -4,
      average_speed_mps: 1000 / 290,
      average_heart_rate: 160,
    },
  ],
  laps: [],
  conversation_id: null,
};

/** The first day the server's activity cache holds in full in these specs. */
export const HISTORY_START = '2026-09-09';

/** The calendar read of the strip's own week, as the screen asks for it on arrival. */
export const CALENDAR_URL = 'GET /api/me/calendar?from=2026-09-21&to=2026-09-27';

/**
 * The calendar answer for `from..=to`, as the server gives it with `plan`
 * active (null for none) and `activities` cached: each workout on the day of
 * its start — mid-afternoon UTC, the same day in any zone — and the plan
 * weeks overlapping the span.
 */
export function calendarAnswer(
  from: string,
  to: string,
  {
    plan = PLAN,
    activities = ACTIVITIES,
    historyStart = HISTORY_START,
  }: { plan?: WorkoutPlan | null; activities?: HomeActivity[]; historyStart?: string } = {},
): CalendarResponse {
  return {
    today: TODAY,
    from,
    to,
    history_start: historyStart,
    activities: activities
      .map((activity) => ({ date: activity.start_date.slice(0, 10), activity }))
      .filter((entry) => entry.date >= from && entry.date <= to && entry.date >= historyStart)
      .sort((a, b) => a.activity.start_date.localeCompare(b.activity.start_date)),
    plan_weeks:
      plan === null
        ? null
        : plan.weeks.filter((week) => week.week_start <= to && addCivilDays(week.week_start, 6) >= from),
  };
}
