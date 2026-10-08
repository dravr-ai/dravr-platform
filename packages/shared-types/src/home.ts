// ABOUTME: Wire shapes of the athlete Home page's reads — recent activities, one activity's view and route, the plan, the calendar
// ABOUTME: Mirrors the pierre-server `/api/me/...` handlers; each parser rejects a body of any other shape instead of rendering half of it

import type { RouteView } from '@pierre/scene-types';
import { isWorkoutPlan, type PlanDay, type PlanWeek, type WorkoutPlan } from './workout-plan.js';

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
  /**
   * The attribution the activity must be shown with, beside its title:
   * `"Garmin Forerunner 965"` when a Garmin device recorded it and its model
   * is known, `"Garmin"` when only its Garmin origin is (intervals.icu's API
   * terms require it in the form Garmin's brand guidelines set), null
   * otherwise.
   * Show it as served; no client composes an attribution of its own.
   */
  attribution: string | null;
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

/**
 * A route answer that is not one yet: the server's 25-second bound ran out
 * while the read waited for the athlete's provider turn behind other reads,
 * or while the read it started was still running. Nothing was stored and
 * nothing is known about the route — it is neither `unavailable` nor drawn —
 * so the client asks again, and the read, once it lands, answers from the
 * server's store. Never shown to the athlete as a reason: the map keeps
 * loading.
 */
export interface ActivityRoutePending {
  route: null;
  reason: 'pending';
  /**
   * Seconds the read can still take by the server's own bounds: one
   * provider-read bound for every read ahead of it in the athlete's turn and
   * one for its own. The client keeps asking within it. `null` when the
   * server named no bound.
   */
  settles_within_secs: number | null;
}

/**
 * Every body `GET /api/me/activities/{provider}/{activity_id}/route` answers:
 * a settled {@link ActivityRouteResponse}, or {@link ActivityRoutePending}.
 */
export type ActivityRouteAnswer = ActivityRouteResponse | ActivityRoutePending;

/** One split of an activity: a uniform distance bucket the provider carved. */
export interface ActivitySplit {
  /** 1-based position along the activity. */
  index: number;
  distance_meters: number;
  /** Stops included. */
  elapsed_time_seconds: number;
  /** Stops left out; null when the provider sent none. */
  moving_time_seconds: number | null;
  /** Net change over the split, negative downhill. */
  elevation_difference_meters: number | null;
  average_speed_mps: number | null;
  average_heart_rate: number | null;
}

/** One lap of an activity, as the athlete's button or the workout marked it. */
export interface ActivityLap {
  /** 1-based position along the activity. */
  index: number;
  distance_meters: number;
  /** Stops included. */
  elapsed_time_seconds: number;
  /** Stops left out; null when the provider sent none. */
  moving_time_seconds: number | null;
  elevation_gain_meters: number | null;
  average_speed_mps: number | null;
  average_heart_rate: number | null;
  max_heart_rate: number | null;
  average_power: number | null;
}

/**
 * `GET /api/me/activities/{provider}/{activity_id}` — one workout as its own
 * view shows it, from the server's activity cache.
 *
 * `activity` is exactly the workout's Home row: merged the same way, its
 * `provider` and `id` those of the copy whose route it draws, so the route
 * endpoint is asked with them. Every figure the cache does not hold is null —
 * never estimated — and a workout without splits or laps has empty lists.
 */
export interface ActivityDetailResponse {
  activity: HomeActivity;
  /** Beats per minute. */
  average_heart_rate: number | null;
  max_heart_rate: number | null;
  /** Metres per second, as the provider computed it. */
  average_speed_mps: number | null;
  max_speed_mps: number | null;
  /** Watts. */
  average_power: number | null;
  /** Kilocalories. */
  calories: number | null;
  splits: ActivitySplit[];
  laps: ActivityLap[];
  /**
   * The conversation the view opened about this workout, while it is still
   * the athlete's own; null before the view's first question. Linked with
   * `PUT …/conversation`, so the thread resumes on any device.
   */
  conversation_id: string | null;
}

/**
 * The bands form falls in, as a share of the athlete's own fitness — the
 * engine's one form vocabulary (`FormBand` in dravr-cageux). The edges live
 * there and nowhere else: a client names the band it is handed and never
 * derives one from a number.
 */
export const FORM_BANDS = [
  'insufficient_history',
  'deep_fatigue',
  'heavy_block',
  'productive',
  'balanced',
  'fresh',
  'detraining',
] as const;

