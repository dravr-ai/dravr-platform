// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Distances, elevation, pace and speed in the athlete's unit system — kilometres and metres, or miles and feet
// ABOUTME: The one conversion both clients print through, so a mile on Home, the activity view and the route marks is the same mile

import type { UnitSystem } from '@pierre/shared-types';

import { formatDecimal } from './number-format';

/** The unit system an athlete reads: metric (km, m) or imperial (mi, ft) — the server's `units`. */
export type DistanceUnit = UnitSystem;

/** Metres in a kilometre. */
export const METRES_PER_KILOMETRE = 1000;
/** Metres in a statute mile. */
export const METRES_PER_MILE = 1609.344;
/** Metres in an international foot. */
export const METRES_PER_FOOT = 0.3048;

const SECONDS_PER_HOUR = 3600;

/** Metres in one distance unit of `unit`: a kilometre or a mile. */
export function metresPerDistanceUnit(unit: DistanceUnit): number {
  return unit === 'imperial' ? METRES_PER_MILE : METRES_PER_KILOMETRE;
}

/** A distance in metres, counted in kilometres or miles. */
export function distanceInUnit(meters: number, unit: DistanceUnit): number {
  return meters / metresPerDistanceUnit(unit);
}

/** The symbol a distance in `unit` is printed with: `km` or `mi`. */
export function distanceSymbol(unit: DistanceUnit): 'km' | 'mi' {
  return unit === 'imperial' ? 'mi' : 'km';
}

/**
 * A distance in metres as `unit` reads it, to `digits` decimals in the
 * notation of `language`: `92.0 km`, `57.2 mi`, `92,0 km` in French.
 */
export function formatDistance(meters: number, unit: DistanceUnit, digits: number, language: string): string {
  return `${formatDecimal(distanceInUnit(meters, unit), digits, language)} ${distanceSymbol(unit)}`;
}

/**
 * A distance the way a sentence says it: `15 km`, `12.5 km`, `9.3 mi` — to
 * the tenth of a kilometre or mile, without the decimal a whole figure does
 * not need.
 */
export function formatSpokenDistance(meters: number, unit: DistanceUnit, language: string): string {
  const tenths = Math.round(distanceInUnit(meters, unit) * 10);
  return `${formatDecimal(tenths / 10, tenths % 10 === 0 ? 0 : 1, language)} ${distanceSymbol(unit)}`;
}

/** An elevation in metres as `unit` reads it, to the whole metre or foot: `120 m`, `394 ft`. */
export function formatElevation(meters: number, unit: DistanceUnit, language: string): string {
  const value = unit === 'imperial' ? meters / METRES_PER_FOOT : meters;
  return `${formatDecimal(Math.round(value), 0, language)} ${unit === 'imperial' ? 'ft' : 'm'}`;
}

/**
 * A climb or a descent as `unit` reads it, with its sign — `+4 m`, `-10 ft` —
 * the sign written here rather than by `Intl`, which spells a minus
 * differently by locale; a change that rounds to zero carries none.
 */
export function formatSignedElevation(meters: number, unit: DistanceUnit, language: string): string {
  const rounded = Math.round(unit === 'imperial' ? meters / METRES_PER_FOOT : meters);
  const sign = rounded > 0 ? '+' : rounded < 0 ? '-' : '';
  return `${sign}${formatDecimal(Math.abs(rounded), 0, language)} ${unit === 'imperial' ? 'ft' : 'm'}`;
}

/** Seconds to cover one kilometre or one mile at `metersPerSecond`. */
export function secondsPerDistanceUnit(metersPerSecond: number, unit: DistanceUnit): number {
  return metresPerDistanceUnit(unit) / metersPerSecond;
}

/** The symbol a pace in `unit` is printed with: `/km` or `/mi`. */
export function paceSymbol(unit: DistanceUnit): '/km' | '/mi' {
  return unit === 'imperial' ? '/mi' : '/km';
}

/** A speed in metres per second as `unit` reads it, to one decimal: `28.4 km/h`, `17.6 mph`. */
export function formatSpeedIn(metersPerSecond: number, unit: DistanceUnit, language: string): string {
  const perHour = (metersPerSecond * SECONDS_PER_HOUR) / metresPerDistanceUnit(unit);
  return `${formatDecimal(perHour, 1, language)} ${unit === 'imperial' ? 'mph' : 'km/h'}`;
}
