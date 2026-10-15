// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The athlete's units wire shape — metric unless they chose imperial in Settings (carnet#835, carnet#1044)
// ABOUTME: With its parser, so a malformed body is an error rather than a page silently printed in the wrong units

/** The unit system an athlete reads: metric (km, m) or imperial (mi, ft). */
export type UnitSystem = 'metric' | 'imperial';

/** `GET` and `PUT /api/me/units`, the `PUT` body included: metric unless the athlete chose imperial. */
export interface UnitPreferences {
  /** The system every distance, elevation and pace is printed in. */
  units: UnitSystem;
}

const UNIT_SYSTEMS: readonly string[] = ['metric', 'imperial'];

/** Read a `/api/me/units` body, or null when it is not one. */
export function parseUnitPreferences(body: unknown): UnitPreferences | null {
  if (typeof body !== 'object' || body === null || Array.isArray(body)) return null;
  const { units } = body as Record<string, unknown>;
  if (typeof units !== 'string' || !UNIT_SYSTEMS.includes(units)) return null;
  return { units: units as UnitSystem };
}
