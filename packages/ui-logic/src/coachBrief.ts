// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach's morning brief on both clients — its status line, its hours, and the lists its settings replace in full (carnet#1791)
// ABOUTME: Pure over the client's own t(): the status reads the last run's outcome, a paused brief says so, and a change is the brief the server will answer

import type { CoachAttentionKind, CoachBrief, CoachBriefOutcome, CoachBriefState, CoachBriefUpdate } from '@pierre/shared-types';
import { COACH_BRIEF_REASON_LABEL_KEY } from '@pierre/shared-constants';
import { formatInstant, type Translate } from './homeFormat';

/** The first and last hour a coach may have the brief at, coach-local. */
const FIRST_HOUR = 5;
const LAST_HOUR = 23;

/** Every hour the brief can post at, earliest first: 05 through 23. */
export const BRIEF_HOURS: readonly number[] = Array.from({ length: LAST_HOUR - FIRST_HOUR + 1 }, (_, i) => FIRST_HOUR + i);

/** An attention reason the brief checks: every kind but the stop it only counts. */
export type BriefReasonKind = keyof typeof COACH_BRIEF_REASON_LABEL_KEY;

/** Every reason the brief can check, in the order its settings draw them. */
export const BRIEF_REASON_KINDS = Object.keys(COACH_BRIEF_REASON_LABEL_KEY) as readonly BriefReasonKind[];

/** The states and outcomes this client can word: a brief naming any other is not one it draws. */
const BRIEF_STATES: readonly CoachBriefState[] = ['active', 'paused'];
const BRIEF_OUTCOMES: readonly CoachBriefOutcome[] = ['posted', 'withheld', 'empty', 'held', 'skipped'];

/**
 * Whether an answer is a brief this client can draw. Anything else — the
 * HTML page a proxy or a dev server's fallback answers an unknown route with,
 * a body from a server that predates the route, an outcome this build cannot
 * word — is refused where it comes in, so the queue shows no brief rather
 * than a broken page.
 */
export function isCoachBrief(value: unknown): value is CoachBrief {
  if (value === null || typeof value !== 'object') return false;
  const brief = value as Record<string, unknown>;
  const run = brief.last_run as Record<string, unknown> | null | undefined;
  const runOk =
    run === null ||
    (typeof run === 'object' &&
      typeof run.ran_at === 'string' &&
      typeof run.items === 'number' &&
      typeof run.not_sharing === 'number' &&
      typeof run.unreadable === 'number' &&
      BRIEF_OUTCOMES.includes(run.outcome as CoachBriefOutcome));
  return (
    BRIEF_STATES.includes(brief.state as CoachBriefState) &&
    typeof brief.hour === 'number' &&
    Array.isArray(brief.reasons_off) &&
    Array.isArray(brief.muted) &&
    runOk
  );
}

/** A run's time of day: 24-hour, minutes always shown, in the device's zone. */
const RUN_TIME: Intl.DateTimeFormatOptions = { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' };

/** An hour as the brief's schedule names it: `07:00`. */
export function briefHourLabel(hour: number): string {
  return `${String(hour).padStart(2, '0')}:00`;
}

/** What the brief's line says: where it stands, and when it runs while it runs. */
export interface BriefStatusWords {
  /** The last run's outcome, "Paused", or "Hasn't run yet". */
  status: string;
  /** "Every day at 07:00" while active; null while paused, when nothing runs at that hour. */
  schedule: string | null;
}

/**
 * The brief's status in words. A paused brief says only that it is paused:
 * a schedule beside it would promise a run that will not come. Otherwise the
 * last run speaks: nothing to report (with how many it could not judge, when
 * any), sent with how many to handle, or held back (`held`, `withheld` and
 * `skipped` all mean it ran and sent nothing the coach asked for), each at
 * the time it ran on this device's clock.
 */
export function briefStatusWords(t: Translate, language: string, brief: CoachBrief): BriefStatusWords {
  if (brief.state === 'paused') return { status: t('coachHome.brief.status_paused'), schedule: null };
  const schedule = t('coachHome.brief.schedule', { time: briefHourLabel(brief.hour) });
  const run = brief.last_run;
  if (run === null) return { status: t('coachHome.brief.status_never'), schedule };
  const time = formatInstant(run.ran_at, language, RUN_TIME);
  switch (run.outcome) {
    case 'empty': {
      // Nothing to report is not "all fine" when some athletes were not read.
      const leftOut = run.not_sharing + run.unreadable;
      return leftOut > 0
        ? { status: t('coachHome.brief.status_empty_left_out', { time, count: leftOut }), schedule }
        : { status: t('coachHome.brief.status_empty', { time }), schedule };
    }
    case 'posted':
      return { status: t('coachHome.brief.status_posted', { time, count: run.items }), schedule };
    case 'held':
    case 'withheld':
    case 'skipped':
      return { status: t('coachHome.brief.status_held', { time }), schedule };
  }
}

/** The status and the schedule on one line, as the queue's brief control prints them. */
export function briefStatusLine(words: BriefStatusWords): string {
  return words.schedule === null ? words.status : `${words.status} · ${words.schedule}`;
}

/**
 * The brief as the server will answer a change, for showing it before the
 * answer lands: `paused` moves the state, and each list given replaces the
 * stored one whole, as the PUT does.
 */
export function applyBriefUpdate(brief: CoachBrief, update: CoachBriefUpdate): CoachBrief {
  return {
    ...brief,
    state: update.paused === undefined ? brief.state : update.paused ? 'paused' : 'active',
    hour: update.hour ?? brief.hour,
    reasons_off: update.reasons_off ?? brief.reasons_off,
    muted: update.muted ?? brief.muted,
  };
}

/** Whether the brief leaves this athlete out. */
export function isBriefMuted(brief: CoachBrief, athleteId: string): boolean {
  return brief.muted.includes(athleteId);
}

/**
 * The whole `muted` list with one athlete left out (`mute`) or put back. The
 * PUT replaces the list, so every other id travels unchanged, including any
 * the roster no longer names.
 */
export function withMuted(muted: readonly string[], athleteId: string, mute: boolean): string[] {
  const rest = muted.filter((id) => id !== athleteId);
  return mute ? [...rest, athleteId] : rest;
}

/** The whole `reasons_off` list with one reason checked (`on`) or not, in the settings' order. */
export function withReason(reasonsOff: readonly CoachAttentionKind[], kind: CoachAttentionKind, on: boolean): CoachAttentionKind[] {
  const off = new Set(reasonsOff);
  if (on) off.delete(kind);
  else off.add(kind);
  // A kind this client does not draw stays off, after the ones it draws.
  const drawn: readonly CoachAttentionKind[] = BRIEF_REASON_KINDS;
  return [...BRIEF_REASON_KINDS.filter((k) => off.has(k)), ...reasonsOff.filter((k) => !drawn.includes(k))];
}

/**
 * The athletes the brief leaves out that the roster names, in the order they
 * were left out. An id the roster no longer carries has no name to show and
 * is not listed; it stays in `muted` all the same.
 */
export function mutedAthletes<A extends { user_id: string }>(muted: readonly string[], athletes: readonly A[]): A[] {
  const byId = new Map(athletes.map((athlete) => [athlete.user_id, athlete]));
  return muted.flatMap((id) => {
    const athlete = byId.get(id);
    return athlete === undefined ? [] : [athlete];
  });
}
