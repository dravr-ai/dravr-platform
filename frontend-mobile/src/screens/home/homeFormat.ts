// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The words and figures the Home tab prints — civil and instant dates, durations, distances, chat drafts, activity names
// ABOUTME: Pure functions of the app language, so the screen and its tests format a date, a sport and a draft one way

import { planDayDistanceMeters, type HomeActivity, type PlanDay } from '@pierre/shared-types';
import { activitySportLabelKey, sportHasRoutes } from '@pierre/shared-constants';
import { formatDuration } from '@pierre/domain-utils';
import { formatDistance, formatElevation, formatSpokenDistance, type DistanceUnit } from '@pierre/chat-utils';

/** The translator the helpers take; module scope holds no hook. */
export type Translate = (key: string, options?: Record<string, unknown>) => string;

const CIVIL_DATE = /^(\d{4})-(\d{2})-(\d{2})$/;

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

/** "20 Sep" — a day of the form trend, at the ends of its axis and in its spoken summary. */
export function civilShortDate(date: string, language: string): string {
  return new Intl.DateTimeFormat(language, { day: 'numeric', month: 'short', timeZone: 'UTC' }).format(
    civilInstant(date),
  );
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
 * The distance a Home row prints: one decimal of a kilometre or a mile, in
 * the notation of `language` — `92.0 km`, `92,0 km`, `57.2 mi`.
 */
export function rowDistance(metres: number, unit: DistanceUnit, language: string): string {
  return formatDistance(metres, unit, 1, language);
}

/** Metres climbed, rounded to the metre or the foot, in the notation of `language`. */
export function climbed(metres: number, unit: DistanceUnit, language: string): string {
  return `+${formatElevation(metres, unit, language)}`;
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
 * and the athlete's `unit` system, the duration in the words of `t`
 * (`45 min 45 s` in French). A missing distance is left out rather than
 * printed as zero.
 */
export function activityFigures(
  t: Translate,
  activity: HomeActivity,
  language: string,
  unit: DistanceUnit,
): string[] {
  const figures: string[] = [];
  if (activity.distance_meters !== null && activity.distance_meters > 0) {
    figures.push(rowDistance(activity.distance_meters, unit, language));
  }
  figures.push(formatDuration(t, activity.duration_seconds));
  if (activity.elevation_gain_meters !== null && activity.elevation_gain_meters > 0) {
    figures.push(climbed(activity.elevation_gain_meters, unit, language));
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

/**
 * The draft asking for a route for a planned session, or null when the day
 * has none to look for: a rest day, or a session in a sport with no route (a
 * swim, strength work, the trainer). It names the day and the workout — and
 * the session's distance when the plan gives one, so the agent can rank
 * routes against it, in the athlete's `unit` system — and leaves the agent to
 * ask where the athlete is.
 */
export function planDayRouteDraft(t: Translate, day: PlanDay, language: string, unit: DistanceUnit): string | null {
  if (day.rest || typeof day.sport !== 'string' || !sportHasRoutes(day.sport)) {
    return null;
  }
  const date = civilLongDate(day.date, language);
  const metres = planDayDistanceMeters(day);
  return metres === null
    ? t('home.plan.routeDraft', { date, workout: day.workout })
    : t('home.plan.routeDistanceDraft', { distance: formatSpokenDistance(metres, unit, language), date, workout: day.workout });
}

