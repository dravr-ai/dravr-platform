// ABOUTME: The localized date and date-and-time formatters for every surface that stamps a saved thing
// ABOUTME: Memory facts, verdicts, invites, billing and the admin tables all read the same clock in the reader's locale

/**
 * A stored moment, as the reader's locale spells it: `1 sept. 2026, 16:28` in
 * French, `Sep 1, 2026, 4:28 PM` in English.
 *
 * Every client surface that shows when something was saved reads it here —
 * memory facts, verdicts, group invites and the admin coach and user tables.
 * Hand-written copies hard-coded `'en-US'`, so an operator reading French saw
 * English dates on every admin table.
 *
 * An unparseable stamp comes back verbatim rather than as `Invalid Date`: the
 * raw value is at least diagnosable.
 */
export function formatDateTime(iso: string, locale: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return new Intl.DateTimeFormat(locale, {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(date);
}

/**
 * A stored day, as the reader's locale spells it: `1 sept. 2026` in French,
 * `Sep 1, 2026` in English.
 *
 * The date-only sibling of {@link formatDateTime}, for a row whose time of day
 * carries nothing — when a member joined, when an address was approved, when
 * an invoice was issued. Numeric year, abbreviated month and numeric day are
 * spelled out rather than taken from `dateStyle: 'medium'`, which some
 * locales render all-numeric (`01.09.2026` in German).
 *
 * An unparseable stamp comes back verbatim, as it does from
 * {@link formatDateTime}.
 */
export function formatDate(iso: string, locale: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return new Intl.DateTimeFormat(locale, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  }).format(date);
}
