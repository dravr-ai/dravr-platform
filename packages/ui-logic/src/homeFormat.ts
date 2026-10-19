// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The words and figures Home prints on both clients — calendar days, instants, sports, row figures, plan drafts
// ABOUTME: A plan date is a calendar day never shifted by the device timezone; an activity start is an instant in the device's zone

import type { CalendarDay, HomeActivity, PlanDay, PlanDayLookup, PlanPhase } from '@pierre/shared-types';
import { addCivilDays, instantMs, mondayOf, phaseWeekOn, planDayDistanceMeters } from '@pierre/shared-types';
import { activitySportLabelKey, sportHasRoutes, weekMinutesSeconds } from '@pierre/shared-constants';
import { formatDuration, type VolumeMetric } from '@pierre/domain-utils';
import { formatElevation, formatSpokenDistance, type DistanceUnit } from '@pierre/chat-utils';
import { formatActivityDistance } from './activityView';

/** The translator the helpers take: each client's own `t`, since module scope holds no hook. */
export type Translate = (key: string, options?: Record<string, unknown>) => string;

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

/** "20 Sep" — a day of the form trend, at the ends of its axis and in its spoken summary. */
export function civilShortDate(date: string, language: string): string {
  return formatCivilDate(date, language, { day: 'numeric', month: 'short' });
}

/** The one-letter weekday a week-strip cell is headed by: "M", "T", … in the app language. */
export function civilWeekdayNarrow(date: string, language: string): string {
  return formatCivilDate(date, language, { weekday: 'narrow' });
}

/** The day of the month a strip cell prints, as a figure; NaN for a string that is not a calendar day. */
export function civilDayOfMonth(date: string): number {
  const match = CIVIL_DATE.exec(date);
  return match === null ? Number.NaN : Number(match[3]);
}

/** "September 2026" — a month sheet's title, from any day of the month. */
export function civilMonthTitle(date: string, language: string): string {
  return formatCivilDate(date, language, { month: 'long', year: 'numeric' });
}

/** "September 14" — the Monday a paged week is named by. */
export function civilDayMonth(date: string, language: string): string {
  return formatCivilDate(date, language, { day: 'numeric', month: 'long' });
}

/**
 * An RFC 3339 instant in the app's language and the device's own timezone,
 * read by `instantMs` as the Home parser that admitted it reads it, so a
 * stamp to the microsecond formats on Hermes as it does in the browser.
 */
