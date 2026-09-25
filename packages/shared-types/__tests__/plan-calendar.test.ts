// ABOUTME: Unit tests for the civil-date reads over a plan card — tomorrow, the week's Monday, the day a date falls on
// ABOUTME: Pins the rest-day versus uncovered-date distinction and calendar arithmetic across month, year and DST edges

import { describe, it, expect } from 'vitest';
import { addCivilDays, mondayOf, phaseWeekOn, planDayOn } from '../src/plan-calendar';
import type { PlanPhase, WorkoutPlan } from '../src/workout-plan';

const plan: WorkoutPlan = {
  goal_race: { name: 'Big Red', date: '2026-10-18', discipline: 'gravel', priority: 'A' },
  phases: [],
  weeks: [
    {
      week_start: '2026-09-21',
      focus: 'Threshold volume',
      current: true,
      days: [
        { date: '2026-09-24', sport: 'run', workout: 'Tempo run', duration_min: 50, intensity: 'Z3', rest: false },
        { date: '2026-09-25', sport: 'rest', workout: 'Rest', intensity: 'rest', rest: true },
      ],
    },
    {
      week_start: '2026-09-28',
      focus: 'Absorb',
      current: false,
      days: [{ date: '2026-09-29', sport: 'ride', workout: 'Easy spin', duration_min: 60, intensity: 'Z2', rest: false }],
    },
  ],
  weeks_deferred: 2,
};

describe('addCivilDays', () => {
  it('steps across month and year ends', () => {
    expect(addCivilDays('2026-09-30', 1)).toBe('2026-10-01');
    expect(addCivilDays('2026-12-31', 1)).toBe('2027-01-01');
    expect(addCivilDays('2026-03-01', -1)).toBe('2026-02-28');
    expect(addCivilDays('2028-02-28', 1)).toBe('2028-02-29');
  });

  it('counts whole days across a daylight-saving change', () => {
    // Europe and North America both change clocks in these windows; a
    // local-time Date would lose or gain an hour and can land on the wrong day.
    expect(addCivilDays('2026-03-07', 2)).toBe('2026-03-09');
    expect(addCivilDays('2026-10-24', 2)).toBe('2026-10-26');
  });

  it('refuses a string that is not a calendar date, and a fractional step', () => {
    expect(() => addCivilDays('2026-02-30', 1)).toThrow(RangeError);
    expect(() => addCivilDays('24/09/2026', 1)).toThrow(RangeError);
    expect(() => addCivilDays('2026-09-24', 0.5)).toThrow(RangeError);
  });
});

describe('mondayOf', () => {
  it('returns the Monday of the week, the date itself on a Monday', () => {
    expect(mondayOf('2026-09-24')).toBe('2026-09-21');
    expect(mondayOf('2026-09-21')).toBe('2026-09-21');
    expect(mondayOf('2026-09-27')).toBe('2026-09-21');
    expect(mondayOf('2027-01-01')).toBe('2026-12-28');
  });
});

describe('planDayOn', () => {
  it('finds a session with the week it belongs to', () => {
    const today = planDayOn(plan, '2026-09-24');
    expect(today.kind).toBe('session');
    if (today.kind === 'session') {
      expect(today.day.workout).toBe('Tempo run');
      expect(today.week.week_start).toBe('2026-09-21');
    }
  });

  it('reads a planned rest day as rest', () => {
    const tomorrow = planDayOn(plan, addCivilDays('2026-09-24', 1));
    expect(tomorrow.kind).toBe('rest');
  });

  it('reads a date the plan says nothing about as uncovered, never as rest', () => {
    expect(planDayOn(plan, '2026-09-26')).toEqual({ kind: 'uncovered' });
    expect(planDayOn(plan, '2026-10-12')).toEqual({ kind: 'uncovered' });
  });

  it('finds a day in the next week', () => {
    const lookup = planDayOn(plan, '2026-09-29');
    expect(lookup.kind).toBe('session');
    if (lookup.kind === 'session') {
      expect(lookup.week.focus).toBe('Absorb');
    }
  });
});

describe('phaseWeekOn', () => {
  const build: PlanPhase = {
    kind: 'build',
    start: '2026-09-07',
    end: '2026-10-05',
    weeks: 4,
    purpose: 'Raise threshold',
    intent: 'Two quality days a week',
    current: true,
  };

  it('counts weeks from the phase start, starting at 1', () => {
    expect(phaseWeekOn(build, '2026-09-07')).toBe(1);
    expect(phaseWeekOn(build, '2026-09-13')).toBe(1);
    expect(phaseWeekOn(build, '2026-09-14')).toBe(2);
    expect(phaseWeekOn(build, '2026-09-24')).toBe(3);
    expect(phaseWeekOn(build, '2026-10-04')).toBe(4);
  });

  it('is null outside the phase, whose end is the day after its last', () => {
    expect(phaseWeekOn(build, '2026-09-06')).toBeNull();
    expect(phaseWeekOn(build, '2026-10-05')).toBeNull();
  });

  it('keeps counting through an open-ended phase', () => {
    const open: PlanPhase = { ...build, end: undefined };
    expect(phaseWeekOn(open, '2026-11-02')).toBe(9);
  });
});
