// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the athlete workspace's reading of a subject plan — the proposed dates, a date's entries, the pre-publish check
// ABOUTME: Red if a coach's mark reads as a proposal, a moved or added session does not, or the check asks about a day already kept or past

import { describe, expect, it } from 'vitest';
import type { PlanDay, PlanWeek, WorkoutPlan } from '@pierre/shared-types';
import {
  entriesByDate,
  evidenceCallReason,
  planProposedDates,
  prePublishChecks,
} from '../src/plan-proposals';

function day(date: string, workout: string, extra: Partial<PlanDay> = {}): PlanDay {
  return { date, sport: 'ride', workout, intensity: 'Z2', rest: false, duration_min: 60, ...extra };
}

function week(weekStart: string, days: PlanDay[]): PlanWeek {
  return { week_start: weekStart, focus: '', current: false, days };
}

function plan(weeks: PlanWeek[]): WorkoutPlan {
  return {
    goal_race: { name: 'The Traka 200', date: '2027-04-30', discipline: 'gravel', priority: 'A' },
    phases: [],
    weeks,
    weeks_deferred: 0,
  };
}

const LIVE_WEEK = [
  day('2026-11-23', 'Powerwatts'),
  day('2026-11-24', 'CrossFit', { sport: 'strength' }),
  day('2026-11-25', 'Track'),
  day('2026-11-26', 'CrossFit', { sport: 'strength' }),
];
const LIVE = plan([week('2026-11-23', LIVE_WEEK)]);

describe('planProposedDates', () => {
  it('names nothing when the draft is the live plan', () => {
    expect(planProposedDates(plan([week('2026-11-23', LIVE_WEEK)]), LIVE)).toEqual([]);
  });

  it('names the two dates of a swap, ascending, and nothing else', () => {
    const swapped = [
      LIVE_WEEK[0],
      LIVE_WEEK[1],
      day('2026-11-25', 'CrossFit', { sport: 'strength' }),
      day('2026-11-26', 'Track'),
    ];
    expect(planProposedDates(plan([week('2026-11-23', swapped)]), LIVE)).toEqual(['2026-11-25', '2026-11-26']);
  });

  it('reads a session added to a date, and one taken off it, as a change of that date', () => {
    const added = [...LIVE_WEEK, day('2026-11-24', 'Hip mobility', { position: 1, sport: 'mobility' })];
    expect(planProposedDates(plan([week('2026-11-23', added)]), LIVE)).toEqual(['2026-11-24']);
    const removed = LIVE_WEEK.filter((entry) => entry.date !== '2026-11-25');
    expect(planProposedDates(plan([week('2026-11-23', removed)]), LIVE)).toEqual(['2026-11-25']);
  });

  it('reads two sessions of a date swapped in order as a change', () => {
    const two = [day('2026-11-28', 'Hike', { position: 0 }), day('2026-11-28', 'Mobility', { position: 1 })];
    const live = plan([week('2026-11-23', two)]);
    const reordered = [{ ...two[1], position: 0 }, { ...two[0], position: 1 }];
    expect(planProposedDates(plan([week('2026-11-23', reordered)]), live)).toEqual(['2026-11-28']);
  });

  it('ignores the coach’s call and the editor’s marks, which change no session', () => {
    const marked = LIVE_WEEK.map((entry, index) =>
      index === 0 ? { ...entry, coach_call: true } : index === 1 ? { ...entry, edited: { workout: 'Old', rest: false } } : entry,
    );
    expect(planProposedDates(plan([week('2026-11-23', marked)]), LIVE)).toEqual([]);
  });

  it('reads a changed duration or step as a change, whatever the key order', () => {
    const longer = [{ ...LIVE_WEEK[0], duration_min: 75 }, ...LIVE_WEEK.slice(1)];
    expect(planProposedDates(plan([week('2026-11-23', longer)]), LIVE)).toEqual(['2026-11-23']);
    const reKeyed = LIVE_WEEK.map((entry) => Object.fromEntries(Object.entries(entry).reverse()) as unknown as PlanDay);
    expect(planProposedDates(plan([week('2026-11-23', reKeyed)]), LIVE)).toEqual([]);
  });

  it('compares a week with the live week of the same Monday, and every date of a week live never held', () => {
    const next = [day('2026-11-30', 'Powerwatts'), day('2026-12-01', 'CrossFit')];
    const draft = plan([week('2026-11-23', LIVE_WEEK), week('2026-11-30', next)]);
    expect(planProposedDates(draft, LIVE)).toEqual(['2026-11-30', '2026-12-01']);
  });

  it('proposes every date the draft holds when the athlete has no live plan', () => {
    expect(planProposedDates(LIVE, null)).toEqual(['2026-11-23', '2026-11-24', '2026-11-25', '2026-11-26']);
  });
});

describe('entriesByDate', () => {
  it('groups a week by date in the order the dates come, each date in its own order', () => {
    const days = [day('2026-11-28', 'Hike'), day('2026-11-28', 'Mobility'), day('2026-11-29', 'Long ride')];
    expect(entriesByDate(days).map((entries) => entries.map((entry) => entry.workout))).toEqual([
      ['Hike', 'Mobility'],
      ['Long ride'],
    ]);
  });
});

describe('evidenceCallReason', () => {
  it('says outside before convention, and nothing for a rest day or a plain session', () => {
    const convention = { kind: 'convention' as const, label: 'Coaching convention' };
    expect(evidenceCallReason(day('2026-11-29', 'Long ride', { evidence: convention }))).toBe('convention');
    expect(evidenceCallReason(day('2026-11-29', 'VO2', { outside: true, evidence: convention }))).toBe('outside');
    expect(evidenceCallReason(day('2026-11-29', 'VO2', { outside_evidence: 'Outside the evidence' }))).toBe('outside');
    expect(evidenceCallReason(day('2026-11-29', 'Rest', { rest: true, outside: true }))).toBeNull();
    expect(evidenceCallReason(day('2026-11-29', 'Steady', { evidence: { kind: 'transposed', label: 'Transposed' } }))).toBeNull();
  });
});

describe('prePublishChecks', () => {
  const convention = { kind: 'convention' as const, label: 'Coaching convention' };
  const draft = plan([
    week('2026-11-23', [
      day('2026-11-23', 'Past long ride', { evidence: convention }),
      day('2026-11-28', 'Hike'),
      day('2026-11-28', 'Mobility', { evidence: convention }),
      day('2026-11-29', 'Long ride', { evidence: convention, coach_call: true }),
    ]),
    week('2026-11-30', [day('2026-12-01', 'Over-unders', { outside: true })]),
  ]);

  it('lists the convention and outside sessions from today on that the coach has not kept, with their place', () => {
    const items = prePublishChecks(draft, '2026-11-24');
    expect(items.map((item) => [item.day.workout, item.position, item.reason, item.entries.length])).toEqual([
      ['Mobility', 1, 'convention', 2],
      ['Over-unders', 0, 'outside', 1],
    ]);
  });

  it('asks about today itself', () => {
    expect(prePublishChecks(draft, '2026-11-23').map((item) => item.day.workout)).toContain('Past long ride');
  });
});
