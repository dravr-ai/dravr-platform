// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The check-in cadence wire shape — how often the coach writes first: daily, three a week, weekly or off
// ABOUTME: With its parser, so a malformed body is an error rather than a choice silently read as another (carnet#831)

/** Every cadence, in the order the choice is offered. `off` is a real answer. */
export const CHECK_IN_CADENCES = ['daily', 'three_per_week', 'weekly', 'off'] as const;

/** How often the coach checks in with the athlete on its own. */
export type CheckInCadence = (typeof CHECK_IN_CADENCES)[number];

/** `GET` and `PUT /api/me/check-in`. */
export interface CheckInPreference {
  /** The cadence the athlete chose, or null when they never chose (never checked in on). */
  cadence: CheckInCadence | null;
  /**
   * The local hour (0–23) a due check-in goes out from — the server's
   * `CHECK_IN_HOUR`, sent so no client keeps a copy. Optional because a
   * choice is written into the cache before the server answers, and that
   * optimistic entry carries the cadence alone.
   */
  local_hour?: number;
}

/** Whether `value` is an hour of the day, 0 to 23. */
function isLocalHour(value: unknown): value is number {
  return typeof value === 'number' && Number.isInteger(value) && value >= 0 && value <= 23;
}

/** Whether `value` is one of the four cadences. */
export function isCheckInCadence(value: unknown): value is CheckInCadence {
  return typeof value === 'string' && (CHECK_IN_CADENCES as readonly string[]).includes(value);
}

/** Read a `/api/me/check-in` body, or null when it is not one. */
export function parseCheckInPreference(body: unknown): CheckInPreference | null {
  if (typeof body !== 'object' || body === null || Array.isArray(body) || !('cadence' in body)) {
    return null;
  }
  const { cadence, local_hour: localHour } = body as { cadence: unknown; local_hour?: unknown };
  if (cadence !== null && !isCheckInCadence(cadence)) return null;
  if (localHour === undefined) return { cadence };
  if (!isLocalHour(localHour)) return null;
  return { cadence, local_hour: localHour };
}
