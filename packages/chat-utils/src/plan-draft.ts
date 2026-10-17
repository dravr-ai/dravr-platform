// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The words of a coach's plan thread and its draft card — the catalogue keys, and the conflict a refused publish names
// ABOUTME: Both clients render from these, so a conflict reads the same on web and on the phone (carnet#1069/#1070)

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
} as const;

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
