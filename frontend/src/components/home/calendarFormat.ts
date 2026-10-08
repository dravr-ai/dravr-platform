// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Words for Home's calendar days — what a screen reader hears for a strip cell or a month cell
// ABOUTME: The workouts done, then the plan's session; a day older than stored history says it is not kept

import type { TFunction } from '@pierre/i18n';
import type { CalendarDay, PlanDayLookup } from '@pierre/shared-types';
import { sessionFacts } from './homeFormat';

/** What the plan holds on the day, in words; a past day the plan never reached says nothing of it. */
function planWords(t: TFunction, lookup: PlanDayLookup | null, past: boolean): string | null {
  if (lookup === null || (past && lookup.kind === 'uncovered')) return null;
  if (lookup.kind === 'rest') return t('chat.restDay');
  if (lookup.kind === 'uncovered') return t('home.plan.notCovered');
  return [lookup.day.workout, ...sessionFacts(t, lookup.day)].join(' · ');
}

/**
 * What a screen reader hears for a cell, in full — the cell itself only has
 * room for a hint: the workouts done, then the plan's session for the day.
 */
export function calendarDayLabel(t: TFunction, named: string, day: CalendarDay | null, today: string): string {
  if (day === null) return named;
  if (day.beforeHistory) return `${named} — ${t('home.calendar.notKept')}`;
  const parts = [
    day.activities.length > 0 ? `${t('home.calendar.done')}: ${day.activities.map((a) => a.name).join(', ')}` : null,
    planWords(t, day.plan, day.date < today),
  ].filter((part): part is string => part !== null);
  return parts.length === 0 ? named : `${named} — ${parts.join(' — ')}`;
}
