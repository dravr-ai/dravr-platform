// ABOUTME: Unit tests for the words of a coach's plan draft card and room — a refused publish's lines, whose room it is
// ABOUTME: Red if a conflict stops naming its weeks in the locale's list, or drops the re-laid season line

import { describe, expect, it } from 'vitest';
import { coachRoomName, PLAN_DRAFT_KEYS, planConflictText } from '../src/plan-draft';

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
