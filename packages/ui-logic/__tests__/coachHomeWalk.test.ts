// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Coach Home rules a walk of the phone pinned — a reason restated, mornings as one line, a plan's last day, a cut reading's day, the filter's order
// ABOUTME: Run on a key-echoing translator so a catalogue edit cannot hide a rule that changed

import { describe, it, expect } from 'vitest';
import type { CoachAttentionEvidence, CoachAttentionReason, CoachRosterAthlete } from '@pierre/shared-types';
import {
  FORM_MORNING_EVIDENCE_KEY,
  buildQueue,
  cutReadingDay,
  describeEvidence,
  endedPlan,
  planLastDay,
  restatesReason,
  threadState,
} from '../src/coachHome';
import { formWords } from '../src/coachHomeWords';
import { ALL_COACHED_ATHLETES, coachedAthleteOptions } from '../src/coachReplies';

const echo = (key: string, options?: Record<string, unknown>) =>
  options === undefined ? key : `${key}${JSON.stringify(options)}`;

const STRAVA = { kind: 'provider' as const, providers: ['strava'] };

function line(key: string, params: Record<string, string | number>, at = '2026-11-02T10:19:00Z'): CoachAttentionEvidence {
  return { key, params, source: STRAVA, at };
}

function reason(over: Partial<CoachAttentionReason> & { id: string }): CoachAttentionReason {
  return {
    kind: 'nothing_received',
    horizon: 'today',
    rank: 30,
    key: 'coachHome.reason.nothingReceivedReconnect',
    params: { days: 8, provider: 'strava' },
    evidence: [],
    action: 'write_to_athlete',
    target: {},
    fingerprint: `fp-${over.id}`,
    dismissed_at: null,
    ...over,
  };
}

function athlete(over: Partial<CoachRosterAthlete> = {}): CoachRosterAthlete {
  return { user_id: 'jacob', name: 'Jacob Vachon', groups: [{ id: 'g1', name: 'Tri' }], status: 'shared', attention: [], ...over };
}

describe('a reason the lead already says', () => {
  const refused = line('coachHome.evidence.sourceRefusedSince', { provider: 'strava', date: '2026-11-02' });
  const received = line('coachHome.evidence.lastReceived', { provider: 'strava', date: '2026-10-25' });
  const lead = reason({ id: 'nothing_received', evidence: [received, refused] });
  const reconnect = reason({ id: 'provider_reconnect:strava', kind: 'provider_reconnect', rank: 31, evidence: [refused, received] });

  it('is a restatement when every line of its evidence is one of the lead’s, in any order', () => {
    expect(restatesReason(lead, reconnect)).toBe(true);
    expect(restatesReason(lead, reason({ id: 'other', evidence: [line('coachHome.evidence.dataStale', {})] }))).toBe(false);
    expect(restatesReason(lead, reason({ id: 'bare', evidence: [] }))).toBe(false);
  });

  it('is dropped from the queue entry’s other reasons', () => {
    const queue = buildQueue([athlete({ attention: [lead, reconnect] })]);
    expect(queue.today[0].reason.id).toBe('nothing_received');
    expect(queue.today[0].others).toEqual([]);
  });
});

