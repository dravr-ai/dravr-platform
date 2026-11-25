// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: What a coach's athlete workspace reads off the subject-plan answer — the dates the draft changes over the live plan, a date's entries, the pre-publish check
// ABOUTME: One comparison for web and the phone, on the server's own rule: a date differs when its sessions do, the coach's marks aside (carnet#1816)

import type { PlanDay, WorkoutPlan } from '@pierre/shared-types';

/**
 * The fields of a day that say something about the coach's work on it rather
 * than about the session the athlete trains on: the coach's call
 * (carnet#1814), the editor's marks (carnet#1714, #1716), and the entry's
 * place, which the order of a date's entries already carries.
 */
const COACH_MARKS = ['coach_call', 'edited', 'added', 'position'] as const;

/**
 * The fields the server words in a language when it draws a day and stores
 * none of — the outside-evidence warning, "Counts as …": it compares the
 * session it stores, and the field beside each (`outside`,
 * `counts_as_purpose`) carries what it says. Left out, so a plan card saved
 * in one language reads alike with the same plan read in another.
 */
const WORDED_FIELDS = ['outside_evidence', 'counts_as'] as const;

/** The evidence chip's words in a language; its `kind`, `basis` and `note_kind` say the same. */
const WORDED_EVIDENCE_FIELDS = ['label', 'note'] as const;

/** `value` with every object's keys in one order and its absent fields dropped, so two equal days print alike. */
function canonical(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonical);
  if (value !== null && typeof value === 'object') {
    const record = value as Record<string, unknown>;
    return Object.fromEntries(
      Object.keys(record)
        .filter((key) => record[key] !== undefined)
        .sort()
        .map((key) => [key, canonical(record[key])]),
    );
  }
  return value;
}

/** A day as the comparison reads it: the session the athlete trains on, the coach's marks and the server's words aside. */
function trainingContent(day: PlanDay): string {
  const content: Record<string, unknown> = { ...day };
  for (const field of [...COACH_MARKS, ...WORDED_FIELDS]) delete content[field];
  if (day.evidence) {
    const evidence: Record<string, unknown> = { ...day.evidence };
    for (const field of WORDED_EVIDENCE_FIELDS) delete evidence[field];
    content.evidence = evidence;
  }
  return JSON.stringify(canonical(content));
}

/** The entries of `date` in `days`, in order, as the comparison reads them. */
function contentOn(days: readonly PlanDay[], date: string): string[] {
  return days.filter((day) => day.date === date).map(trainingContent);
}

/**
 * The dates whose entries the draft changes over the live plan, ascending:
 * the dates the workspace's calendar marks « Proposé », and the count its
 * publish button names.
 *
 * The rule is the server's (`training_plans::changed_dates`), so the
 * calendar marks the dates a publish announces in the room: each week of the
 * draft is compared with the live week of the same Monday, or with nothing
 * when the live plan holds none; a week only the live plan holds is not
 * compared. A date differs when its entries do, in order and in every field
 * but the coach's call and editor marks — a session rewritten, added, taken
 * away or moved within the date — and the words the server drew in a
 * language, which it never stores. With no live plan, every date the draft
 * holds differs.
 */
export function planProposedDates(draft: WorkoutPlan, live: WorkoutPlan | null): string[] {
  const changed = new Set<string>();
  for (const week of draft.weeks) {
    const prior = live?.weeks.find((held) => held.week_start === week.week_start)?.days ?? [];
    const dates = new Set([...prior, ...week.days].map((day) => day.date));
    for (const date of dates) {
      const was = contentOn(prior, date);
      const is = contentOn(week.days, date);
      if (was.length !== is.length || was.some((entry, index) => entry !== is[index])) {
        changed.add(date);
      }
    }
  }
  return [...changed].sort();
}

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
