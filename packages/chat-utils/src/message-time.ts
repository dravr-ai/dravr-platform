// ABOUTME: The clock and the day a message row shows — one rule for web and mobile, in the reader's locale
// ABOUTME: A 24-hour time inside the bubble, a day pill between days, and the grouping window for consecutive rows

/** Rows from the same author closer than this are drawn as one group. */
export const MESSAGE_GROUP_WINDOW_MS = 5 * 60 * 1000;

const DAY_MS = 24 * 60 * 60 * 1000;

function startOfDay(date: Date): number {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
}

/**
 * The time a bubble shows: `16:18`, on the same 24-hour clock the list row
 * uses, spelled by `Intl` in `locale`. An unparseable stamp yields an empty
 * string rather than `Invalid Date` inside a bubble.
 */
export function formatMessageTime(iso: string, locale: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return '';
  return new Intl.DateTimeFormat(locale, {
    hour: '2-digit',
    minute: '2-digit',
    hourCycle: 'h23',
  }).format(date);
}

/** The local calendar day a stamp falls on, `YYYY-MM-DD`; empty when unparseable. */
export function localDayKey(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return '';
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const day = String(date.getDate()).padStart(2, '0');
  return `${date.getFullYear()}-${month}-${day}`;
}

/**
 * What the day pill above a run of messages says.
 *
 * Today and yesterday are words the client owns (they are translated by its
 * own corpus); any earlier day is the full date, spelled by `Intl` in
 * `locale`, with the year only once it is not this one.
 */
export type DayLabel =
  | { kind: 'today' }
  | { kind: 'yesterday' }
  | { kind: 'date'; label: string };

export function dayLabelFor(iso: string, locale: string, now: Date = new Date()): DayLabel {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return { kind: 'date', label: '' };
  const dayDiff = Math.round((startOfDay(now) - startOfDay(date)) / DAY_MS);
  if (dayDiff <= 0) return { kind: 'today' };
  if (dayDiff === 1) return { kind: 'yesterday' };
  const label = new Intl.DateTimeFormat(
    locale,
    date.getFullYear() === now.getFullYear()
      ? { weekday: 'long', day: 'numeric', month: 'long' }
      : { weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' },
  ).format(date);
  return { kind: 'date', label };
}

/**
 * Whether two consecutive rows belong to one visual group: the same author,
 * on the same day, within {@link MESSAGE_GROUP_WINDOW_MS} of each other.
 */
export function isSameMessageGroup(
  previous: { role: string; created_at?: string | null },
  next: { role: string; created_at?: string | null },
): boolean {
  if (previous.role !== next.role) return false;
  if (!previous.created_at || !next.created_at) return false;
  const a = Date.parse(previous.created_at);
  const b = Date.parse(next.created_at);
  if (Number.isNaN(a) || Number.isNaN(b)) return false;
  if (localDayKey(previous.created_at) !== localDayKey(next.created_at)) return false;
  return Math.abs(b - a) <= MESSAGE_GROUP_WINDOW_MS;
}
