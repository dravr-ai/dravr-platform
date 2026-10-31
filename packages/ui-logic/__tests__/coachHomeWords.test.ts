// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the words of one roster row — last activity, form band (dated when cut), plan, agent week, thread and a non-sharer's status
// ABOUTME: Run on a key-echoing translator so a catalogue edit cannot hide a rule that changed

import { describe, it, expect } from 'vitest';
import type { CoachRosterAthlete } from '@pierre/shared-types';
import {
  agentWords,
  formWords,
  lastWhenWords,
  nonSharerStatusWords,
  planWords,
  threadWords,
} from '../src/coachHomeWords';

const echo = (key: string, options?: Record<string, unknown>) =>
  options === undefined ? key : `${key}${JSON.stringify(options)}`;

function athlete(over: Partial<CoachRosterAthlete>): CoachRosterAthlete {
  return { user_id: 'a', name: 'A', groups: [], status: 'shared', attention: [], ...over };
}

describe('lastWhenWords', () => {
  it('names today, yesterday, n days ago and nothing on record, and stresses the last two', () => {
    expect(lastWhenWords(echo, 0)).toEqual({ text: 'coachHome.roster.lastToday', recent: false });
    expect(lastWhenWords(echo, 1)).toEqual({ text: 'coachHome.roster.lastYesterday', recent: false });
    expect(lastWhenWords(echo, 6)).toEqual({ text: 'coachHome.roster.lastDaysAgo{"count":6}', recent: true });
    expect(lastWhenWords(echo, null)).toEqual({ text: 'coachHome.roster.lastNever', recent: true });
  });
});

describe('formWords', () => {
  const training = (stale: boolean, reconnect: string[]) => ({
    form: { band: 'balanced' as const, as_of: '2026-10-22' },
    ctl: 60,
    days_since_last_activity: 6,
    last_session: null,
    last_activity_per_source: {},
    needs_reconnect: reconnect,
    stale,
  });

  it('prints the band alone while the data is live', () => {
    const words = formWords(echo, 'en', athlete({ training: training(false, []) }));
    expect(words.cut).toBe(false);
    expect(words.asOf).toBeNull();
    expect(words.text).toContain('balanced');
  });

  it('dates a reading cut by a stale cache or a source to reconnect', () => {
    for (const cutTraining of [training(true, []), training(false, ['garmin'])]) {
      const words = formWords(echo, 'en', athlete({ training: cutTraining }));
      expect(words.cut).toBe(true);
      expect(words.text).not.toContain('coachHome.roster.staleBand');
      expect(words.asOf).toContain('coachHome.roster.staleBand');
    }
  });

  it('reads an athlete with no training as insufficient history, not cut', () => {
    const words = formWords(echo, 'en', athlete({}));
    expect(words.cut).toBe(false);
    expect(words.text).toContain('insufficient');
  });
});

describe('planWords, agentWords, threadWords', () => {
  it('says no plan for a sharer without one', () => {
    expect(planWords(echo, athlete({}))).toEqual({ phase: 'coachHome.roster.planNone', week: '' });
  });

  it('counts the agent week by its unmatched figures first', () => {
    const replies = { count: 3, verified: 2, unmatched_figures: 1, capped: false };
    expect(agentWords(echo, 'en', athlete({ agent_replies: replies }))).toContain('coachHome.detail.verdictUnverifiedOne');
    expect(agentWords(echo, 'en', athlete({}))).toBe('coachHome.roster.agentNone');
  });

  it('asks the coach for a draft to read and a plan to create, and is quiet otherwise', () => {
    const draft = athlete({
      plan_thread: { draft_updated_at: '2026-10-29T10:00:00Z', draft_has_conflicts: false, published_at: null, draft_phases: [] },
    });
    expect(threadWords(echo, 'en', draft)).toEqual({ text: 'coachHome.roster.threadDraft', strong: true });
    expect(threadWords(echo, 'en', athlete({}))).toEqual({ text: 'coachHome.roster.threadCreate', strong: true });
  });
});

describe('nonSharerStatusWords', () => {
  it('names a source-less athlete, one who stopped on a date, and one who never accepted', () => {
    expect(nonSharerStatusWords(echo, 'en', athlete({ status: 'no_source' }))).toBe('coachHome.roster.noShareNoSource');
    expect(
      nonSharerStatusWords(echo, 'en', athlete({ status: 'no_coach_consent', sharing_stopped_at: '2026-10-20T08:00:00Z' })),
    ).toContain('coachHome.roster.noShareRevoked');
    expect(nonSharerStatusWords(echo, 'en', athlete({ status: 'no_coach_consent' }))).toBe('coachHome.roster.noShareNever');
  });
});
