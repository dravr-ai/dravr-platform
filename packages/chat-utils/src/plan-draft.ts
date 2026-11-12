// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The words of a coach's plan thread, its draft card and the group room as the coach reads it — keys, and a refused publish's conflict
// ABOUTME: Both clients render from these, so a conflict reads the same on web and on the phone (carnet#1069/#1070)

import type { CalendarPushOutcome, CoachingGroup } from '@pierre/shared-types';
import { formatList } from './list-format';
import type { Translate } from './text';

/** The catalogue keys of a coach's plan thread: the Group info action, the list kind, the header line. */
export const PLAN_THREAD_KEYS = {
  /** "Work on {{name}}'s plan" — the member row action in Group info. */
  open: 'humanCoach.planThread.open',
  /** "Plan · {{name}}" — the thread header's line and the row glyph's tooltip. */
  subtitle: 'humanCoach.planThread.subtitle',
  /** "Athlete plan" — what a screen reader hears for the row's kind glyph. */
  kindLabel: 'humanCoach.planThread.kindLabel',
  /** The thread could not be opened. */
  openError: 'humanCoach.planThread.openError',
  /** The coach's empty plan thread: their planning space with the agent, in their voice. */
  empty: 'humanCoach.planThread.empty',
  /** "Write to the agent about {{name}}'s plan…" — the coach's composer in the thread. */
  composerPlaceholder: 'humanCoach.planThread.composerPlaceholder',
  /** "Draft {{name}}'s plan" — the one-tap start in the coach's empty plan thread. */
  starterLabel: 'humanCoach.planThread.starterLabel',
  /** What that start sends to the agent, in the coach's words. */
  starterMessage: 'humanCoach.planThread.starterMessage',
} as const;

/** The catalogue keys of a group's room as its human coach reads it. */
export const GROUP_ROOM_KEYS = {
  /** "Group room" — the header line under a title that already names the group. */
  kindLabel: 'humanCoach.groupRoom.kindLabel',
  /** The coach's empty room: who reads what they write there. */
  empty: 'humanCoach.groupRoom.empty',
  /** "Write to {{group}}…" — the coach's composer in the room. */
  composerPlaceholder: 'humanCoach.groupRoom.composerPlaceholder',
  /** The room could not be opened. */
  openError: 'humanCoach.groupRoom.openError',
} as const;

/**
 * The group's name when `viewerId` is the human coach of the room's group —
 * the reader whose empty state and composer are the room's, not an
 * athlete's — else `null`. A member, the coach of another group, or a room
 * whose group has not been read yet keeps the athlete's words.
 */
export function coachRoomName(
  group: Pick<CoachingGroup, 'name' | 'coach_user_id'> | null,
  viewerId: string | null,
  fallbackName: string | null = null,
): string | null {
  if (group === null || viewerId === null || group.coach_user_id !== viewerId) return null;
  return group.name.trim() || fallbackName?.trim() || null;
}

/** The catalogue keys of the draft controls on a plan card. */
export const PLAN_DRAFT_KEYS = {
  /** "Draft · {{name}} can't see it yet". */
  badge: 'humanCoach.planDraft.badge',
  /** "Publish to {{name}}". */
  publish: 'humanCoach.planDraft.publish',
  /** "Discard draft" — the button, and the confirmation's title and action. */
  discard: 'humanCoach.planDraft.discard',
  /** The discard confirmation's question, naming the athlete. */
  discardConfirm: 'humanCoach.planDraft.discardConfirm',
  /** The publish landed and the athlete was told. */
  published: 'humanCoach.planDraft.published',
  /** The heading over a refused publish's conflicts. */
  conflictTitle: 'humanCoach.planDraft.conflictTitle',
  /** The weeks both the draft and the athlete changed, as `{{weeks}}`. */
  conflictWeeks: 'humanCoach.planDraft.conflictWeeks',
  /** The athlete's season was re-laid under the draft. */
  conflictSeason: 'humanCoach.planDraft.conflictSeason',
  /** "Publish anyway" — publishes over the conflicts. */
  overwrite: 'humanCoach.planDraft.overwrite',
  /** The athlete stopped sharing with the coach. */
  consentRevoked: 'humanCoach.planDraft.consentRevoked',
  /** The publish put the plan on the athlete's calendar, every day of it. */
  pushed: 'humanCoach.planDraft.pushed',
  /** It did, but some days were left alone or refused. */
  pushedPartly: 'humanCoach.planDraft.pushedPartly',
  /** Not on the calendar: the athlete connects none Dravr writes. */
  pushNoCalendar: 'humanCoach.planDraft.pushNoCalendar',
  /** Not on the calendar: the athlete has not allowed the coach calendar writes. */
  pushNoConsent: 'humanCoach.planDraft.pushNoConsent',
  /** Not on the calendar: several qualify and the athlete chose none. */
  pushChooseCalendar: 'humanCoach.planDraft.pushChooseCalendar',
  /** Not on the calendar: the athlete's calendar platform does not let the coach write it. */
  pushNoAccess: 'humanCoach.planDraft.pushNoAccess',
  /** Not on the calendar, for any other reason: the agent in the thread can say which. */
  pushNotWritten: 'humanCoach.planDraft.pushNotWritten',
} as const;

