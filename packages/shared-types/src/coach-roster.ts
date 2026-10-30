// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Wire shape of GET /api/coach/roster — the human coach's athletes, one row each, for Coach Home (carnet#1251)
// ABOUTME: Only a `shared` row carries training, plan, plan thread and agent replies; the others carry name, groups and status

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

/** The athlete's live plan, summarised. */
export interface CoachRosterPlan {
  /** The phase today falls in (`base`, `build`, `peak`, …). */
  phase: string | null;
  /** The season week today falls in, from 1. */
  week: number | null;
  weeks_total: number;
  goal_race: { name: string; date: string };
  /** The day after the season's last day, `YYYY-MM-DD`. */
  season_end: string | null;
  /** Coaching platforms the plan's sessions from today on were pushed to. */
  pushed_to: string[];
}

/** The caller's draft and publish history on the athlete's plan. */
export interface CoachRosterPlanThread {
  draft_updated_at: string | null;
  draft_has_conflicts: boolean;
  published_at: string | null;
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
  training?: CoachRosterTraining;
  /** Absent for a sharer with no live plan. */
  plan?: CoachRosterPlan;
  plan_thread?: CoachRosterPlanThread;
  agent_replies?: CoachRosterAgentReplies;
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
