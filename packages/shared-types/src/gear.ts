// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The gear ledger wire shapes — the shoes the athlete declares, their distance, and the pair one activity was run in
// ABOUTME: With their parsers and the bounds the server holds a declaration to, so a client refuses a bad field before sending it

/**
 * The sports a pair of shoes is declared for, as the server spells them
 * (`shoe_sport_slug` in pierre-core). A `run` pair also takes treadmill runs,
 * and trail runs when no `trail_running` pair does.
 */
export const GEAR_SPORTS = ['run', 'trail_running', 'walk', 'hike'] as const;

/** One of {@link GEAR_SPORTS}. */
export type GearSport = (typeof GEAR_SPORTS)[number];

/** What a piece of gear is. Shoes are the only kind. */
export type GearKind = 'shoe';

/**
 * How an activity came to be attributed to a pair: `declared` when the
 * athlete chose (a pair, or none of theirs), `default` when the ledger gave it
 * its sport's default pair.
 */
export type GearAttributionSource = 'declared' | 'default';

/**
 * The warning distance a pair takes unless the athlete sets another, in
 * metres (`DEFAULT_SHOE_WARN_DISTANCE_METERS` in pierre-core): 500 km, the
 * threshold the athlete who asked for the ledger named.
 */
export const GEAR_DEFAULT_WARN_DISTANCE_METERS = 500_000;

/**
 * The largest distance a pair can be declared with or warned at, in metres
 * (`MAX_GEAR_DISTANCE_METERS` in pierre-core); past it the server answers 400.
 */
export const GEAR_MAX_DISTANCE_METERS = 10_000_000;

/** The longest name a pair carries, in characters once trimmed (`MAX_GEAR_NAME_CHARS`). */
export const GEAR_NAME_MAX_CHARS = 80;

/** One pair, as every gear endpoint answers with it. */
export interface GearItem {
  id: string;
  kind: GearKind;
  /** The athlete's name for it. Athlete-written, untrusted text. */
  name: string;
  /** The sport it is worn for. */
  sport: GearSport;
  /** The distance it had when declared, in metres. */
  initial_distance_meters: number;
  /** The distance at which it reaches its warning, in metres. */
  warn_distance_meters: number;
  /** Its distance: declared plus every activity attributed to it, in metres. */
  distance_meters: number;
  /** How many activities are attributed to it. */
  activity_count: number;
  /**
   * Whether `distance_meters` is at or past `warn_distance_meters`. A fact
   * about the pair, never a claim about the athlete's body.
   */
  warn_reached: boolean;
  /** Whether the athlete pinned it as its sport's default. */
  pinned: boolean;
  /** Whether a new activity of its sport accrues to it now: pinned, or used last, or declared last. */
  is_default: boolean;
  /** When it was declared, RFC 3339; it takes no activity that started before. */
  added_at: string;
  /** When it was retired, RFC 3339; `null` while it is in use. */
  retired_at: string | null;
  /** When the latest activity attributed to it started, RFC 3339. */
  last_used_at: string | null;
}

/** `GET /api/me/gear`. */
export interface GearListResponse {
  /** Every pair, retired ones included, newest first. */
  gear: GearItem[];
}

/**
 * `POST /api/me/gear`. The server refuses any other field; a distance left
 * out is 0, a warning left out is {@link GEAR_DEFAULT_WARN_DISTANCE_METERS}.
 */
export interface CreateGearRequest {
  name: string;
  sport: GearSport;
  initial_distance_meters?: number;
  warn_distance_meters?: number;
  pinned?: boolean;
}

/**
 * `PATCH /api/me/gear/{id}`: each field present replaces what was declared,
 * each one absent keeps it. Pinning a retired pair answers 400.
 */
export type UpdateGearRequest = Partial<CreateGearRequest>;

/** `GET` and `PUT /api/me/activities/{provider}/{activity_id}/gear`. */
export interface ActivityGearResponse {
  /** The pair, `null` when none is attributed or the athlete declared none. */
  gear_id: string | null;
  /** How it was attributed, `null` when nothing is. */
  source: GearAttributionSource | null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isDistance(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0;
}

function isInstant(value: unknown): value is string {
  return typeof value === 'string' && !Number.isNaN(Date.parse(value));
}

function isNullableInstant(value: unknown): value is string | null {
  return value === null || isInstant(value);
}

/** Whether `value` names one of {@link GEAR_SPORTS}. */
export function isGearSport(value: unknown): value is GearSport {
  return typeof value === 'string' && (GEAR_SPORTS as readonly string[]).includes(value);
}

function isSource(value: unknown): value is GearAttributionSource {
  return value === 'declared' || value === 'default';
}

/** Read one pair, or `null` for any other shape. */
export function parseGearItem(value: unknown): GearItem | null {
  if (
    !isRecord(value) ||
    typeof value.id !== 'string' ||
    value.id === '' ||
    value.kind !== 'shoe' ||
    typeof value.name !== 'string' ||
    !isGearSport(value.sport) ||
    !isDistance(value.initial_distance_meters) ||
    !isDistance(value.warn_distance_meters) ||
    !isDistance(value.distance_meters) ||
    !isDistance(value.activity_count) ||
    !Number.isInteger(value.activity_count) ||
    typeof value.warn_reached !== 'boolean' ||
    typeof value.pinned !== 'boolean' ||
    typeof value.is_default !== 'boolean' ||
    !isInstant(value.added_at) ||
    !isNullableInstant(value.retired_at) ||
    !isNullableInstant(value.last_used_at)
  ) {
    return null;
  }
  return {
    id: value.id,
    kind: value.kind,
    name: value.name,
    sport: value.sport,
    initial_distance_meters: value.initial_distance_meters,
    warn_distance_meters: value.warn_distance_meters,
    distance_meters: value.distance_meters,
    activity_count: value.activity_count,
    warn_reached: value.warn_reached,
    pinned: value.pinned,
    is_default: value.is_default,
    added_at: value.added_at,
    retired_at: value.retired_at,
    last_used_at: value.last_used_at,
  };
}

/** Read a `GET /api/me/gear` body, or `null` when any pair in it is malformed. */
export function parseGearListResponse(body: unknown): GearListResponse | null {
  if (!isRecord(body) || !Array.isArray(body.gear)) {
    return null;
  }
  const gear: GearItem[] = [];
  for (const entry of body.gear) {
    const pair = parseGearItem(entry);
    if (pair === null) return null;
    gear.push(pair);
  }
  return { gear };
}

/**
 * Read an activity's gear body, or `null` for any other shape — a pair with
 * no source among them, since the server never attributes one without saying
 * how.
 */
export function parseActivityGearResponse(body: unknown): ActivityGearResponse | null {
  if (!isRecord(body)) {
    return null;
  }
  const gearId = body.gear_id;
  const source = body.source;
  if (!(gearId === null || (typeof gearId === 'string' && gearId !== ''))) return null;
  if (!(source === null || isSource(source))) return null;
  if (gearId !== null && source === null) return null;
  return { gear_id: gearId, source };
}