/** The line each refusal code a publish's push names is said with; any other gets `pushNotWritten`. */
const PUSH_REFUSAL_KEYS: Readonly<Record<string, string>> = {
  no_calendar: PLAN_DRAFT_KEYS.pushNoCalendar,
  no_calendar_write_consent: PLAN_DRAFT_KEYS.pushNoConsent,
  choose_calendar: PLAN_DRAFT_KEYS.pushChooseCalendar,
  coach_follower: PLAN_DRAFT_KEYS.pushNoAccess,
  athlete_no_access: PLAN_DRAFT_KEYS.pushNoAccess,
};

/**
 * Whether a publish's push put every day of the plan on the calendar: it ran,
 * and left no day alone or unwritten. Both clients colour the calendar line
 * by it, and {@link planPushText} words it, so the two never disagree.
 */
export function planPushComplete(push: CalendarPushOutcome): boolean {
  return push.status === 'pushed' && push.report.skipped.length === 0 && push.report.failed.length === 0;
}

/**
 * The line a published card adds about `name`'s calendar (carnet#1492): the
 * plan is on it, on it but for some days, or not on it and why. A refusal is
 * worded by its code, never with the server's English message, so it reads
 * in the coach's language; a code the card has no words for sends the coach
 * to the agent in the thread, which reads the full reason.
 */
export function planPushText(push: CalendarPushOutcome, name: string, t: Translate): string {
  if (push.status === 'pushed') {
    return t(planPushComplete(push) ? PLAN_DRAFT_KEYS.pushed : PLAN_DRAFT_KEYS.pushedPartly, { name });
  }
  return t(PUSH_REFUSAL_KEYS[push.reason] ?? PLAN_DRAFT_KEYS.pushNotWritten, { name });
}

/** The catalogue keys of the read-only plan a coach opens from Group info (carnet#1244). */
export const PLAN_VIEW_KEYS = {
  /** "View {{name}}'s plan" — the member row action. */
  open: 'humanCoach.planView.open',
  /** "{{name}}'s plan" — the sheet's title. */
  title: 'humanCoach.planView.title',
  /** The line saying the plan is the one the athlete sees, and cannot be edited here. */
  readOnly: 'humanCoach.planView.readOnly',
  /** The athlete has no published plan. */
  empty: 'humanCoach.planView.empty',
  /** The athlete stopped sharing with the coach, so nothing is shown. */
  consentRevoked: 'humanCoach.planView.consentRevoked',
  /** The plan could not be read. */
  loadError: 'humanCoach.planView.loadError',
} as const;

/** What a refused publish collided with: the weeks both sides changed, and a re-laid season. */
export interface PlanDraftConflict {
  /** The Mondays (`YYYY-MM-DD`) of the weeks both the draft and the athlete changed. */
  weeks: string[];
  seasonRelaid: boolean;
}

/** A refused publish as the card prints it: one heading and a line per collision. */
export interface PlanConflictText {
  title: string;
  lines: string[];
}

/**
 * The heading and lines a card shows over a refused publish.
 *
 * The weeks are the Mondays the plan card itself heads its weeks with, so the
 * coach finds each one on the card above, joined the way `locale` joins a
 * list. A re-laid season gets its own line; a conflict carries one or both.
 */
export function planConflictText(
  conflict: PlanDraftConflict,
  name: string,
  locale: string,
  t: Translate,
): PlanConflictText {
  const lines: string[] = [];
  if (conflict.weeks.length > 0) {
    lines.push(t(PLAN_DRAFT_KEYS.conflictWeeks, { weeks: formatList(conflict.weeks, locale) }));
  }
  if (conflict.seasonRelaid) {
    lines.push(t(PLAN_DRAFT_KEYS.conflictSeason, { name }));
  }
  return { title: t(PLAN_DRAFT_KEYS.conflictTitle, { name }), lines };
}
