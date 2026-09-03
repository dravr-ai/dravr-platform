// ABOUTME: One localized date-and-time formatter for every surface that stamps a saved thing
// ABOUTME: Memory facts, verdicts and the admin tables all read the same clock in the reader's locale

/**
 * A stored moment, as the reader's locale spells it: `1 sept. 2026, 16:28` in
 * French, `Sep 1, 2026, 4:28 PM` in English.
 *
 * Written by hand at fifteen sites before this: three copies of the same
 * `Intl.DateTimeFormat(language, { dateStyle: 'medium', timeStyle: 'short' })`
 * in the memory panels and the verdict drawer, and twelve admin components
 * that each hard-coded `'en-US'` — so an operator reading French saw English
 * dates on every admin table.
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
