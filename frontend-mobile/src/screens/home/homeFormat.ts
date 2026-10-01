// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The words and figures the Home tab prints — civil and instant dates, durations, distances, chat drafts, activity names
// ABOUTME: Pure functions of the app language, so the screen and its tests format a date, a sport and a draft one way

import type { HomeActivity, PlanDay } from '@pierre/shared-types';
import { activitySportLabelKey } from '@pierre/shared-constants';
import { formatDuration } from '@pierre/domain-utils';
import { formatDecimal } from '@pierre/chat-utils';

/** The translator the helpers take; module scope holds no hook. */
export type Translate = (key: string, options?: Record<string, unknown>) => string;

const CIVIL_DATE = /^(\d{4})-(\d{2})-(\d{2})$/;
const METRES_PER_KILOMETRE = 1000;

/**
 * Midnight UTC of a `YYYY-MM-DD` date. A civil date carries no zone, so it is
 * formatted in UTC: formatting it in the device's zone would print the day
 * before for an athlete west of Greenwich.
 *
 * @throws RangeError for a string that is not `YYYY-MM-DD`.
 */
function civilInstant(date: string): Date {
  const match = CIVIL_DATE.exec(date);
  if (match === null) {
    throw new RangeError(`not a civil date (YYYY-MM-DD): ${date}`);
  }
  return new Date(Date.UTC(Number(match[1]), Number(match[2]) - 1, Number(match[3])));
}

/** "mardi 23 septembre" — a plan day, in the words a chat draft names it by. */
export function civilLongDate(date: string, language: string): string {
  return new Intl.DateTimeFormat(language, {
    weekday: 'long',
    day: 'numeric',
    month: 'long',
    timeZone: 'UTC',
  }).format(civilInstant(date));
}

/** The one-letter weekday a week-strip cell is headed by: "M", "T", … in the app language. */
export function civilWeekdayNarrow(date: string, language: string): string {
  return new Intl.DateTimeFormat(language, { weekday: 'narrow', timeZone: 'UTC' }).format(
    civilInstant(date),
  );
}

/** The full weekday of a strip cell, for the spoken label its single letter cannot carry. */
export function civilWeekdayLong(date: string, language: string): string {
  return new Intl.DateTimeFormat(language, { weekday: 'long', timeZone: 'UTC' }).format(
    civilInstant(date),
  );
}

/** The day of the month a strip cell prints, as a figure. */
export function civilDayOfMonth(date: string): number {
  return civilInstant(date).getUTCDate();
}

/**
 * "Saturday 20 September" for an activity's start — in the device's zone,
 * which is where the athlete was when they recorded it.
 */
export function instantLongDate(instant: string, language: string): string {
  return new Intl.DateTimeFormat(language, { weekday: 'long', day: 'numeric', month: 'long' }).format(
    new Date(instant),
  );
}

/** "Sat 20 Sep" — the compact date a row leads with. */
export function instantShortDate(instant: string, language: string): string {
  return new Intl.DateTimeFormat(language, { weekday: 'short', day: 'numeric', month: 'short' }).format(
    new Date(instant),
  );
}

/** When the cache last reached a provider: a date and a 24-hour time, the clock the list rows use. */
export function syncedAtLabel(instant: string, language: string): string {
  return new Intl.DateTimeFormat(language, {
    day: 'numeric',
    month: 'short',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  }).format(new Date(instant));
}

/**
 * The distance a Home row prints: metres as one decimal of a kilometre, in
 * the notation of `language` — `92.0 km`, `92,0 km`.
 */
export function kilometres(metres: number, language: string): string {
  return `${formatDecimal(metres / METRES_PER_KILOMETRE, 1, language)} km`;
}

/** Metres climbed, rounded to the metre, in the notation of `language`. */
export function climbed(metres: number, language: string): string {
  return `+${formatDecimal(Math.round(metres), 0, language)} m`;
}

/**
 * The sport's name in the app language, or the provider's own string when
 * the vocabulary has no label for it — a sport is never shown as a key.
 */
export function sportLabel(t: Translate, sportType: string): string {
  const key = activitySportLabelKey(sportType);
  return key === null ? sportType : t(key);
}

/**
 * The figures a row prints after its name: distance, then duration, then the
 * climb — only the ones the provider reported, in the notation of `language`
 * and the duration in the words of `t` (`45 min 45 s` in French).
 * A missing distance is left out rather than printed as zero.
 */
export function activityFigures(t: Translate, activity: HomeActivity, language: string): string[] {
  const figures: string[] = [];
  if (activity.distance_meters !== null && activity.distance_meters > 0) {
    figures.push(kilometres(activity.distance_meters, language));
  }
  figures.push(formatDuration(t, activity.duration_seconds));
  if (activity.elevation_gain_meters !== null && activity.elevation_gain_meters > 0) {
    figures.push(climbed(activity.elevation_gain_meters, language));
  }
  return figures;
}

/**
 * How a question about the activity names it: its title — its sport when the
 * provider stored none — and its day, the two things the agent finds it by.
 */
export function activityNaming(
  t: Translate,
  activity: HomeActivity,
  language: string,
): { name: string; date: string } {
  return {
    name: activity.name.trim() || sportLabel(t, activity.sport_type),
    date: instantLongDate(activity.start_date, language),
  };
}

/** The composer text a tap on a plan day opens a new chat with: the session, or why it is a rest day. */
export function planDayDraft(t: Translate, day: PlanDay, language: string): string {
  const date = civilLongDate(day.date, language);
  return day.rest
    ? t('home.plan.restDayDraft', { date })
    : t('home.plan.dayDraft', { date, workout: day.workout });
}
