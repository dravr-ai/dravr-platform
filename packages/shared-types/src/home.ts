// ABOUTME: Wire shapes of the athlete Home page's three reads — recent activities, one activity's route, the plan for today
// ABOUTME: Mirrors the pierre-server `/api/me/...` handlers; each parser rejects a body of any other shape instead of rendering half of it

import type { RouteView } from '@pierre/scene-types';
import { isWorkoutPlan, type WorkoutPlan } from './workout-plan.js';

/**
 * One activity on the Home page, projected from the durable activity cache.
 *
 * Every field is always present on the wire; an absent value is `null`, never
 * an omitted key. The parser below still reads an omitted nullable as `null`,
 * so a serializer that skips `None` degrades to the same meaning rather than
 * to an error.
 */
export interface HomeActivity {
  /** The provider's own id. Unique only together with `provider`. */
  id: string;
  /** Provider slug the activity came from: `strava`, `garmin`, `intervals_icu`, … */
  provider: string;
  /** The title the provider stores. Athlete-written, untrusted text. */
  name: string;
  /**
   * Sport as the wire spells it: a snake_case canonical key (`run`, `ride`,
   * `virtual_ride`, …) or, for a sport the vocabulary has no variant for, the
   * provider's own string. Resolve the label with `activitySportLabelKey`
   * from `@pierre/shared-constants`, and show this string when it has none.
   */
  sport_type: string;
  /** Start instant, RFC 3339 in UTC. */
  start_date: string;
  /** Duration in seconds as the provider reports it — elapsed time on Strava. */
  duration_seconds: number;
  /** Distance in metres, or null for an activity without distance. */
  distance_meters: number | null;
  /** Elevation gained in metres, or null when the provider reports none. */
  elevation_gain_meters: number | null;
  /**
   * True when the activity carries a start position or a route polyline —
   * recorded outdoors with GPS. False for indoor, trainer and manual entries,
   * which have no map to draw and no route to ask for.
   */
  has_gps: boolean;
  /**
   * The route as a Google encoded polyline at precision 5, trimmed on the
   * server at both ends by the same privacy radius every drawn route gets —
   * a sketch never shows where the athlete starts or stops. Null when the
   * provider sends no summary polyline (every provider but Strava, and cache
   * rows written before it was carried) or when nothing survives the trim.
   * Decode with `decodePolyline` from `@pierre/domain-utils`.
   */
  summary_polyline: string | null;
}

/** `GET /api/me/activities/recent?limit=N` — `limit` is clamped to 1..=20 and defaults to 5. */
export interface RecentActivitiesResponse {
  /** Newest first, at most `limit` of them, across every connected provider. */
  activities: HomeActivity[];
  /**
   * When a fetch last reached any of the athlete's providers, RFC 3339 — the
   * age of what `activities` shows. Null when nothing has ever been fetched.
   */
  as_of: string | null;
  /**
   * True when `as_of` is older than the freshness window: the server has
   * started a background refresh and the client asks again once, after
   * `HOME_STALE_REFETCH_DELAY_MS`.
   */
  stale: boolean;
}

/**
 * Why an activity has no route to draw.
 *
 * - `no_gps` — the activity recorded no GPS track: indoor, trainer, manual.
 * - `too_short` — a track exists, but after the privacy trim at both ends
 *   fewer than two points are left, so the only honest map is none.
 */
export type ActivityRouteUnavailableReason = 'no_gps' | 'too_short';

/** Every value {@link ActivityRouteUnavailableReason} takes, for exhaustive rendering. */
export const ACTIVITY_ROUTE_UNAVAILABLE_REASONS: readonly ActivityRouteUnavailableReason[] = [
  'no_gps',
  'too_short',
];

/**
 * `GET /api/me/activities/{provider}/{activity_id}/route`.
 *
 * Exactly one of `route` and `reason` is non-null. `route` is the same
 * `RouteView` the chat map renders, privacy-trimmed and simplified on the
 * server, so both clients' map components take it unchanged.
 */
export interface ActivityRouteResponse {
  route: RouteView | null;
  reason: ActivityRouteUnavailableReason | null;
}

/** `GET /api/me/training-plan?locale=xx`. */
export interface TrainingPlanResponse {
  /**
   * The athlete's active plan projected for a card — the same projection the
   * chat's `workout_plan` block carries — or null when there is no active plan.
   */
  plan: WorkoutPlan | null;
  /** The athlete's today, `YYYY-MM-DD` in their own timezone — the day the plan was projected for. */
  today: string;
}

const CIVIL_DATE = /^\d{4}-\d{2}-\d{2}$/;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

function isInstant(value: unknown): value is string {
  return typeof value === 'string' && !Number.isNaN(Date.parse(value));
}

/**
 * A nullable field read as `null` when omitted, `undefined` when present with
 * the wrong type — the caller rejects the whole body on `undefined`.
 */
function nullable<T>(value: unknown, accept: (candidate: unknown) => candidate is T): T | null | undefined {
  if (value === null || value === undefined) {
    return null;
  }
  return accept(value) ? value : undefined;
}

function isString(value: unknown): value is string {
  return typeof value === 'string';
}

function isCoordinate(value: unknown): value is [number, number] {
  return (
    Array.isArray(value) &&
    value.length === 2 &&
    isFiniteNumber(value[0]) &&
    isFiniteNumber(value[1]) &&
    Math.abs(value[0]) <= 90 &&
    Math.abs(value[1]) <= 180
  );
}

function isNumberSeries(value: unknown): value is number[] {
  return Array.isArray(value) && value.every(isFiniteNumber);
}

