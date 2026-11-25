// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the athlete workspace's reading of a subject plan — a date's entries, why a session is the coach's call, the pre-publish check
// ABOUTME: Red if a date's entries lose their order, the reasons swap, or the check asks about a day already kept or past

import { describe, expect, it } from 'vitest';
import type { PlanDay, PlanWeek, WorkoutPlan } from '@pierre/shared-types';
import { entriesByDate, evidenceCallReason, prePublishChecks } from '../src/plan-proposals';

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
