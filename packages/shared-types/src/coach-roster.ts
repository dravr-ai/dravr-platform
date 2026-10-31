// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Wire shape of GET /api/coach/roster — the human coach's athletes, one row each, for Coach Home (carnet#1251, #1252, #1250)
// ABOUTME: Only a `shared` row carries training, plan, season and replies; every row carries its ranked attention reasons

import type { FormBand } from './home';

/** An athlete's place on the coach's roster. */
export type CoachRosterStatus = 'shared' | 'no_coach_consent' | 'no_source';

/** A group the coach holds, named on a row or covered by the read. */
export interface CoachRosterGroupRef {
  id: string;
  name: string;
}

/** One group the read covers. */
export interface CoachRosterGroup extends CoachRosterGroupRef {
  /** Live athletes in the group. */
  athlete_count: number;
}

/**
 * Form on the athlete's own scale and the day it was read. A client names the
 * band it is handed: never a number, never compared across athletes.
 */
export interface CoachRosterForm {
  band: FormBand;
  /** The athlete's civil date, `YYYY-MM-DD`. */
  as_of: string;
}

/** The athlete's most recent session. */
export interface CoachRosterLastSession {
  /** `YYYY-MM-DD`, on the athlete's clock. */
  date: string;
  sport: string;
  duration_seconds: number;
}

/** A sharing athlete's training. */
export interface CoachRosterTraining {
  form: CoachRosterForm | null;
  /** Fitness (CTL), rounded; null when not computed. */
  ctl: number | null;
  days_since_last_activity: number | null;
  last_session: CoachRosterLastSession | null;
  /** Source → the last day it delivered a session, `YYYY-MM-DD`. */
  last_activity_per_source: Record<string, string>;
  /** Sources the athlete must reconnect. */
  needs_reconnect: string[];
  /** Served from a cache that could not be refreshed: quiet days prove nothing. */
  stale: boolean;
}

/** A race on a plan. */
export interface CoachRosterRace {
  name: string;
  /** `YYYY-MM-DD`. */
  date: string;
  /** `A` (a goal the season peaks for), `B` or `C`. */
  priority: 'A' | 'B' | 'C';
}

/** The kind of a season phase, as the plan stores it. */
export type CoachRosterPhaseKind =
  | 'prep'
  | 'base'
  | 'build'
  | 'specialty'
  | 'peak'
  | 'taper'
  | 'race'
  | 'transition'
  | 'recovery';

/** One phase of a plan, as the season view draws it. */
export interface CoachRosterPhase {
  kind: CoachRosterPhaseKind;
  /** First day, `YYYY-MM-DD`. */
  start: string;
  /** The day after its last day, `YYYY-MM-DD`; null when its start does not parse. */
  end: string | null;
  weeks: number;
}

/** The athlete's live plan: today's place in it, and its phases and races for the season view. */
export interface CoachRosterPlan {
  /** The phase today falls in (`base`, `build`, `peak`, …). */
  phase: CoachRosterPhaseKind | null;
  /** The season week today falls in, from 1. */
  week: number | null;
  weeks_total: number;
  goal_race: CoachRosterRace;
  /** The rest of the race calendar, in date order; a second A race is a second peak. */
  races: CoachRosterRace[];
  /** The season's last day, `YYYY-MM-DD`; null when the plan ends with its goal race. */
  season_end: string | null;
  /** The day after the plan's last day (last phase, last race or season end), `YYYY-MM-DD`. */
  ends: string | null;
  /** The season's phases in order. */
  phases: CoachRosterPhase[];
  /** Coaching platforms the plan's sessions from today on were pushed to. */
  pushed_to: string[];
}

/**
 * The caller's draft and publish history on the athlete's plan. A row with no
 * `plan` and a `draft_updated_at` is "draft only" in the season view.
 */
export interface CoachRosterPlanThread {
  draft_updated_at: string | null;
  draft_has_conflicts: boolean;
  published_at: string | null;
  /** The open draft's phases (the season view's draft band); empty with none. */
  draft_phases: CoachRosterPhase[];
}

/**
 * Where a sharer's season stands, for the season view's two sections:
 * `a_race_ahead` is "Course A à venir"; every other state is "Sans course A":
 * `no_a_race` (a plan runs, no A race on it ahead), `nothing_since_last_race`
 * (the plan ended with its A race), `plan_ended` (it ran out with no A race at
 * its end), `draft_only`, `new_athlete`, `no_plan`.
 */
export type CoachSeasonState =
  | 'a_race_ahead'
  | 'no_a_race'
  | 'nothing_since_last_race'
  | 'plan_ended'
  | 'draft_only'
  | 'new_athlete'
  | 'no_plan';

/** A race the season view names. */
export interface CoachSeasonRace {
  name: string;
  /** `YYYY-MM-DD`. */
  date: string;
  a_race: boolean;
}

/** A sharer's season. */
export interface CoachRosterSeason {
  state: CoachSeasonState;
  /** The A race ahead (`a_race_ahead`) or the last A race run (`nothing_since_last_race`). */
  race: CoachSeasonRace | null;
  /** The day after the plan's last day, `YYYY-MM-DD`, when there is a plan. */
  ends: string | null;
}

/** What an attention reason is about. */
export type CoachAttentionKind =
  | 'consent_revoked'
  | 'unverified_figure'
  | 'provider_reconnect'
  | 'nothing_received'
  | 'deep_fatigue_streak'
  | 'plan_draft_waiting'
  | 'new_member_no_plan'
  | 'plan_ended';

