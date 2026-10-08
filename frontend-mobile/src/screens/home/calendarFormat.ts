// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Words and dates for Home's calendar — a strip or month cell's spoken label, a month's title, a week's start
// ABOUTME: Civil dates are formatted in UTC, so a day never slides to its neighbour on a device west of Greenwich

import type { CalendarDay, PlanDayLookup } from '@pierre/shared-types';
import { civilDayOfMonth, civilWeekdayLong, type Translate } from './homeFormat';

/** What the plan holds on the day, in words; a past day the plan never reached says nothing of it. */
function planWords(t: Translate, lookup: PlanDayLookup | null, past: boolean): string | null {
  if (lookup === null || (past && lookup.kind === 'uncovered')) return null;
  switch (lookup.kind) {
    case 'session':
      return lookup.day.workout;
    case 'rest':
      return t('chat.restDay');
    case 'uncovered':
      return t('home.plan.notCovered');
  }
}

/**
 * What a cell is, in words, for a screen reader that cannot see its marks:
 * "Tuesday 22", then the workouts done, then what the plan holds.
 */
export function calendarCellLabel(
  t: Translate,
  language: string,
  date: string,
  day: CalendarDay | null,
  today: string,
): string {
  const when = `${civilWeekdayLong(date, language)} ${civilDayOfMonth(date)}`;
  if (day === null) return when;
  if (day.beforeHistory) return `${when}, ${t('home.calendar.notKept')}`;
  const parts = [
    day.activities.length > 0 ? `${t('home.calendar.done')}: ${day.activities.map((a) => a.name).join(', ')}` : null,
    planWords(t, day.plan, date < today),
  ].filter((part): part is string => part !== null);
  return [when, ...parts].join(', ');
}

/** "September 2026" — the month sheet's title. */
export function civilMonthTitle(first: string, language: string): string {
  return new Intl.DateTimeFormat(language, { month: 'long', year: 'numeric', timeZone: 'UTC' }).format(
    new Date(`${first}T00:00:00Z`),
  );
}

/** "September 14" — the Monday a paged week is named by. */
export function civilDayMonth(date: string, language: string): string {
  return new Intl.DateTimeFormat(language, { day: 'numeric', month: 'long', timeZone: 'UTC' }).format(
    new Date(`${date}T00:00:00Z`),
  );
}
