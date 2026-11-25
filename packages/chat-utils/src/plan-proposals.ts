// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: What a coach's athlete workspace reads off a subject plan — a date's entries, why a session is the coach's call to make, the pre-publish check
// ABOUTME: Which dates are proposed is the server's to say (`proposed_dates`, carnet#1816); nothing here compares the draft with the live plan

import type { PlanDay, WorkoutPlan } from '@pierre/shared-types';

/**
 * A week's entries by date, in the order the dates come, each date's entries
 * in the order they are done: a date may hold several (carnet#1716). The plan
 * card's table and the workspace's calendar lay a week out from this.
 */
export function entriesByDate(days: readonly PlanDay[]): PlanDay[][] {
  const byDate = new Map<string, PlanDay[]>();
  for (const day of days) {
    const entries = byDate.get(day.date);
    if (entries) entries.push(day);
    else byDate.set(day.date, [day]);
  }
  return [...byDate.values()];
}

/** Why a session's evidence is one the coach may keep as their call: it rests on coaching convention, or it sits outside its family's evidence. */
export type EvidenceCallReason = 'convention' | 'outside';

/**
 * Why a session is one the coach may keep as their call, « C'est mon choix »
 * (carnet#1814), or `null` for a rest day and for a session whose evidence
 * the check has nothing to say about. A session outside its family's evidence
 * says so first: it is the stronger of the two.
 */
export function evidenceCallReason(
  day: Pick<PlanDay, 'rest' | 'evidence' | 'outside' | 'outside_evidence'>,
): EvidenceCallReason | null {
  if (day.rest) return null;
  if (day.outside === true || day.outside_evidence !== undefined) return 'outside';
  if (day.evidence?.kind === 'convention') return 'convention';
  return null;
}

/** One session the pre-publish check names. */
export interface PrePublishItem {
  day: PlanDay;
  /** The session's place among its date's entries, from 0: the position a day write names. */
  position: number;
  /** Every entry on the session's date, in order, for the editor « Revoir » opens. */
  entries: PlanDay[];
  reason: EvidenceCallReason;
}

/**
 * The sessions the coach is asked about before publishing (carnet#1816):
 * those of the draft from `today` on that rest on coaching convention or sit
 * outside their family's evidence, and that the coach has not already kept as
 * their call. In date order, a date's sessions in the order they are done. A
 * day already past is not asked about: publishing changes nothing the athlete
 * still has to do there.
 */
export function prePublishChecks(draft: WorkoutPlan, today: string): PrePublishItem[] {
  const items: PrePublishItem[] = [];
  for (const week of draft.weeks) {
    for (const entries of entriesByDate(week.days)) {
      entries.forEach((day, position) => {
        if (day.date < today || day.coach_call === true) return;
        const reason = evidenceCallReason(day);
        if (reason !== null) items.push({ day, position, entries, reason });
      });
    }
  }
  return items.sort((a, b) => a.day.date.localeCompare(b.day.date) || a.position - b.position);
}
