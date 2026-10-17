// ABOUTME: Unit tests for the words of a coach's plan draft card — the heading and lines a refused publish shows
// ABOUTME: Red if a conflict stops naming its weeks in the locale's list, or drops the re-laid season line

import { describe, expect, it } from 'vitest';
import { PLAN_DRAFT_KEYS, planConflictText } from '../src/plan-draft';

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
