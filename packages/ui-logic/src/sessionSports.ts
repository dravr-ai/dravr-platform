// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The sports a coach picks for a session on both clients — the one-tap chips, the day's own word, and a sport the coach names (carnet#1727)
// ABOUTME: The wire word is the plan's own free text; a chip reads as the card prints it, and only a family's three sports open the family mode

import { activitySportLabelKey } from '@pierre/shared-constants';
import { SESSION_SPORTS, type PlanDay, type SessionSport } from '@pierre/shared-types';

/**
 * The one-tap sports, in the editor's order. Each is the word the plan
 * stores, the agent writes and the calendar push maps (`strength` →
 * strength training, `mobility` → a generic workout); the chip shows the
 * label the card prints for it.
 */
export const QUICK_SPORTS = ['ride', 'run', 'swim', 'strength', 'mobility', 'yoga', 'crossfit'] as const;
export type QuickSport = (typeof QUICK_SPORTS)[number];

/** Whether two sport words name the same sport: the same label in the shared vocabulary, or the same word. */
export function sameSport(a: string, b: string): boolean {
  const left = a.trim();
  const right = b.trim();
  if (left.toLowerCase() === right.toLowerCase()) return left.length > 0;
  const key = activitySportLabelKey(left);
  return key !== null && key === activitySportLabelKey(right);
}

/** The one-tap sport a word names, or `null` for a sport the chips do not carry. */
export function quickSportOf(sport: string): QuickSport | null {
  return QUICK_SPORTS.find((quick) => sameSport(quick, sport)) ?? null;
}

/**
 * The sport chips the editor offers for a session that opened on `opening`:
 * the one-tap sports, then the day's own word when it names none of them,
 * so a sport the agent or the coach named stays one tap away. Built once
 * from the opening session, so the chips never reorder under the cursor.
 */
export function sportChoices(opening: string): string[] {
  const own = opening.trim();
  return own.length > 0 && quickSportOf(own) === null ? [...QUICK_SPORTS, own] : [...QUICK_SPORTS];
}

/** The chip a sport word selects among `choices`, or `null` when the coach named a sport of their own. */
export function chosenSport(sport: string, choices: readonly string[]): string | null {
  return choices.find((choice) => sameSport(choice, sport)) ?? null;
}

/**
 * The family sport a session's sport word reads as, when it is one a
 * family is written for: the chip's own word, or the day's sport as the
 * server read it (`family_sport`) while the coach keeps the day's word.
 * `undefined` for every other sport, which opens no family mode.
 */
export function familySportFor(sport: string, opening: Pick<PlanDay, 'sport' | 'family_sport'>): SessionSport | undefined {
  const word = sport.trim();
  const own = SESSION_SPORTS.find((family) => family === word);
  if (own) return own;
  if (opening.family_sport !== undefined && sameSport(word, opening.sport)) return opening.family_sport;
  return undefined;
}
