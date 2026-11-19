// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: What web and phone share for the member's record of their human coach's reads — its words, the class labels, the outside-tool mark, the notice
// ABOUTME: A constants module cannot translate, so keys are named here and each client resolves them (carnet#1445)

import type { CoachReadDataClass } from '@pierre/shared-types';

/** The words of the member's record of their human coach's reads. */
export const COACH_READ_LOG_KEYS = {
  title: 'humanCoach.readLog.title',
  hint: 'humanCoach.readLog.hint',
  empty: 'humanCoach.readLog.empty',
  formerCoach: 'humanCoach.readLog.formerCoach',
  loadFailed: 'humanCoach.readLog.loadFailed',
  /** Beside a read the server marked as served to an AI tool the coach connects to Dravr. */
  servedToAssistant: 'humanCoach.readLog.servedToAssistant',
} as const;

/**
 * The words of the one-time notice to a member who shares with a human coach
 * (`assistant_notice_due` on their coach sharing): AI tools the coach connects
 * to Dravr read them by name. Its title, its body (naming `{{coach}}`), and its
 * two answers, worded as the wellness notice's: keeping is a fresh grant of
 * coach sharing, stopping is its revoke.
 */
export const COACH_ASSISTANT_NOTICE_KEYS = {
  title: 'humanCoach.assistantNotice.title',
  body: 'humanCoach.assistantNotice.body',
  keep: 'delegation.wellnessWidenedKeep',
  stop: 'delegation.wellnessRevokeConfirm',
} as const;

/** The key of the words each kind of data a read reached reads as. */
const COACH_READ_CLASS_KEYS: Record<CoachReadDataClass, string> = {
  training: 'humanCoach.readLog.training',
  wellness: 'humanCoach.readLog.wellness',
};

/** The key of the words a read's kind of data reads as. */
export function coachReadClassKey(dataClass: CoachReadDataClass): string {
  return COACH_READ_CLASS_KEYS[dataClass];
}

/**
 * The key of the words each read made outside a tool call reads as, by the
 * label the server records in place of a tool (`CoachReadSurface`): Group
 * info's plan view, `/plan <athlete>`, the coach's roster, the athlete block
 * of the coach's thread about them, and the group views built from every
 * member's training — the room's member cards, the group's stats, weekly
 * report and health flags, and the coach's weekly digest — and Group info's
 * view of the athlete's thresholds (carnet#1561).
 */
const COACH_READ_SOURCE_KEYS: Readonly<Record<string, string>> = {
  plan_view: 'humanCoach.readLog.sourcePlanView',
  plan_command: 'humanCoach.readLog.sourcePlanCommand',
  roster: 'humanCoach.readLog.sourceRoster',
  coach_thread: 'humanCoach.readLog.sourceCoachThread',
  group_room: 'humanCoach.readLog.sourceGroupRoom',
  group_stats: 'humanCoach.readLog.sourceGroupStats',
  group_report: 'humanCoach.readLog.sourceGroupReport',
  group_health: 'humanCoach.readLog.sourceGroupHealth',
  weekly_digest: 'humanCoach.readLog.sourceWeeklyDigest',
  physiology_view: 'humanCoach.readLog.sourcePhysiologyView',
};

/**
 * The key of the words a read's source reads as, or `null` when the source is
 * a tool, which the record names as it is.
 */
export function coachReadSourceKey(tool: string): string | null {
  return Object.prototype.hasOwnProperty.call(COACH_READ_SOURCE_KEYS, tool) ? COACH_READ_SOURCE_KEYS[tool] : null;
}

/** Every key the record asks the catalogue for, for a test that pins them. */
export const COACH_READ_LOG_ALL_KEYS: readonly string[] = [
  ...Object.values(COACH_READ_LOG_KEYS),
  ...Object.values(COACH_ASSISTANT_NOTICE_KEYS),
  ...Object.values(COACH_READ_CLASS_KEYS),
  ...Object.values(COACH_READ_SOURCE_KEYS),
];
