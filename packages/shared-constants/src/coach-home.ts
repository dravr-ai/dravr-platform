// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The words Coach Home prints for what the roster endpoint names — actions, evidence sources, season states, phases, counts
// ABOUTME: The server sends keys and numbers only; web and mobile resolve them through these tables and their own t() (carnet#1250, #1252, #1253)

import type {
  CoachAttentionAction,
  CoachEvidenceSourceKind,
  CoachRosterPhaseKind,
  CoachSeasonState,
} from '@pierre/shared-types';
import type { CountKeys } from '@pierre/chat-utils';

/** The label of each action a reason names as its primary one. */
export const COACH_ACTION_LABEL_KEY: Record<CoachAttentionAction, string> = {
  write_to_athlete: 'coachHome.action.writeToAthlete',
  correct_reply: 'coachHome.action.correctReply',
  open_plan_thread: 'coachHome.action.openPlanThread',
  review_draft: 'coachHome.action.reviewDraft',
  create_plan: 'coachHome.action.createPlan',
};

/** The label of each place an evidence line comes from. */
export const COACH_EVIDENCE_SOURCE_LABEL_KEY: Record<CoachEvidenceSourceKind, string> = {
  provider: 'coachHome.source.provider',
  training_load: 'coachHome.source.training_load',
  connection: 'coachHome.source.connection',
  cache: 'coachHome.source.cache',
  consent: 'coachHome.source.consent',
  verification: 'coachHome.source.verification',
  plan: 'coachHome.source.plan',
  plan_thread: 'coachHome.source.plan_thread',
  membership: 'coachHome.source.membership',
};

/** The line a row of the season view carries for where the athlete's season stands. */
export const COACH_SEASON_STATE_KEY: Record<CoachSeasonState, string> = {
  a_race_ahead: 'coachHome.season.a_race_ahead',
  no_a_race: 'coachHome.season.no_a_race',
  nothing_since_last_race: 'coachHome.season.nothing_since_last_race',
  plan_ended: 'coachHome.season.plan_ended',
  draft_only: 'coachHome.season.draft_only',
  new_athlete: 'coachHome.season.new_athlete',
  no_plan: 'coachHome.season.no_plan',
};

/**
 * The name of each phase kind on a plan: the plan card's own words, so a
 * phase is called the same thing on the athlete's plan and on the coach's
 * season view.
 */
export const COACH_PHASE_LABEL_KEY: Record<CoachRosterPhaseKind | 'draft', string> = {
  prep: 'plan.card.phase.prep',
  base: 'plan.card.phase.base',
  build: 'plan.card.phase.build',
  specialty: 'plan.card.phase.specialty',
  peak: 'plan.card.phase.peak',
  taper: 'plan.card.phase.taper',
  race: 'plan.card.phase.race',
  transition: 'plan.card.phase.transition',
  recovery: 'plan.card.phase.recovery',
  draft: 'coachHome.phase.draft',
};

/** How many athletes share, of how many the coach holds: « 12 athlètes sur 14 partagent… ». */
export const COACH_SCOPE_KEYS: CountKeys = {
  one: 'coachHome.scopeOne',
  many: 'coachHome.scopeN',
};

/** A number of days, as a phrase a reason embeds: « 6 jours ». */
export const COACH_DAY_UNIT_KEYS: CountKeys = {
  one: 'coachHome.unit.dayOne',
  many: 'coachHome.unit.dayN',
};

/** A run of mornings, as a phrase a reason embeds: « 3 matins de suite ». */
export const COACH_MORNING_UNIT_KEYS: CountKeys = {
  one: 'coachHome.unit.morningOne',
  many: 'coachHome.unit.morningN',
};

/** The agent's replies in the athlete's groups this week, with how many were verified. */
export const COACH_AGENT_COUNT_KEYS: CountKeys = {
  one: 'coachHome.detail.agentCountOne',
  many: 'coachHome.detail.agentCountN',
};

/** The same count when every reply was verified. */
export const COACH_AGENT_ALL_VERIFIED_KEYS: CountKeys = {
  one: 'coachHome.detail.agentAllVerifiedOne',
  many: 'coachHome.detail.agentAllVerifiedN',
};

/** The figures of a reply no source in the athlete's data backs: the verdict chip. */
export const COACH_UNVERIFIED_KEYS: CountKeys = {
  one: 'coachHome.detail.verdictUnverifiedOne',
  many: 'coachHome.detail.verdictUnverifiedN',
};

/**
 * The keys the server names once whose line changes with a count. The
 * singular reads `<key>One`; the key itself is the plural, since the server
 * names the same key whatever the count. The value is the param that counts.
 */
export const COACH_PLURAL_LINE_PARAM: Readonly<Record<string, string>> = {
  'coachHome.reason.unverifiedFigure': 'count',
};

/**
 * The keys whose line names a number of days the server may not know: with
 * no `days` param the line reads `<key>NoDays`.
 */
export const COACH_OPTIONAL_DAYS_KEYS: ReadonlySet<string> = new Set([
  'coachHome.reason.nothingReceivedReconnect',
  'coachHome.reason.nothingReceivedStale',
]);
