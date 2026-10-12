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
  const { cadence } = body as { cadence: unknown };
  if (cadence !== null && !isCheckInCadence(cadence)) return null;
  return { cadence };
}
