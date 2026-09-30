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
   * Whether the activity may have a route to draw. False only when its route
   * was read once and the recording held no GPS, so there is no map to draw
   * and no route to ask for. True otherwise — including an activity whose
   * route was never read, since most providers' activity lists carry no
   * position: the route endpoint is what says whether there is a track.
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
   * The latest refresh of one of the athlete's providers that failed — a
   * scrape that errored or never answered, or an answer the server judged
   * incomplete — with no good sync of that provider after it; null when
   * every provider's latest attempt succeeded. It names the provider and
   * that provider's own last good sync, which is how old its rows on the
   * page are: `as_of` spans every provider.
   */
  sync_failure: SyncFailure | null;
  /**
   * True when the server started, or found running, a background refresh of
   * a provider that needs one — past the freshness window, or whose last
   * refresh failed — and the client asks again on the
   * `HOME_STALE_REFETCH_DELAYS_MS` schedule, stopping at the first answer
   * that is not stale. False while a failing provider is paused between
   * attempts: `sync_failure` says why, and the athlete's retry
   * (`?retry=true`) refreshes it at once.
   */
  stale: boolean;
}

/** A provider's latest refresh that failed, with that provider's own last good sync. */
export interface SyncFailure {
  /** The provider, by its user-facing slug: `strava`, `garmin`, … */
  provider: string;
  /** The provider's name as the athlete reads it: `Strava`. */
  provider_name: string;
  /** When the refresh failed, RFC 3339. */
  failed_at: string;
  /** When that provider last synced well, RFC 3339; null when it never has. */
  last_synced_at: string | null;
}

/**
 * Why an activity has no route to draw.
 *
 * - `no_gps` — the activity recorded no GPS track: indoor, trainer, manual.
 * - `too_short` — a track exists, but after the privacy trim at both ends
 *   fewer than two points are left, so the only honest map is none.
 * - `unavailable` — the route could not be read just now: the provider read
 *   failed, timed out, or answered without stream data. It says nothing
 *   about the recording, so `has_gps` stays true. The server keeps this answer
 *   for 10 minutes (`UNREAD_ROUTE_RECHECK_MINUTES` in pierre-server) and
 *   reads the route again on a request after that; the athlete's retry
 *   (`?retry=true`) reads it again at once.
 */
export type ActivityRouteUnavailableReason = 'no_gps' | 'too_short' | 'unavailable';

/** Every value {@link ActivityRouteUnavailableReason} takes, for exhaustive rendering. */
export const ACTIVITY_ROUTE_UNAVAILABLE_REASONS: readonly ActivityRouteUnavailableReason[] = [
  'no_gps',
  'too_short',
  'unavailable',
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
  const syncFailure = parseSyncFailure(body.sync_failure);
  if (asOf === undefined || syncFailure === undefined) {
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
  return { activities, as_of: asOf, sync_failure: syncFailure, stale: body.stale };
}

/**
 * A `sync_failure` read as `null` when omitted, `undefined` when present in
 * any other shape — the caller rejects the whole body on `undefined`.
 */
function parseSyncFailure(value: unknown): SyncFailure | null | undefined {
  if (value === null || value === undefined) {
    return null;
  }
  if (
    !isRecord(value) ||
    typeof value.provider !== 'string' ||
    value.provider === '' ||
    typeof value.provider_name !== 'string' ||
    value.provider_name === '' ||
    !isInstant(value.failed_at)
  ) {
    return undefined;
  }
  const lastSynced = nullable(value.last_synced_at, isInstant);
  if (lastSynced === undefined) {
    return undefined;
  }
  return {
    provider: value.provider,
    provider_name: value.provider_name,
    failed_at: value.failed_at,
    last_synced_at: lastSynced,
  };
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
