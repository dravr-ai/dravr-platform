// ABOUTME: The day arithmetic and the 24-hour clock every chat surface agrees on
// ABOUTME: Private to chat-utils — the list row, the message bubble and the default title share it

/** One calendar day, for bucketing two stamps into "how many days apart". */
export const DAY_MS = 24 * 60 * 60 * 1000;

/** Midnight local time on the day a stamp falls on. */
export function startOfDay(date: Date): number {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
}

/** Whole local days between two moments — 0 today, 1 yesterday. */
export function dayDiff(now: Date, date: Date): number {
  return Math.round((startOfDay(now) - startOfDay(date)) / DAY_MS);
}

/**
 * The clock every chat surface shows: 24-hour, whatever the locale's own
 * convention, spelled by `Intl` so the digits are the reader's.
 *
 * The list row built this by hand with a `pad2` helper while the bubble and
 * the default title used `Intl` with `hourCycle: 'h23'` — three spellings of
 * one rule, and the row's comment already said it had to agree with the title.
 */
export function clock24(date: Date, locale: string): string {
  return new Intl.DateTimeFormat(locale, {
    hour: '2-digit',
    minute: '2-digit',
    hourCycle: 'h23',
  }).format(date);
}