export function formatInstant(instant: string, language: string, options: Intl.DateTimeFormatOptions): string {
  const ms = instantMs(instant);
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
export function sportLabel(t: Translate, sport: string): string {
  const key = activitySportLabelKey(sport);
  return key === null ? sport : t(key);
}

/**
 * The figures a Home row prints after the sport: distance, time, climbing —
 * each only when the activity carries it, in the notation of `language`
 * (`42,00 km` in French), the athlete's `unit` system (`26.10 mi`, `+279 ft`)
 * and the time in the words of `t` (`45 min 45 s`). The distance reads as the
 * activity's own view reads it (`formatActivityDistance`), so a row and the
 * view it opens print the same figure.
 */
export function activityRowFigures(
  t: Translate,
  activity: HomeActivity,
  language: string,
  unit: DistanceUnit,
): string[] {
  const figures: string[] = [];
  if (activity.distance_meters !== null && activity.distance_meters > 0) {
    figures.push(formatActivityDistance(activity.distance_meters, unit, language));
  }
  figures.push(formatDuration(t, activity.duration_seconds));
  if (activity.elevation_gain_meters !== null && activity.elevation_gain_meters > 0) {
    figures.push(`+${formatElevation(activity.elevation_gain_meters, unit, language)}`);
  }
  return figures;
}

/**
 * A week's figure for the metric the volume bars stand for: a distance as a
 * Home row prints it, or the week's time as a duration in whole minutes.
 */
export function volumeMetricText(
  t: Translate,
  metric: VolumeMetric,
  value: number,
  language: string,
  unit: DistanceUnit,
): string {
  return metric === 'distance'
    ? formatActivityDistance(value, unit, language)
    : formatDuration(t, weekMinutesSeconds(value));
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
    date: formatInstant(activity.start_date, language, DRAFT_DATE),
  };
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
export function phaseWeekLabel(t: Translate, phases: readonly PlanPhase[], date: string): string | null {
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
export function planDayDraft(t: Translate, language: string, date: string, lookup: PlanDayLookup): string | null {
  const named = formatCivilDate(date, language, DRAFT_DATE);
  if (lookup.kind === 'session') {
    return t('home.plan.dayDraft', { date: named, workout: lookup.day.workout });
  }
  if (lookup.kind === 'rest') {
    return t('home.plan.restDayDraft', { date: named });
  }
  return null;
}

/**
 * The chat draft asking for a route for a planned session, or null when the
 * day has none to look for: a rest day, a day the plan does not cover, or a
 * session in a sport with no route (a swim, strength work, the trainer). The
 * draft names the day and the workout — and the session's distance when the
 * plan gives one, so the agent can rank routes against it — and leaves the
 * agent to ask where the athlete is before it searches. The distance reads
 * in the athlete's `unit` system.
 */
export function planDayRouteDraft(
  t: Translate,
  language: string,
  date: string,
  lookup: PlanDayLookup,
  unit: DistanceUnit,
): string | null {
  if (lookup.kind !== 'session' || typeof lookup.day.sport !== 'string' || !sportHasRoutes(lookup.day.sport)) {
    return null;
  }
  const named = formatCivilDate(date, language, DRAFT_DATE);
  const meters = planDayDistanceMeters(lookup.day);
  return meters === null
    ? t('home.plan.routeDraft', { date: named, workout: lookup.day.workout })
    : t('home.plan.routeDistanceDraft', {
        distance: formatSpokenDistance(meters, unit, language),
        date: named,
        workout: lookup.day.workout,
      });
}

/** A session's sport, minutes and intensity, in that order, each only when the day carries it. */
export function sessionFacts(t: Translate, day: PlanDay): string[] {
  // Typed as strings, but the card is checked only for its outline on the
  // way in, so each field is read the way WorkoutPlanCard reads it.
  const facts: unknown[] = [
    typeof day.sport === 'string' ? sportLabel(t, day.sport) : null,
    day.duration_min !== undefined ? `${day.duration_min} min` : null,
    day.intensity,
  ];
  return facts.filter((fact): fact is string => typeof fact === 'string' && fact.length > 0);
}

/** What the plan holds on the day, in words; a past day the plan never reached says nothing of it. */
function planWords(t: Translate, lookup: PlanDayLookup | null, past: boolean): string | null {
  if (lookup === null || (past && lookup.kind === 'uncovered')) return null;
  if (lookup.kind === 'rest') return t('chat.restDay');
  if (lookup.kind === 'uncovered') return t('home.plan.notCovered');
  return [lookup.day.workout, ...sessionFacts(t, lookup.day)].join(' · ');
}

/**
 * What a screen reader hears for a calendar cell, in full — the cell itself
 * only has room for a hint: the day as `named`, the workouts done, then the
 * plan's session for the day with its sport, minutes and intensity.
 */
export function calendarDayLabel(t: Translate, named: string, day: CalendarDay | null, today: string): string {
  if (day === null) return named;
  if (day.beforeHistory) return `${named} — ${t('home.calendar.notKept')}`;
  const parts = [
    day.activities.length > 0 ? `${t('home.calendar.done')}: ${day.activities.map((a) => a.name).join(', ')}` : null,
    planWords(t, day.plan, day.date < today),
  ].filter((part): part is string => part !== null);
  return parts.length === 0 ? named : `${named} — ${parts.join(' — ')}`;
}