/** A climb addresses the track by index, so both ends must land on it, in order. */
function isClimbOn(length: number): (value: unknown) => value is RouteView['climbs'][number] {
  return (value: unknown): value is RouteView['climbs'][number] =>
    isRecord(value) &&
    Number.isInteger(value.start_index) &&
    Number.isInteger(value.end_index) &&
    (value.start_index as number) >= 0 &&
    (value.start_index as number) <= (value.end_index as number) &&
    (value.end_index as number) < length &&
    isFiniteNumber(value.avg_gradient) &&
    (value.category === null || typeof value.category === 'string');
}

function parseHomeActivity(value: unknown): HomeActivity | null {
  if (!isRecord(value)) {
    return null;
  }
  const distance = nullable(value.distance_meters, isFiniteNumber);
  const elevation = nullable(value.elevation_gain_meters, isFiniteNumber);
  const polyline = nullable(value.summary_polyline, isString);
  if (
    typeof value.id !== 'string' ||
    value.id === '' ||
    typeof value.provider !== 'string' ||
    value.provider === '' ||
    typeof value.name !== 'string' ||
    typeof value.sport_type !== 'string' ||
    !isInstant(value.start_date) ||
    !isFiniteNumber(value.duration_seconds) ||
    value.duration_seconds < 0 ||
    typeof value.has_gps !== 'boolean' ||
    distance === undefined ||
    elevation === undefined ||
    polyline === undefined
  ) {
    return null;
  }
  return {
    id: value.id,
    provider: value.provider,
    name: value.name,
    sport_type: value.sport_type,
    start_date: value.start_date,
    duration_seconds: value.duration_seconds,
    distance_meters: distance,
    elevation_gain_meters: elevation,
    has_gps: value.has_gps,
    summary_polyline: polyline,
  };
}

/**
 * Read a `GET /api/me/activities/recent` body.
 *
 * Returns `null` for any body that is not that response — including one whose
 * list holds a single malformed row, because a row the page cannot trust is a
 * contract bug to surface, not an activity to leave out quietly.
 */
export function parseRecentActivitiesResponse(body: unknown): RecentActivitiesResponse | null {
  if (!isRecord(body) || !Array.isArray(body.activities) || typeof body.stale !== 'boolean') {
    return null;
  }
  const asOf = nullable(body.as_of, isInstant);
  if (asOf === undefined) {
    return null;
  }
  const activities: HomeActivity[] = [];
  for (const row of body.activities) {
    const activity = parseHomeActivity(row);
    if (activity === null) {
      return null;
    }
    activities.push(activity);
  }
  return { activities, as_of: asOf, stale: body.stale };
}

function parseRouteView(value: unknown): RouteView | null {
  if (!isRecord(value) || !isRecord(value.bounds)) {
    return null;
  }
  const { bounds } = value;
  const elevations = nullable(value.elevation_meters, isNumberSeries);
  const distances = nullable(value.distances_meters, isNumberSeries);
  const title = nullable(value.title, isString);
  const climbs = value.climbs === undefined ? [] : value.climbs;
  if (
    !Array.isArray(value.coordinates) ||
    value.coordinates.length < 2 ||
    !value.coordinates.every(isCoordinate) ||
    !isFiniteNumber(bounds.min_latitude) ||
    !isFiniteNumber(bounds.max_latitude) ||
    !isFiniteNumber(bounds.min_longitude) ||
    !isFiniteNumber(bounds.max_longitude) ||
    elevations === undefined ||
    distances === undefined ||
    title === undefined ||
    typeof value.source_tool !== 'string' ||
    !Array.isArray(climbs)
  ) {
    return null;
  }
  const length = value.coordinates.length;
  // A parallel series is index-aligned with the track or absent; a series of
  // any other length would put a climb marker on the wrong kilometre.
  if (
    (elevations !== null && elevations.length !== length) ||
    (distances !== null && distances.length !== length) ||
    !climbs.every(isClimbOn(length))
  ) {
    return null;
  }
  return {
    coordinates: value.coordinates,
    bounds: {
      min_latitude: bounds.min_latitude,
      max_latitude: bounds.max_latitude,
      min_longitude: bounds.min_longitude,
      max_longitude: bounds.max_longitude,
    },
    elevation_meters: elevations,
    distances_meters: distances,
    climbs,
    title,
    source_tool: value.source_tool,
  };
}

/**
 * Read a `GET /api/me/activities/{provider}/{activity_id}/route` body.
 *
 * Returns `null` unless exactly one of `route` and `reason` is set and the
 * one that is set is well formed.
 */
export function parseActivityRouteResponse(body: unknown): ActivityRouteResponse | null {
  if (!isRecord(body)) {
    return null;
  }
  const hasRoute = body.route !== null && body.route !== undefined;
  const hasReason = body.reason !== null && body.reason !== undefined;
  if (hasRoute === hasReason) {
    return null;
  }
  if (hasRoute) {
    const route = parseRouteView(body.route);
    return route === null ? null : { route, reason: null };
  }
  const reason = ACTIVITY_ROUTE_UNAVAILABLE_REASONS.find((known) => known === body.reason);
  return reason === undefined ? null : { route: null, reason };
}

/**
 * Read a `GET /api/me/training-plan` body.
 *
 * `plan` must be present — null or a card. An omitted `plan` is rejected
 * rather than read as "no plan", because the page answers "no plan" with a
 * call to build one, which is the wrong thing to offer an athlete whose plan
 * the server simply failed to send.
 */
export function parseTrainingPlanResponse(body: unknown): TrainingPlanResponse | null {
  if (!isRecord(body) || typeof body.today !== 'string' || !CIVIL_DATE.test(body.today)) {
    return null;
  }
  if (body.plan === null) {
    return { plan: null, today: body.today };
  }
  return isWorkoutPlan(body.plan) ? { plan: body.plan, today: body.today } : null;
}