/** Aujourd'hui or Cette semaine. */
export type CoachAttentionHorizon = 'today' | 'week';

/** The one primary action a reason names; the client labels it `coachHome.action.<camelCase>`. */
export type CoachAttentionAction =
  | 'write_to_athlete'
  | 'correct_reply'
  | 'open_plan_thread'
  | 'review_draft'
  | 'create_plan';

/** Where an evidence line comes from. */
export type CoachEvidenceSourceKind =
  | 'provider'
  | 'training_load'
  | 'connection'
  | 'cache'
  | 'consent'
  | 'verification'
  | 'plan'
  | 'plan_thread'
  | 'membership';

/** The source of one evidence line. */
export interface CoachEvidenceSource {
  kind: CoachEvidenceSourceKind;
  /** The sources the line reads, as the athlete names them (`garmin`, `strava`); empty for none. */
  providers: string[];
}

/** One line of evidence behind a reason. */
export interface CoachAttentionEvidence {
  /** An i18n key under `coachHome.evidence.*`. */
  key: string;
  /** The values the string interpolates: dates `YYYY-MM-DD`, instants ISO 8601, numbers. */
  params: Record<string, string | number>;
  source: CoachEvidenceSource;
  /** When the source said it (ISO 8601); null only for a revoke whose instant was never recorded. */
  at: string | null;
}

/** What a reason's action acts on, when it names one thing. */
export interface CoachAttentionTarget {
  /** The agent reply to correct. */
  reply_id?: string;
  /** The group it was sent into, or the group a new member joined. */
  group_id?: string;
  /** The source to reconnect. */
  provider?: string;
}

/**
 * One reason to look at an athlete, ranked and evidenced by the server: a
 * client sorts on `rank` (lower first, comparable across pages) and never
 * derives a band or a threshold.
 */
export interface CoachAttentionReason {
  /** Stable while the situation lasts (`nothing_received`, `provider_reconnect:garmin`, `unverified_figure:<reply id>`). */
  id: string;
  kind: CoachAttentionKind;
  horizon: CoachAttentionHorizon;
  rank: number;
  /** An i18n key under `coachHome.reason.*`: the reason's one line. */
  key: string;
  params: Record<string, string | number>;
  evidence: CoachAttentionEvidence[];
  action: CoachAttentionAction;
  target: CoachAttentionTarget;
  /** The signal it was raised on: what a dismissal sends back. */
  fingerprint: string;
  /** When the caller marked it handled for this very signal (ISO 8601); null while it awaits them. */
  dismissed_at: string | null;
}

/** The agent's replies to the athlete in the coach's groups since Monday (UTC). */
export interface CoachRosterAgentReplies {
  count: number;
  /** Replies with no contradicted or unsupported claim and no unmatched figure. */
  verified: number;
  /** Figures across those replies with no source in the athlete's data. */
  unmatched_figures: number;
  /** The week held more replies than one read covers: the counts are a floor. */
  capped: boolean;
}

/** One athlete on the roster. */
export interface CoachRosterAthlete {
  user_id: string;
  name: string;
  groups: CoachRosterGroupRef[];
  status: CoachRosterStatus;
  /**
   * When the athlete stopped sharing with the coach (ISO 8601). Only for
   * `no_coach_consent`, and absent when the date is unknown.
   */
  sharing_stopped_at?: string;
  training?: CoachRosterTraining;
  /** Absent for a sharer with no live plan. */
  plan?: CoachRosterPlan;
  plan_thread?: CoachRosterPlanThread;
  agent_replies?: CoachRosterAgentReplies;
  /** Only for `shared` when the plan was read. */
  season?: CoachRosterSeason;
  /**
   * The reasons to look at the athlete today or this week, lowest `rank`
   * first; empty for a calm athlete. A non-sharer carries `consent_revoked`
   * alone.
   */
  attention: CoachAttentionReason[];
}

/** `GET /api/coach/roster`. */
export interface CoachRosterResponse {
  /** Every aggregate states its scope: how many athletes share, of how many held. */
  scope: { visible_members: number; roster_members: number };
  groups: CoachRosterGroup[];
  /** Sharers first, longest without a session at the top; then the others by name. */
  athletes: CoachRosterAthlete[];
  /** The `offset` of the next page; null on the last. */
  next_offset: number | null;
}

/** Query of `GET /api/coach/roster`. */
export interface CoachRosterQuery {
  offset?: number;
  /** 1–100; 50 when absent. */
  limit?: number;
}

/** Body of `POST /api/coach/attention/dismiss` ("Traité"). */
export interface CoachAttentionDismissRequest {
  athlete_user_id: string;
  /** The reason's `id`. */
  reason_id: string;
  /** The reason's `fingerprint`: the dismissal holds while it is unchanged. */
  fingerprint: string;
}

/** `POST /api/coach/attention/dismiss`. */
export interface CoachAttentionDismissResponse {
  reason_id: string;
  fingerprint: string;
  dismissed_at: string;
}

/** Body of `POST /api/coach/attention/restore` (undo "Traité"). */
export interface CoachAttentionRestoreRequest {
  athlete_user_id: string;
  reason_id: string;
}

/** `POST /api/coach/attention/restore`. */
export interface CoachAttentionRestoreResponse {
  reason_id: string;
  /** Whether it had been dismissed. */
  restored: boolean;
}
