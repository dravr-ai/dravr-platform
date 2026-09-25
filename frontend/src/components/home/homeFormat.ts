// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Formatting for the Home page — plan calendar days, activity instants, sport labels and activity figures
// ABOUTME: A plan date is a calendar day and is never shifted by the device timezone; an activity start is an instant and is

import type { TFunction } from '@pierre/i18n';
import type { HomeActivity, PlanDay, PlanDayLookup, PlanPhase } from '@pierre/shared-types';
import { addCivilDays, mondayOf, phaseWeekOn } from '@pierre/shared-types';
import { activitySportLabelKey } from '@pierre/shared-constants';
import { formatDistance, formatDuration } from '@pierre/domain-utils';

const CIVIL_DATE = /^(\d{4})-(\d{2})-(\d{2})$/;
const DAYS_PER_WEEK = 7;

/** The weekday, day and month a chat draft names a day by — "Tuesday 23 September", "mardi 23 septembre". */
export const DRAFT_DATE: Intl.DateTimeFormatOptions = { weekday: 'long', day: 'numeric', month: 'long' };

/** The short form a row or a heading carries — "Tue 23 Sep". */
export const ROW_DATE: Intl.DateTimeFormatOptions = { weekday: 'short', day: 'numeric', month: 'short' };

/** When the activity cache last heard from a provider — a 24-hour clock in every locale, like the chat list. */
const SYNC_TIME: Intl.DateTimeFormatOptions = {
  day: 'numeric',
  month: 'short',
  hour: '2-digit',
  minute: '2-digit',
  hourCycle: 'h23',
};

/**
 * A `YYYY-MM-DD` calendar day in the app's language.
 *
 * Formatted at midnight UTC in UTC, so the weekday is the plan's weekday on
 * every device — formatting the same instant in the device's zone would put a
 * Monday session on Sunday for anyone west of Greenwich. A string that is not
 * a calendar day comes back unchanged rather than as a wrong date.
 */
export function formatCivilDate(date: string, language: string, options: Intl.DateTimeFormatOptions): string {
  const match = CIVIL_DATE.exec(date);
  if (match === null) return date;
  const ms = Date.UTC(Number(match[1]), Number(match[2]) - 1, Number(match[3]));
  if (Number.isNaN(ms)) return date;
  return new Intl.DateTimeFormat(language, { ...options, timeZone: 'UTC' }).format(new Date(ms));
}

/** An RFC 3339 instant in the app's language and the device's own timezone. */
export function formatInstant(instant: string, language: string, options: Intl.DateTimeFormatOptions): string {
  const ms = Date.parse(instant);
  if (Number.isNaN(ms)) return instant;
  return new Intl.DateTimeFormat(language, options).format(new Date(ms));
}

/** "Last synced" time for the activity cache's `as_of`. */
export function formatSyncTime(instant: string, language: string): string {
  return formatInstant(instant, language, SYNC_TIME);
}

/**
 * The sport's label in the app's language, or the wire spelling itself for a
 * sport the shared vocabulary does not know — never a raw catalogue key.
 */
export function sportLabel(t: TFunction, sport: string): string {
  const key = activitySportLabelKey(sport);
  return key === null ? sport : t(key);
}

/** The figures a row prints after the sport: distance, time, climbing — each only when the activity carries it. */
export function activityFigures(activity: HomeActivity): string[] {
  const figures: string[] = [];
  if (activity.distance_meters !== null && activity.distance_meters > 0) {
    figures.push(formatDistance(activity.distance_meters));
  }
  figures.push(formatDuration(activity.duration_seconds));
  if (activity.elevation_gain_meters !== null && activity.elevation_gain_meters > 0) {
    figures.push(`+${Math.round(activity.elevation_gain_meters)} m`);
  }
  return figures;
}

/** The days the Home page reads out of the plan, all computed from the athlete's own today. */
export interface PlanWindow {
  today: string;
  tomorrow: string;
  /** Monday to Sunday of the week `today` falls in. */
  week: string[];
  /** The Monday after that week — the `week_start` of next week's plan week. */
  nextMonday: string;
}

/**
 * The days around `today`, or null when `today` is not a calendar day.
 *
 * The response parser checks the `YYYY-MM-DD` shape but not the calendar, and
 * the civil-date helpers throw on a day like `2026-02-30`; the page shows its
 * load error for that rather than a strip of guessed dates.
 */
export function planWindow(today: string): PlanWindow | null {
  try {
    const monday = mondayOf(today);
    return {
      today,
      tomorrow: addCivilDays(today, 1),
      week: Array.from({ length: DAYS_PER_WEEK }, (_, index) => addCivilDays(monday, index)),
      nextMonday: addCivilDays(monday, DAYS_PER_WEEK),
    };
  } catch (error) {
    if (error instanceof RangeError) return null;
    throw error;
  }
}

/**
 * "Build · week 3" for the phase covering `date`, or null when no phase does.
 *
 * The phase is found by its dates rather than by the `current` flag, which
 * the server sets for its own today; a phase whose dates are not calendar
 * days is skipped rather than allowed to take the page down.
 */
export function phaseWeekLabel(t: TFunction, phases: readonly PlanPhase[], date: string): string | null {
  for (const phase of phases) {
    let week: number | null;
    try {
      week = phaseWeekOn(phase, date);
    } catch (error) {
      if (error instanceof RangeError) continue;
      throw error;
    }
    if (week !== null) {
      return t('home.plan.phaseWeek', {
        phase: t(`plan.card.phase.${phase.kind}`, { defaultValue: phase.kind }),
        week,
      });
    }
  }
  return null;
}

/**
 * The chat draft a tap on a plan day opens, or null for a day the plan does
 * not cover — there is nothing of the plan's to ask about.
 */
export function planDayDraft(t: TFunction, language: string, date: string, lookup: PlanDayLookup): string | null {
  const named = formatCivilDate(date, language, DRAFT_DATE);
  if (lookup.kind === 'session') {
    return t('home.plan.dayDraft', { date: named, workout: lookup.day.workout });
  }
  if (lookup.kind === 'rest') {
    return t('home.plan.restDayDraft', { date: named });
  }
  return null;
}

/** A session's sport, minutes and intensity, in that order, each only when the day carries it. */
export function sessionFacts(t: TFunction, day: PlanDay): string[] {
  // Typed as strings, but the card is checked only for its outline on the
  // way in, so each field is read the way WorkoutPlanCard reads it.
  const facts: unknown[] = [
    typeof day.sport === 'string' ? sportLabel(t, day.sport) : null,
    day.duration_min !== undefined ? `${day.duration_min} min` : null,
    day.intensity,
  ];
  return facts.filter((fact): fact is string => typeof fact === 'string' && fact.length > 0);
}