export type FormBand = (typeof FORM_BANDS)[number];

/** Form on one day: its band, and form as a whole percentage of fitness. */
export interface FormReading {
  band: FormBand;
  /** Null when there is no chronic base to scale form against; the band then says so. */
  pct_of_fitness: number | null;
}

/** One day of the form trend. */
export interface FormTrendPoint extends FormReading {
  /** The athlete's civil date, `YYYY-MM-DD`. */
  date: string;
}

/** The recent load as a multiple of the athlete's own baseline — a magnitude, never a verdict. */
export interface TrainingLoadRatio {
  ratio: number;
  /** Days in the recent window. */
  acute_days: number;
  /** Days in the baseline window. */
  chronic_days: number;
}

/** `GET /api/me/training-status`. */
export interface TrainingStatusResponse {
  /** The athlete's today, `YYYY-MM-DD` in their own timezone. */
  today: string;
  /** Form today; null when the stored history cannot stand behind that day. */
  form: FormReading | null;
  /** Form on each day the stored history stands behind, oldest first, ending today; empty when `form` is null. */
  trend: FormTrendPoint[];
  /** Null until the baseline window holds enough history. */
  load_ratio: TrainingLoadRatio | null;
  /** Lighter days today's form calls for; null when form cannot be judged. */
  recovery_days: number | null;
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

/** One cached workout on the athlete's day it belongs to. */
export interface CalendarActivity {
  /** The athlete's civil date of the workout's start, `YYYY-MM-DD` in their own timezone. */
  date: string;
  /** The workout exactly as its Home row projects it. */
  activity: HomeActivity;
}

/**
 * `GET /api/me/calendar?from=YYYY-MM-DD&to=YYYY-MM-DD` — the athlete's days
 * between two civil dates, at most 42 of them: the cached workouts on each
 * day and the plan's weeks over them. Served from the server's cache and plan
 * store alone; reading an older span never starts a provider fetch.
 */
export interface CalendarResponse {
  /** The athlete's today, `YYYY-MM-DD` in their own timezone. */
  today: string;
  /** The first day read. */
  from: string;
  /** The last day read, included. */
  to: string;
  /**
   * The first day whose workouts the cache still holds in full. A day before
   * it is unknown — pruned past the retention window — never a day without
   * training, and no workout is answered for it.
   */
  history_start: string;
  /** The workouts on the days read, oldest first. */
  activities: CalendarActivity[];
  /**
   * The active plan's weeks overlapping the days read, projected as the plan
   * card projects a week, in calendar order; null when there is no active
   * plan. Look a day up with `planWeeksDayOn`.
   */
  plan_weeks: PlanWeek[] | null;
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
  const attribution = nullable(value.attribution, isString);
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
    polyline === undefined ||
    attribution === undefined
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
    attribution,
  };
}

/** A value that must be a non-negative finite number. */
function isCount(value: unknown): value is number {
  return isFiniteNumber(value) && value >= 0;
}

function parseSplit(value: unknown): ActivitySplit | null {
  if (!isRecord(value)) return null;
  const moving = nullable(value.moving_time_seconds, isCount);
  const elevation = nullable(value.elevation_difference_meters, isFiniteNumber);
  const speed = nullable(value.average_speed_mps, isCount);
  const heartRate = nullable(value.average_heart_rate, isCount);
  if (
    !Number.isInteger(value.index) ||
    !isCount(value.distance_meters) ||
    !isCount(value.elapsed_time_seconds) ||
    moving === undefined ||
    elevation === undefined ||
    speed === undefined ||
    heartRate === undefined
  ) {
    return null;
  }
  return {
    index: value.index as number,
    distance_meters: value.distance_meters,
    elapsed_time_seconds: value.elapsed_time_seconds,
    moving_time_seconds: moving,
    elevation_difference_meters: elevation,
    average_speed_mps: speed,
    average_heart_rate: heartRate,
  };
}