describe('describeEvidence', () => {
  it('reads a streak of mornings as one line, today first, the earlier ones in brackets, where the first stood', () => {
    const mornings = ['2026-10-30', '2026-11-02', '2026-10-31', '2026-11-01'].map((date, i) =>
      line(FORM_MORNING_EVIDENCE_KEY, { date, form_pct: -30 - i }),
    );
    const lines = describeEvidence(echo, 'en', [line('coachHome.evidence.planWeek', { week: 3 }), ...mornings]);
    expect(lines).toHaveLength(2);
    expect(lines[0].line.key).toBe('coachHome.evidence.planWeek');
    expect(lines[1].line.params.date).toBe('2026-11-02');
    expect(lines[1].text).toMatch(/^coachHome\.evidence\.formMornings/);
    expect(lines[1].text).toContain('coachHome.evidence.formMorningEarlier');
  });

  it('leaves a single morning as the line it came as', () => {
    const lines = describeEvidence(echo, 'en', [line(FORM_MORNING_EVIDENCE_KEY, { date: '2026-11-02', form_pct: -34 })]);
    expect(lines).toHaveLength(1);
    expect(lines[0].text).toMatch(/^coachHome\.evidence\.formMorning\{/);
  });
});

describe('a plan’s last day', () => {
  it('is the day before the exclusive `ends`', () => {
    expect(planLastDay('2026-10-19')).toBe('2026-10-18');
    expect(planLastDay('2026-03-01')).toBe('2026-02-28');
    expect(planLastDay(null)).toBeNull();
  });

  it('dates an ended thread by it, as the queue’s « Plan ended on » does', () => {
    const ended = athlete({ season: { state: 'plan_ended', race: null, ends: '2026-10-19' } });
    expect(threadState(ended)).toEqual({ kind: 'ended', date: '2026-10-18' });
  });
});

describe('a cut reading', () => {
  const training = {
    form: { band: 'fresh' as const, as_of: '2026-11-02' },
    ctl: 60,
    days_since_last_activity: 8,
    last_session: null,
    last_activity_per_source: { strava: '2026-10-25' },
    needs_reconnect: ['strava'],
    stale: false,
  };

  it('is as new as the last day a source delivered, never the day it was recomputed', () => {
    expect(cutReadingDay(training)).toBe('2026-10-25');
    expect(cutReadingDay({ ...training, form: { band: 'fresh', as_of: '2026-10-20' } })).toBe('2026-10-20');
    expect(cutReadingDay(undefined)).toBeNull();
  });

  it('says that day in the roster row', () => {
    const words = formWords(echo, 'en', athlete({ training }));
    expect(words.cut).toBe(true);
    expect(words.asOf).toContain('Oct 25');
    expect(words.asOf).not.toContain('Nov 2');
  });
});

describe('the reply review’s athlete filter', () => {
  it('leads with every athlete, then names in the reader’s collation, a non-sharer named as one', () => {
    const options = coachedAthleteOptions(echo, 'fr', [
      { user_id: 'u-florence', name: 'Florence Martel', shares_with_coach: true },
      { user_id: 'u-felix', name: 'Félix Morin', shares_with_coach: true },
      { user_id: 'u-clara', name: 'Clara Girouard', shares_with_coach: false },
    ]);
    expect(options.map((o) => o.key)).toEqual([ALL_COACHED_ATHLETES, 'u-clara', 'u-felix', 'u-florence']);
    expect(options[1].label).toBe('coachReview.notSharing{"name":"Clara Girouard"}');
  });
});

describe('a plan that ran out', () => {
  const plan = (ends: string | null, lastEnd: string | null): NonNullable<CoachRosterAthlete['plan']> => ({
    phase: null,
    week: null,
    weeks_total: 12,
    goal_race: { name: 'Fall 10K', date: '2026-10-11', a_race: false } as unknown as NonNullable<CoachRosterAthlete['plan']>['goal_race'],
    races: [],
    season_end: null,
    ends,
    phases: [
      { kind: 'base', start: '2026-08-03', end: '2026-09-14', weeks: 6 },
      { kind: 'build', start: '2026-09-14', end: lastEnd, weeks: 5 },
    ],
    pushed_to: [],
  });

  it('is in transition once the server says so, dated by its last day', () => {
    const season = { state: 'plan_ended' as const, race: null, ends: '2026-10-19' };
    expect(endedPlan(plan('2026-10-19', '2026-10-19'), season, '2026-10-05')).toEqual({ state: 'plan_ended', date: '2026-10-18' });
  });

  it('is in transition from the day after its last phase, even before the season state catches up', () => {
    const season = { state: 'no_a_race' as const, race: null, ends: '2026-10-19' };
    expect(endedPlan(plan('2026-10-19', '2026-10-19'), season, '2026-10-18')).toBeNull();
    expect(endedPlan(plan('2026-10-19', '2026-10-19'), season, '2026-10-19')).toEqual({ state: 'plan_ended', date: '2026-10-18' });
  });

  it('names the race a plan closed on', () => {
    const season = { state: 'nothing_since_last_race' as const, race: { name: 'Marathon', date: '2026-10-11', a_race: true }, ends: '2026-10-12' };
    expect(endedPlan(plan('2026-10-12', '2026-10-12'), season, '2026-11-02')).toEqual({
      state: 'nothing_since_last_race',
      date: '2026-10-11',
    });
  });

  it('is nothing for no plan, or one still running', () => {
    expect(endedPlan(undefined, undefined, '2026-11-02')).toBeNull();
    expect(endedPlan(plan('2026-12-01', '2026-12-01'), { state: 'a_race_ahead', race: null, ends: '2026-12-01' }, '2026-11-02')).toBeNull();
  });
});
