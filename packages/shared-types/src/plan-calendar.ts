// ABOUTME: Civil-date reads over a plan card — what the plan holds on a date: its sessions, a rest day, or nothing
// ABOUTME: One answer for both Home screens, so a rest day and a day the plan never reached are never confused

import type { PlanDay, PlanPhase, PlanWeek, WorkoutPlan } from './workout-plan.js';

const CIVIL_DATE = /^(\d{4})-(\d{2})-(\d{2})$/;
const DAY_MS = 86400000;
const DAYS_PER_WEEK = 7;

function pad(value: number, width: number): string {
  return String(value).padStart(width, '0');
}

function formatCivil(ms: number): string {
  const date = new Date(ms);
  return `${pad(date.getUTCFullYear(), 4)}-${pad(date.getUTCMonth() + 1, 2)}-${pad(date.getUTCDate(), 2)}`;
}

/**
 * Midnight UTC of a `YYYY-MM-DD` date, used only as a day counter: a civil
 * date has no timezone, and counting in UTC keeps a daylight-saving change
 * from turning one day into 23 or 25 hours.
 *
 * @throws RangeError for a string that is not a real calendar date.
 */
function civilDayMs(date: string): number {
  const match = CIVIL_DATE.exec(date);
  if (match === null) {
    throw new RangeError(`not a civil date (YYYY-MM-DD): ${date}`);
  }
  const ms = Date.UTC(Number(match[1]), Number(match[2]) - 1, Number(match[3]));
  // Date.UTC rolls 2026-02-30 over into March; a date that does not come
  // back unchanged was never on the calendar.
  if (formatCivil(ms) !== date) {
    throw new RangeError(`not a calendar date (it rolls over to ${formatCivil(ms)}): ${date}`);
  }
  return ms;
}

/**
 * The civil date `days` after `date` (before it, when negative) — tomorrow is
 * `addCivilDays(today, 1)`. Pure calendar arithmetic, never a local `Date`,
 * so the answer does not depend on the device's timezone.
 *
 * @throws RangeError when `date` is not a `YYYY-MM-DD` calendar date or `days` is not an integer.
 */
export function addCivilDays(date: string, days: number): string {
  if (!Number.isInteger(days)) {
    throw new RangeError(`days must be an integer (got ${days})`);
  }
  return formatCivil(civilDayMs(date) + days * DAY_MS);
}

/**
 * The Monday of the week `date` falls in — the `week_start` a plan week
 * carries, and the first cell of a Monday-to-Sunday strip.
 *
 * @throws RangeError when `date` is not a `YYYY-MM-DD` calendar date.
 */
export function mondayOf(date: string): string {
  const weekday = new Date(civilDayMs(date)).getUTCDay();
  // getUTCDay counts from Sunday = 0; Monday-based it is (weekday + 6) % 7.
  return addCivilDays(date, -((weekday + 6) % DAYS_PER_WEEK));
}

/**
 * What the plan holds on one date. A date may hold several entries, in the
 * order they are done — two sessions, or a rest entry beside an optional one
 * (carnet#1716) — so `days` lists every entry on the date, rest included.
 *
 * - `session` — at least one entry is a workout.
 * - `rest` — the date holds only its rest entry: a day kept free.
 * - `uncovered` — the plan says nothing about the date: before it starts,
 *   after its shown weeks, or a gap in a week. Never shown as rest, because
 *   the plan did not say rest.
 */
export type PlanDayLookup =
  | { kind: 'session'; days: PlanDay[]; week: PlanWeek }
  | { kind: 'rest'; days: PlanDay[]; week: PlanWeek }
  | { kind: 'uncovered' };

/** The workouts a date holds, in order: its entries without the rest entry. */
export function planSessionsOf(lookup: PlanDayLookup | null): PlanDay[] {
  if (lookup === null || lookup.kind === 'uncovered') return [];
  return lookup.days.filter((day) => !day.rest);
}

/** Look `date` (`YYYY-MM-DD`) up in the plan's shown weeks. */
export function planDayOn(plan: WorkoutPlan, date: string): PlanDayLookup {
  return planWeeksDayOn(plan.weeks, date);
}

/**
 * Look `date` (`YYYY-MM-DD`) up in any run of plan weeks — the card's shown
 * fortnight, or the weeks a calendar read answered for the days it spans.
 */
export function planWeeksDayOn(weeks: readonly PlanWeek[], date: string): PlanDayLookup {
  for (const week of weeks) {
    const days = week.days.filter((candidate) => candidate.date === date);
    if (days.length > 0) {
      return days.every((day) => day.rest) ? { kind: 'rest', days, week } : { kind: 'session', days, week };
    }
  }
  return { kind: 'uncovered' };
}

/**
 * The distance a planned session covers, in metres, or null when the plan
 * does not say: a session with no steps, or one where any step is set by
 * time alone — a sum over the rest would be a part of the session passed off
 * as the whole. A step's distance counts once per `repeat`.
 */
export function planDayDistanceMeters(day: PlanDay): number | null {
  if (day.rest || day.steps === undefined || day.steps.length === 0) {
    return null;
  }
  let total = 0;
  for (const step of day.steps) {
    const distance = step.distance_meters;
    if (typeof distance !== 'number' || !Number.isFinite(distance) || distance <= 0) {
      return null;
    }
    const repeat = typeof step.repeat === 'number' && step.repeat > 1 ? Math.floor(step.repeat) : 1;
    total += distance * repeat;
  }
  return total;
}

/**
 * Which week of `phase` the date falls in, counting from 1 — the "week 3" of
 * "Build · week 3" — or null when the date is outside the phase.
 *
 * @throws RangeError when `date` or the phase's own dates are not `YYYY-MM-DD` calendar dates.
 */
export function phaseWeekOn(phase: PlanPhase, date: string): number | null {
  const day = civilDayMs(date);
  const start = civilDayMs(phase.start);
  if (day < start || (phase.end !== undefined && day >= civilDayMs(phase.end))) {
    return null;
  }
  return Math.floor((day - start) / (DAYS_PER_WEEK * DAY_MS)) + 1;
}
