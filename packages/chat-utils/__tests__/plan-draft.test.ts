// ABOUTME: Unit tests for the words of a coach's plan draft card and room — a refused publish's lines, whose room it is
// ABOUTME: Red if a conflict stops naming its weeks in the locale's list, or drops the re-laid season line

import { describe, expect, it } from 'vitest';
import { coachRoomName, PLAN_DRAFT_KEYS, planConflictText, planPushComplete, planPushText } from '../src/plan-draft';

/** A translator that shows which key it was asked for and with what. */
function t(key: string, params?: Record<string, string | number>): string {
  return params ? `${key} ${JSON.stringify(params)}` : key;
}

describe('planConflictText', () => {
  it('names the weeks both sides changed, joined the way the locale joins a list', () => {
    const text = planConflictText({ weeks: ['2026-10-19', '2026-10-26'], seasonRelaid: false }, 'Alice', 'en', t);

    expect(text.title).toBe(`${PLAN_DRAFT_KEYS.conflictTitle} {"name":"Alice"}`);
    expect(text.lines).toEqual([`${PLAN_DRAFT_KEYS.conflictWeeks} {"weeks":"2026-10-19 and 2026-10-26"}`]);
  });

  it('says the season was re-laid when it was, on a line of its own', () => {
    const text = planConflictText({ weeks: [], seasonRelaid: true }, 'Alice', 'fr', t);

    expect(text.lines).toEqual([`${PLAN_DRAFT_KEYS.conflictSeason} {"name":"Alice"}`]);
  });

  it('says both when a publish collided with both', () => {
    const text = planConflictText({ weeks: ['2026-10-19'], seasonRelaid: true }, 'Alice', 'fr', t);

    expect(text.lines).toEqual([
      `${PLAN_DRAFT_KEYS.conflictWeeks} {"weeks":"2026-10-19"}`,
      `${PLAN_DRAFT_KEYS.conflictSeason} {"name":"Alice"}`,
    ]);
  });
});

describe('planPushText', () => {
  const report = { provider: 'intervals_icu', created: 2, updated: 1, removed: 0, skipped: [], failed: [] };
  const notPushed = (reason: string) => ({ status: 'not_pushed' as const, reason, message: 'server words' });

  it('says the plan is on the calendar when every day was written', () => {
    expect(planPushText({ status: 'pushed', report }, 'Alice', t)).toBe(`${PLAN_DRAFT_KEYS.pushed} {"name":"Alice"}`);
  });

  it('says some days were not written when the push skipped or failed any', () => {
    const day = { date: '2026-11-15', name: 'Tempo' };
    for (const partial of [
      { ...report, skipped: [day] },
      { ...report, failed: [day] },
    ]) {
      expect(planPushText({ status: 'pushed', report: partial }, 'Alice', t)).toBe(
        `${PLAN_DRAFT_KEYS.pushedPartly} {"name":"Alice"}`,
      );
    }
  });

  it("words each refusal by its code, never with the server's English message", () => {
    const cases: Array<[string, string]> = [
      ['no_calendar', PLAN_DRAFT_KEYS.pushNoCalendar],
      ['no_calendar_write_consent', PLAN_DRAFT_KEYS.pushNoConsent],
      ['choose_calendar', PLAN_DRAFT_KEYS.pushChooseCalendar],
      ['coach_follower', PLAN_DRAFT_KEYS.pushNoAccess],
      ['athlete_no_access', PLAN_DRAFT_KEYS.pushNoAccess],
    ];
    for (const [reason, key] of cases) {
      expect(planPushText(notPushed(reason), 'Alice', t)).toBe(`${key} {"name":"Alice"}`);
    }
  });

  it('sends the coach to the agent for a refusal the card has no words for', () => {
    expect(planPushText(notPushed('push_failed'), 'Alice', t)).toBe(`${PLAN_DRAFT_KEYS.pushNotWritten} {"name":"Alice"}`);
  });
});

describe('planPushComplete', () => {
  const report = { provider: 'intervals_icu', created: 2, updated: 1, removed: 0, skipped: [], failed: [] };
  const day = { date: '2026-11-15', name: 'Tempo' };

  it('is complete only for a push that wrote every day', () => {
    expect(planPushComplete({ status: 'pushed', report })).toBe(true);
    expect(planPushComplete({ status: 'pushed', report: { ...report, skipped: [day] } })).toBe(false);
    expect(planPushComplete({ status: 'pushed', report: { ...report, failed: [day] } })).toBe(false);
    expect(planPushComplete({ status: 'not_pushed', reason: 'no_calendar', message: 'server words' })).toBe(false);
  });
});

describe('coachRoomName', () => {
  const group = { name: 'Tri Squad', coach_user_id: 'coach-1' };

  it("names the room for the group's human coach", () => {
    expect(coachRoomName(group, 'coach-1')).toBe('Tri Squad');
  });

  it('keeps the athlete words for a member, a signed-out viewer, or a group not read yet', () => {
    expect(coachRoomName(group, 'member-1')).toBeNull();
    expect(coachRoomName(group, null)).toBeNull();
    expect(coachRoomName(null, 'coach-1')).toBeNull();
    expect(coachRoomName({ name: 'Tri Squad', coach_user_id: null }, 'coach-1')).toBeNull();
  });

  it("falls back on the conversation's group name when the group carries a blank one", () => {
    expect(coachRoomName({ ...group, name: '  ' }, 'coach-1', 'Tri')).toBe('Tri');
  });
});
