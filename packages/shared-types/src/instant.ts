// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Reads an RFC 3339 instant off the wire as epoch milliseconds, the same on every JavaScript engine
// ABOUTME: The server writes up to nine fractional digits, ECMA-262 promises Date.parse three, so only the spec format reaches it

/**
 * An RFC 3339 instant: the date, `T`, the time, a fraction of any length or
 * none, then `Z` or a `±HH:MM` offset. `T` and `Z` may be lower case, as
 * RFC 3339 allows.
 */
const RFC3339_INSTANT = /^(\d{4}-\d{2}-\d{2})[Tt](\d{2}:\d{2}:\d{2})(?:\.(\d+))?([Zz]|[+-]\d{2}:\d{2})$/;

/**
 * The epoch milliseconds of an RFC 3339 instant, or `NaN` when `value` is not one.
 *
 * The server serializes its timestamps with chrono, which writes as many
 * fractional digits as the value holds: six from a Postgres column, up to
 * nine from the clock. `Date.parse` is only required to read the ECMA-262
 * date-time format, whose fraction is exactly three digits; any other string
 * is the engine's call, and the phone runs Hermes, not the V8 that the
 * browser and the test runner share. So the fraction is cut (never rounded)
 * or padded to three digits, and only that format reaches `Date.parse`.
 * Instants less than a millisecond apart therefore read as the same instant.
 */
export function instantMs(value: string): number {
  const match = RFC3339_INSTANT.exec(value);
  if (match === null) return Number.NaN;
  const [, date, time, fraction = '', offset = ''] = match;
  const millis = fraction.slice(0, 3).padEnd(3, '0');
  return Date.parse(`${date}T${time}.${millis}${offset.toUpperCase()}`);
}

/** Whether `value` is an RFC 3339 instant {@link instantMs} can read. */
export function isInstant(value: unknown): value is string {
  return typeof value === 'string' && !Number.isNaN(instantMs(value));
}
