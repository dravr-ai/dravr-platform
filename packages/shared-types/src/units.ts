// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The athlete's units wire shape — the Settings choice, the system it resolves to, and the inputs behind it
// ABOUTME: With its parser, so a malformed body is an error rather than a page silently printed in the wrong units

/** The unit system an athlete reads: metric (km, m) or imperial (mi, ft). */
export type UnitSystem = 'metric' | 'imperial';

/** What the athlete chose in Settings; `automatic` lets the provider's setting, then the device locale, decide. */
export type UnitPreference = 'automatic' | UnitSystem;

/** Which input decided the athlete's units. */
export type UnitSource = 'override' | 'provider' | 'locale';

/** `GET` and `PUT /api/me/units`. Every key is always present. */
export interface UnitPreferences {
  /** The athlete's choice in Settings. */
  preference: UnitPreference;
  /** The system every distance, elevation and pace is printed in. */
  units: UnitSystem;
  /** Which input decided `units`. */
  source: UnitSource;
  /** The provider whose own unit setting is stored (`strava`), or null. */
  provider: string | null;
  /** That provider's setting, or null when none was read. */
  provider_units: UnitSystem | null;
  /** The locale tag the athlete's device last stored (`en-US`), or null. */
  device_locale: string | null;
}

/**
 * Body of `PUT /api/me/units`: the Settings choice, the device's locale, or
 * both. Each is stored only when sent, so reporting a locale never rewrites
 * the choice; a body naming neither is refused.
 */
export type UnitPreferencesUpdate =
  | {
      preference: UnitPreference;
      /** The locale the device reports, stored so the agent writes in the same units. */
      device_locale?: string;
    }
  | { device_locale: string };

const UNIT_SYSTEMS: readonly string[] = ['metric', 'imperial'];
const UNIT_PREFERENCES: readonly string[] = ['automatic', 'metric', 'imperial'];
const UNIT_SOURCES: readonly string[] = ['override', 'provider', 'locale'];

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isUnitSystem(value: unknown): value is UnitSystem {
  return typeof value === 'string' && UNIT_SYSTEMS.includes(value);
}

function isNullableString(value: unknown): value is string | null {
  return value === null || typeof value === 'string';
}

/** Read a `/api/me/units` body, or null when it is not one. */
export function parseUnitPreferences(body: unknown): UnitPreferences | null {
  if (!isRecord(body)) return null;
  const { preference, units, source, provider, provider_units: providerUnits, device_locale: deviceLocale } = body;
  if (
    typeof preference !== 'string' ||
    !UNIT_PREFERENCES.includes(preference) ||
    !isUnitSystem(units) ||
    typeof source !== 'string' ||
    !UNIT_SOURCES.includes(source) ||
    !isNullableString(provider) ||
    !(providerUnits === null || isUnitSystem(providerUnits)) ||
    !isNullableString(deviceLocale)
  ) {
    return null;
  }
  return {
    preference: preference as UnitPreference,
    units,
    source: source as UnitSource,
    provider,
    provider_units: providerUnits,
    device_locale: deviceLocale,
  };
}
