// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Wire-shaped calendar answers for the Home week and month tests, built from the plan and activities the Home fixtures hold
// ABOUTME: Answers whatever span is asked, the way the server does: the workouts on those days and the plan weeks over them

import type { CalendarResponse, HomeActivity, WorkoutPlan } from '@pierre/shared-types';
import { addCivilDays } from '@pierre/shared-types';
import { activity, homePlan, TODAY } from './homeFixtures';

/** The first day the cache holds in full in these tests. */
export const HISTORY_START = '2026-09-09';

/** Workouts on the strip's past days: two on Tuesday the 22nd, one the Saturday before. */
export function calendarActivities(): Array<{ date: string; activity: HomeActivity }> {
  return [
    {
      date: '2026-09-19',
      activity: activity({ id: 'cal-1', name: 'Saturday long run', start_date: '2026-09-19T12:00:00Z', duration_seconds: 5400 }),
    },
    {
      date: '2026-09-22',
      activity: activity({ id: 'cal-2', name: 'Strides session', start_date: '2026-09-22T11:00:00Z', duration_seconds: 2100 }),
    },
    {
      date: '2026-09-22',
      activity: activity({
        id: 'cal-3',
        provider: 'garmin',
        name: 'Evening spin',
        sport_type: 'ride',
        start_date: '2026-09-22T22:00:00Z',
        duration_seconds: 1800,
        distance_meters: 15000,
        elevation_gain_meters: null,
      }),
    },
  ];
}

/**
 * The calendar answer for `from..=to`, as the server would give it with
 * `plan` active (null for no plan) and `activities` in the cache.
 */
export function calendarAnswer(
  from: string,
  to: string,
  {
    plan = homePlan(),
    activities = calendarActivities(),
    historyStart = HISTORY_START,
  }: { plan?: WorkoutPlan | null; activities?: Array<{ date: string; activity: HomeActivity }>; historyStart?: string } = {},
): CalendarResponse {
  return {
    today: TODAY,
    from,
    to,
    history_start: historyStart,
    activities: activities.filter((entry) => entry.date >= from && entry.date <= to && entry.date >= historyStart),
    plan_weeks:
      plan === null
        ? null
        : plan.weeks.filter((week) => week.week_start <= to && addCivilDays(week.week_start, 6) >= from),
  };
}
