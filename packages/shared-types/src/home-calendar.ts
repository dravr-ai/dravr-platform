// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Civil-date arithmetic behind Home's calendar — the strip's weeks, a month's grid, how far it pages, one day's entries
// ABOUTME: Pure functions over YYYY-MM-DD strings, so both Home screens' strips and month sheets place every day alike

import type { CalendarResponse, HomeActivity } from './home.js';
import type { WorkoutPlan } from './workout-plan.js';
import { addCivilDays, mondayOf, planWeeksDayOn, type PlanDayLookup } from './plan-calendar.js';

const DAYS_PER_WEEK = 7;

/** The seven days, Monday to Sunday, of the week starting `monday`. */
export function weekDays(monday: string): string[] {
  return Array.from({ length: DAYS_PER_WEEK }, (_, index) => addCivilDays(monday, index));
}

/** `YYYY-MM-01` of the month `date` falls in. */
export function firstOfMonth(date: string): string {
  return `${date.slice(0, 8)}01`;
}

/** The first day of the month `months` after the one starting `first` (before it, when negative). */
export function shiftMonth(first: string, months: number): string {
  const year = Number(first.slice(0, 4));
  const month = Number(first.slice(5, 7)) - 1 + months;
  const shifted = new Date(Date.UTC(year, month, 1));
  return `${String(shifted.getUTCFullYear()).padStart(4, '0')}-${String(shifted.getUTCMonth() + 1).padStart(2, '0')}-01`;
}

/**
 * The month grid for the month starting `first`: whole Monday-to-Sunday
 * weeks from the one holding the 1st to the one holding the last day — four
 * to six of them, never more than the 42 days one calendar read spans.
 */
export function monthGrid(first: string): string[][] {
  const next = shiftMonth(first, 1);
  const weeks: string[][] = [];
  for (let monday = mondayOf(first); monday < next; monday = addCivilDays(monday, DAYS_PER_WEEK)) {
    weeks.push(weekDays(monday));
  }
  return weeks;
}

/**
 * The last Monday the calendar pages forward to: the week holding the plan's
 * last day when it says where that is — its season end, or its last stored
 * week — and never before next week, so an athlete with no plan still sees
 * the week ahead.
 */
export function lastMonday(plan: WorkoutPlan | null, today: string): string {
  const nextMonday = addCivilDays(mondayOf(today), DAYS_PER_WEEK);
  if (plan === null) return nextMonday;
  const candidates = [nextMonday];
  if (plan.season_end !== undefined) {
    try {
      candidates.push(mondayOf(plan.season_end));
    } catch (error) {
      // A season end that is not a calendar day says nothing about how far to page.
      if (!(error instanceof RangeError)) throw error;
    }
  }
  const lastShown = plan.weeks.at(-1);
  if (lastShown !== undefined) {
    try {
      candidates.push(addCivilDays(lastShown.week_start, DAYS_PER_WEEK * plan.weeks_deferred));
    } catch (error) {
      if (!(error instanceof RangeError)) throw error;
    }
  }
  return candidates.reduce((latest, candidate) => (candidate > latest ? candidate : latest));
}

/** What the calendar knows about one day. */
export interface CalendarDay {
  date: string;
  /** The workouts that started on the day, oldest first. */
  activities: HomeActivity[];
  /** What the plan holds on the day; null when there is no active plan. */
  plan: PlanDayLookup | null;
  /** The day is before the first one the activity cache holds in full: unknown, not empty. */
  beforeHistory: boolean;
}

/** One day out of a calendar answer. */
export function calendarDay(response: CalendarResponse, date: string): CalendarDay {
  return {
    date,
    activities: response.activities.filter((entry) => entry.date === date).map((entry) => entry.activity),
    plan: response.plan_weeks === null ? null : planWeeksDayOn(response.plan_weeks, date),
    beforeHistory: date < response.history_start,
  };
}

/** Whole minutes the day's workouts lasted, together. */
export function activityMinutes(activities: readonly HomeActivity[]): number {
  return Math.round(activities.reduce((sum, activity) => sum + activity.duration_seconds, 0) / 60);
}
