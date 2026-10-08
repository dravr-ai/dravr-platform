// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for Home's calendar arithmetic — the strip's weeks, a month's grid, how far it pages, one day's entries
// ABOUTME: Red if a month grid breaks the 42-day read, paging stops short of the plan, or a day before history reads as empty

import { describe, it, expect } from 'vitest';
import {
  activityMinutes,
  calendarDay,
  firstOfMonth,
  lastMonday,
  monthGrid,
  shiftMonth,
  weekDays,
} from '../src/home-calendar';
import type { CalendarResponse, HomeActivity } from '../src/home';
import type { WorkoutPlan } from '../src/workout-plan';

function activity(id: string, duration_seconds: number): HomeActivity {
  return {
    id,
    provider: 'strava',
    name: id,
    sport_type: 'run',
    start_date: '2026-09-22T10:00:00Z',
    duration_seconds,
    distance_meters: null,
    elevation_gain_meters: null,
    has_gps: true,
    summary_polyline: null,
    attribution: null,
  };
}

const plan: WorkoutPlan = {
  goal_race: { name: 'Big Red', date: '2026-10-18', discipline: 'gravel', priority: 'A' },
  phases: [],
  weeks: [
    { week_start: '2026-09-21', focus: '', current: true, days: [] },
    { week_start: '2026-09-28', focus: '', current: false, days: [] },
  ],
  weeks_deferred: 3,
};

describe('weekDays and monthGrid', () => {
  it('lays a week out Monday to Sunday', () => {
    expect(weekDays('2026-09-28')).toEqual([
      '2026-09-28',
      '2026-09-29',
      '2026-09-30',
      '2026-10-01',
      '2026-10-02',
      '2026-10-03',
      '2026-10-04',
    ]);
  });

  it('covers a month in whole weeks, never more than six', () => {
    const october = monthGrid('2026-10-01');
    expect(october[0][0]).toBe('2026-09-28');
    expect(october.at(-1)?.at(-1)).toBe('2026-11-01');
    expect(october).toHaveLength(5);
    // August 2026 starts on a Saturday and ends on a Monday: six weeks.
    expect(monthGrid('2026-08-01')).toHaveLength(6);
    // February 2027 starts on a Monday and ends on a Sunday: four weeks.
    expect(monthGrid('2027-02-01')).toHaveLength(4);
  });

  it('moves between months across a year', () => {
    expect(firstOfMonth('2026-09-24')).toBe('2026-09-01');
    expect(shiftMonth('2026-12-01', 1)).toBe('2027-01-01');
    expect(shiftMonth('2026-01-01', -1)).toBe('2025-12-01');
  });
});

describe('lastMonday', () => {
  it('pages to next week without a plan', () => {
    expect(lastMonday(null, '2026-09-24')).toBe('2026-09-28');
  });

  it("pages to the plan's last week, its season end or its deferred weeks, whichever is later", () => {
    expect(lastMonday(plan, '2026-09-24')).toBe('2026-10-19');
    expect(lastMonday({ ...plan, season_end: '2026-11-22' }, '2026-09-24')).toBe('2026-11-16');
    expect(lastMonday({ ...plan, season_end: 'soon' }, '2026-09-24')).toBe('2026-10-19');
  });
});

describe('calendarDay', () => {
  const response: CalendarResponse = {
    today: '2026-09-24',
    from: '2026-09-21',
    to: '2026-09-27',
    history_start: '2026-09-22',
    activities: [
      { date: '2026-09-22', activity: activity('a', 1_800) },
      { date: '2026-09-22', activity: activity('b', 1_230) },
      { date: '2026-09-23', activity: activity('c', 600) },
    ],
    plan_weeks: [
      {
        week_start: '2026-09-21',
        focus: '',
        current: true,
        days: [{ date: '2026-09-24', sport: 'run', workout: 'Tempo', intensity: 'Z3', rest: false }],
      },
    ],
  };

  it("gathers a day's workouts and what the plan holds on it", () => {
    const day = calendarDay(response, '2026-09-22');
    expect(day.activities.map((entry) => entry.id)).toEqual(['a', 'b']);
    expect(activityMinutes(day.activities)).toBe(51);
    expect(day.plan?.kind).toBe('uncovered');
    expect(calendarDay(response, '2026-09-24').plan?.kind).toBe('session');
  });

  it('marks a day before history as unknown, and says no plan with a null', () => {
    expect(calendarDay(response, '2026-09-21').beforeHistory).toBe(true);
    expect(calendarDay(response, '2026-09-22').beforeHistory).toBe(false);
    expect(calendarDay({ ...response, plan_weeks: null }, '2026-09-24').plan).toBeNull();
  });
});