function parseLap(value: unknown): ActivityLap | null {
  if (!isRecord(value)) return null;
  const moving = nullable(value.moving_time_seconds, isCount);
  const elevation = nullable(value.elevation_gain_meters, isFiniteNumber);
  const speed = nullable(value.average_speed_mps, isCount);
  const heartRate = nullable(value.average_heart_rate, isCount);
  const maxHeartRate = nullable(value.max_heart_rate, isCount);
  const power = nullable(value.average_power, isCount);
  if (
    !Number.isInteger(value.index) ||
    !isCount(value.distance_meters) ||
    !isCount(value.elapsed_time_seconds) ||
    moving === undefined ||
    elevation === undefined ||
    speed === undefined ||
    heartRate === undefined ||
    maxHeartRate === undefined ||
    power === undefined
  ) {
    return null;
  }
  return {
    index: value.index as number,
    distance_meters: value.distance_meters,
    elapsed_time_seconds: value.elapsed_time_seconds,
    moving_time_seconds: moving,
    elevation_gain_meters: elevation,
    average_speed_mps: speed,
    average_heart_rate: heartRate,
    max_heart_rate: maxHeartRate,
    average_power: power,
  };
}

/** Every item of `value` read by `parse`, or null when it is not a list or one item is malformed. */
function parseList<T>(value: unknown, parse: (item: unknown) => T | null): T[] | null {
  if (!Array.isArray(value)) return null;
  const items: T[] = [];
  for (const item of value) {
    const parsed = parse(item);
    if (parsed === null) return null;
    items.push(parsed);
  }
  return items;
}

/**
 * Read a `GET /api/me/activities/{provider}/{activity_id}` body.
 *
 * Returns `null` for any other shape — a malformed split included: a figure
 * the view cannot trust is a contract bug to surface, not a row to drop.
 */
