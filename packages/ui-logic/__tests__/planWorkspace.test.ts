// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the athlete workspace's layout and words — seven dates a week with the proposed ones marked, and a plan change « when · who · what »
// ABOUTME: Red if a date the plan holds nothing on goes missing, past and today read alike, or a change names a fortnight of dates in one line

import { describe, expect, it } from 'vitest';
import type { PlanChangeEvent, PlanDay, WorkoutPlan } from '@pierre/shared-types';
import { planCalendarWeeks, planHistoryLine } from '../src/planWorkspace';

function day(date: string, workout: string, position = 0): PlanDay {
  return { date, position, sport: 'ride', workout, intensity: 'Z2', rest: false };
}

const PLAN: WorkoutPlan = {
  goal_race: { name: 'The Traka 200', date: '2027-04-30', discipline: 'gravel', priority: 'A' },
  phases: [],
  weeks: [
    {
      week_start: '2026-11-23',
      focus: 'Prep',
      current: true,
      days: [day('2026-11-23', 'Powerwatts'), day('2026-11-28', 'Hike'), day('2026-11-28', 'Mobility', 1)],
    },
    { week_start: '2026-11-30', focus: 'Base', current: false, days: [day('2026-12-01', 'CrossFit')] },
  ],
  weeks_deferred: 0,
};

describe('planCalendarWeeks', () => {
  const weeks = planCalendarWeeks(PLAN, ['2026-11-28'], '2026-11-24');

  it('lays every week out as its seven dates from Monday, across a month end', () => {
    expect(weeks.map((week) => week.days.map((d) => d.date))).toEqual([
      ['2026-11-23', '2026-11-24', '2026-11-25', '2026-11-26', '2026-11-27', '2026-11-28', '2026-11-29'],
      ['2026-11-30', '2026-12-01', '2026-12-02', '2026-12-03', '2026-12-04', '2026-12-05', '2026-12-06'],
    ]);
  });

  it('keeps each date’s entries in order, and none on a date the plan holds nothing on', () => {
    const [first] = weeks;
    expect(first.days[5].entries.map((entry) => entry.workout)).toEqual(['Hike', 'Mobility']);
    expect(first.days[2].entries).toEqual([]);
  });

  it('marks the proposed dates and where each date sits against today', () => {
    const [first] = weeks;
    expect(first.days.filter((d) => d.proposed).map((d) => d.date)).toEqual(['2026-11-28']);
    expect(first.days.slice(0, 3).map((d) => d.timing)).toEqual(['past', 'today', 'ahead']);
  });

  it('keeps an entry dated outside its week rather than dropping it', () => {
    const stray: WorkoutPlan = { ...PLAN, weeks: [{ ...PLAN.weeks[1], days: [day('2026-12-09', 'Late')] }] };
    const [only] = planCalendarWeeks(stray, [], '2026-11-24');
    expect(only.days).toHaveLength(8);
    expect(only.days[7].entries.map((entry) => entry.workout)).toEqual(['Late']);
  });
});

describe('planHistoryLine', () => {
  // The words a client would show, keyed: enough to see which key each part took.
  const t = (key: string, params?: Record<string, string | number>) =>
    params?.count !== undefined ? `${key}:${params.count}` : key;
  const now = new Date(2026, 10, 25, 15, 0);
  const at = (hours: number, dayOfMonth = 25) => new Date(2026, 10, dayOfMonth, hours, 5).toISOString();

  function event(partial: Partial<PlanChangeEvent>): PlanChangeEvent {
    return { kind: 'day_edit', source: 'coach', dates: [], created_at: at(10), ...partial };
  }

  it('says the time of a change made today, the coach as "you", the dates it touched', () => {
    const line = planHistoryLine(event({ dates: ['2026-11-25', '2026-11-26'] }), { t, language: 'en', agentName: 'Dravr', now });
    expect(line.when).toBe('10:05');
    expect(line.who).toBe('chat.senderYou');
    expect(line.what).toBe('plan.workspace.history.dayEdit · Wed, Nov 25 and Thu, Nov 26');
  });

  it('says the date of an older change, the agent by name, and nothing after a change with no dates', () => {
    const line = planHistoryLine(event({ kind: 'discard', source: 'agent', created_at: at(9, 22) }), {
      t,
      language: 'fr',
      agentName: 'Coach Endurance',
      now,
    });
    expect(line.when).toBe('22 nov.');
    expect(line.who).toBe('Coach Endurance');
    expect(line.what).toBe('plan.workspace.history.discard');
  });

  it('names three dates and counts the rest', () => {
    const dates = ['2026-11-23', '2026-11-24', '2026-11-25', '2026-11-26', '2026-11-27'];
    const line = planHistoryLine(event({ kind: 'agent_draft', source: 'agent', dates }), { t, language: 'en', agentName: 'Dravr', now });
    expect(line.what).toBe(
      'plan.workspace.history.agentDraft · Mon, Nov 23, Tue, Nov 24, Wed, Nov 25, and plan.workspace.history.moreDatesN:2',
    );
  });

  it('words the coach’s call in its own words', () => {
    const line = planHistoryLine(event({ kind: 'coach_call', dates: ['2026-11-29'] }), { t, language: 'en', agentName: 'Dravr', now });
    expect(line.what).toBe('humanCoach.coachCall.badge · Sun, Nov 29');
  });
});