export function parseActivityDetailResponse(body: unknown): ActivityDetailResponse | null {
  if (!isRecord(body)) return null;
  const activity = parseHomeActivity(body.activity);
  const figures = {
    average_heart_rate: nullable(body.average_heart_rate, isCount),
    max_heart_rate: nullable(body.max_heart_rate, isCount),
    average_speed_mps: nullable(body.average_speed_mps, isCount),
    max_speed_mps: nullable(body.max_speed_mps, isCount),
    average_power: nullable(body.average_power, isCount),
    calories: nullable(body.calories, isCount),
  };
  const splits = parseList(body.splits, parseSplit);
  const laps = parseList(body.laps, parseLap);
  const conversationId = nullable(body.conversation_id, isString);
  if (
    activity === null ||
    splits === null ||
    laps === null ||
    conversationId === undefined ||
    Object.values(figures).includes(undefined)
  ) {
    return null;
  }
  return {
    activity,
    average_heart_rate: figures.average_heart_rate ?? null,
    max_heart_rate: figures.max_heart_rate ?? null,
    average_speed_mps: figures.average_speed_mps ?? null,
    max_speed_mps: figures.max_speed_mps ?? null,
    average_power: figures.average_power ?? null,
    calories: figures.calories ?? null,
    splits,
    laps,
    conversation_id: conversationId,
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
 * one that is set is well formed. `pending` is read as
 * {@link ActivityRoutePending}, with the bound it carries when that is a
 * non-negative number.
 */
export function parseActivityRouteResponse(body: unknown): ActivityRouteAnswer | null {
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
  if (body.reason === 'pending') {
    const within = body.settles_within_secs;
    return {
      route: null,
      reason: 'pending',
      settles_within_secs: typeof within === 'number' && Number.isFinite(within) && within >= 0 ? within : null,
    };
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

function parseFormReading(value: unknown): FormReading | null {
  if (!isRecord(value)) return null;
  const band = FORM_BANDS.find((known) => known === value.band);
  const pct = value.pct_of_fitness;
  if (band === undefined || !(pct === null || (typeof pct === 'number' && Number.isFinite(pct)))) {
    return null;
  }
  return { band, pct_of_fitness: pct };
}

function isWholeCount(value: unknown): value is number {
  return typeof value === 'number' && Number.isInteger(value) && value >= 0;
}

function parseLoadRatio(value: unknown): TrainingLoadRatio | null {
  if (
    !isRecord(value) ||
    typeof value.ratio !== 'number' ||
    !Number.isFinite(value.ratio) ||
    value.ratio < 0 ||
    !isWholeCount(value.acute_days) ||
    !isWholeCount(value.chronic_days)
  ) {
    return null;
  }
  return { ratio: value.ratio, acute_days: value.acute_days, chronic_days: value.chronic_days };
}

/**
 * Read a `GET /api/me/training-status` body.
 *
 * Every key must be present. `form`, `load_ratio` and `recovery_days` are
 * null or well-formed: an omitted one is rejected rather than read as "not
 * enough history", which is an answer the server gives on purpose and the
 * page words as such. A band outside {@link FORM_BANDS} is rejected too — the
 * page has no words for it and must not pick the nearest.
 */
export function parseTrainingStatusResponse(body: unknown): TrainingStatusResponse | null {
  if (
    !isRecord(body) ||
    typeof body.today !== 'string' ||
    !CIVIL_DATE.test(body.today) ||
    !Array.isArray(body.trend)
  ) {
    return null;
  }
  const form = body.form === null ? null : parseFormReading(body.form);
  const loadRatio = body.load_ratio === null ? null : parseLoadRatio(body.load_ratio);
  const recoveryDays = body.recovery_days;
  if (
    (body.form !== null && form === null) ||
    (body.load_ratio !== null && loadRatio === null) ||
    !(recoveryDays === null || isWholeCount(recoveryDays))
  ) {
    return null;
  }
  const trend: FormTrendPoint[] = [];
  for (const entry of body.trend) {
    const reading = parseFormReading(entry);
    if (reading === null || !isRecord(entry) || typeof entry.date !== 'string' || !CIVIL_DATE.test(entry.date)) {
      return null;
    }
    trend.push({ ...reading, date: entry.date });
  }
  return {
    today: body.today,
    form,
    trend,
    load_ratio: loadRatio,
    recovery_days: recoveryDays,
  };
}

/**
 * Body of `GET` and `PUT /api/me/home-preferences`: what the athlete chose
 * about Home, stored per user so the web and the phone show the same page.
 */
export interface HomePreferences {
  /**
   * The athlete set aside the suggestion to build a training plan: Home
   * stops offering it while they have none, until Settings brings it back.
   */
  plan_suggestion_hidden: boolean;
}

/** Read a `/api/me/home-preferences` body, or null when it is not one. */
export function parseHomePreferences(body: unknown): HomePreferences | null {
  if (!isRecord(body) || typeof body.plan_suggestion_hidden !== 'boolean') return null;
  return { plan_suggestion_hidden: body.plan_suggestion_hidden };
}

function isCivilDate(value: unknown): value is string {
  return typeof value === 'string' && CIVIL_DATE.test(value);
}

/**
 * A plan day read for its outline — the date, the session's words and the
 * rest flag every reader branches on; the rest is read the way the plan card
 * reads it, field by field.
 */
function isPlanDay(value: unknown): value is PlanDay {
  return (
    isRecord(value) &&
    isCivilDate(value.date) &&
    typeof value.workout === 'string' &&
    typeof value.sport === 'string' &&
    typeof value.rest === 'boolean'
  );
}

function isPlanWeek(value: unknown): value is PlanWeek {
  return (
    isRecord(value) &&
    isCivilDate(value.week_start) &&
    typeof value.focus === 'string' &&
    typeof value.current === 'boolean' &&
    Array.isArray(value.days) &&
    value.days.every(isPlanDay)
  );
}

/**
 * Read a `GET /api/me/calendar` body.
 *
 * Returns `null` for any body that is not that response: one malformed
 * workout or week rejects it whole, and an omitted `plan_weeks` is rejected
 * rather than read as "no plan", which the calendar words as such.
 */
export function parseCalendarResponse(body: unknown): CalendarResponse | null {
  if (
    !isRecord(body) ||
    !isCivilDate(body.today) ||
    !isCivilDate(body.from) ||
    !isCivilDate(body.to) ||
    !isCivilDate(body.history_start) ||
    !Array.isArray(body.activities) ||
    body.plan_weeks === undefined
  ) {
    return null;
  }
  const activities: CalendarActivity[] = [];
  for (const entry of body.activities) {
    if (!isRecord(entry) || !isCivilDate(entry.date)) {
      return null;
    }
    const activity = parseHomeActivity(entry.activity);
    if (activity === null) {
      return null;
    }
    activities.push({ date: entry.date, activity });
  }
  let planWeeks: PlanWeek[] | null = null;
  if (body.plan_weeks !== null) {
    if (!Array.isArray(body.plan_weeks) || !body.plan_weeks.every(isPlanWeek)) {
      return null;
    }
    planWeeks = body.plan_weeks;
  }
  return {
    today: body.today,
    from: body.from,
    to: body.to,
    history_start: body.history_start,
    activities,
    plan_weeks: planWeeks,
  };
}
