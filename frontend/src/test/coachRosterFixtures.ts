// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A roster for Coach Home's unit tests, mocked e2e and design sweep — every reason, season state and non-sharer, dated from a given today
// ABOUTME: Mirrors GET /api/coach/roster (CoachRosterResponse); a builder, so the dates stay relative to the day the test runs

import type {
  CoachAttentionReason,
  CoachRosterAthlete,
  CoachRosterPhase,
  CoachRosterPlan,
  CoachRosterResponse,
  CoachRosterTraining,
  FormBand,
} from '@pierre/shared-types';
import { civilDayNumber, civilFromDayNumber } from '@pierre/ui-logic';

const TRI = { id: 'g-tri', name: 'Tri — longue distance' };
const ROAD = { id: 'g-road', name: 'Route — Gran Fondo' };
const RUN = { id: 'g-run', name: 'Course — marathon' };

/** `today` shifted by `days`, as `YYYY-MM-DD`. */
export function dayOffset(today: string, days: number): string {
  return civilFromDayNumber(civilDayNumber(today) + days);
}

function at(today: string, days: number, time = '06:30:00'): string {
  return `${dayOffset(today, days)}T${time}Z`;
}

function training(
  today: string,
  band: FormBand,
  daysSince: number | null,
  extra: Partial<CoachRosterTraining> = {},
): CoachRosterTraining {
  return {
    form: { band, as_of: dayOffset(today, 0) },
    ctl: 70,
    days_since_last_activity: daysSince,
    last_session: daysSince === null ? null : { date: dayOffset(today, -daysSince), sport: 'cycling', duration_seconds: 5400 },
    last_activity_per_source: daysSince === null ? {} : { garmin: dayOffset(today, -daysSince) },
    needs_reconnect: [],
    stale: false,
    ...extra,
  };
}

function phase(today: string, kind: CoachRosterPhase['kind'], from: number, to: number): CoachRosterPhase {
  return { kind, start: dayOffset(today, from), end: dayOffset(today, to), weeks: Math.max(1, Math.round((to - from) / 7)) };
}

function plan(today: string, phases: CoachRosterPhase[], raceName: string, raceIn: number, current: CoachRosterPhase['kind']): CoachRosterPlan {
  return {
    phase: current,
    week: 3,
    weeks_total: 12,
    goal_race: { name: raceName, date: dayOffset(today, raceIn), priority: 'A' },
    races: [],
    season_end: null,
    ends: phases[phases.length - 1].end,
    phases,
    pushed_to: ['trainingpeaks'],
  };
}

function reason(over: Partial<CoachAttentionReason> & Pick<CoachAttentionReason, 'id' | 'kind' | 'key'>): CoachAttentionReason {
  return {
    horizon: 'today',
    rank: 100,
    params: {},
    evidence: [],
    action: 'write_to_athlete',
    target: {},
    fingerprint: `fp:${over.id}`,
    dismissed_at: null,
    ...over,
  };
}

/** The named athletes: one for each reason, season state and kind of non-sharer. */
export function namedAthletes(today: string): CoachRosterAthlete[] {
  return [
    {
      user_id: 'u-lea',
      name: 'Léa Gagnon',
      groups: [TRI],
      status: 'shared',
      training: training(today, 'deep_fatigue', 1, { ctl: 92 }),
      plan: plan(today, [phase(today, 'build', -40, -14), phase(today, 'specialty', -14, 25), phase(today, 'taper', 25, 39)], 'IM 70.3 Indian Wells', 39, 'specialty'),
      plan_thread: { draft_updated_at: null, draft_has_conflicts: false, published_at: at(today, -18), draft_phases: [] },
      agent_replies: { count: 2, verified: 2, unmatched_figures: 0, capped: false },
      season: { state: 'a_race_ahead', race: { name: 'IM 70.3 Indian Wells', date: dayOffset(today, 39), a_race: true }, ends: dayOffset(today, 53) },
      attention: [
        reason({
          id: 'deep_fatigue_streak',
          kind: 'deep_fatigue_streak',
          key: 'coachHome.reason.deepFatigueStreak',
          params: { mornings: 3 },
          rank: 40,
          action: 'open_plan_thread',
          target: { group_id: TRI.id },
          evidence: [
            {
              key: 'coachHome.evidence.formMorning',
              params: { date: dayOffset(today, 0), form_pct: -34 },
              source: { kind: 'training_load', providers: ['garmin'] },
              at: at(today, 0),
            },
            {
              key: 'coachHome.evidence.lastSession',
              params: { date: dayOffset(today, -1), sport: 'cycling', duration_seconds: 11100 },
              source: { kind: 'provider', providers: ['garmin'] },
              at: at(today, 0, '06:12:00'),
            },
          ],
        }),
      ],
    },
    {
      user_id: 'u-julien',
      name: 'Julien Côté',
      groups: [TRI],
      status: 'shared',
      training: training(today, 'balanced', 6, {
        form: { band: 'balanced', as_of: dayOffset(today, -8) },
        needs_reconnect: ['garmin'],
        last_activity_per_source: { garmin: dayOffset(today, -8), trainingpeaks: dayOffset(today, -6) },
      }),
      plan: plan(today, [phase(today, 'specialty', -14, 11), phase(today, 'taper', 11, 25)], 'Ironman Cozumel', 25, 'specialty'),
      plan_thread: { draft_updated_at: null, draft_has_conflicts: false, published_at: at(today, -27), draft_phases: [] },
      agent_replies: { count: 0, verified: 0, unmatched_figures: 0, capped: false },
      season: { state: 'a_race_ahead', race: { name: 'Ironman Cozumel', date: dayOffset(today, 25), a_race: true }, ends: dayOffset(today, 39) },
      attention: [
        reason({
          id: 'nothing_received',
          kind: 'nothing_received',
          key: 'coachHome.reason.nothingReceivedReconnect',
          params: { days: 6, provider: 'garmin' },
          rank: 20,
          target: { provider: 'garmin', group_id: TRI.id },
          evidence: [
            {
              key: 'coachHome.evidence.sourceRefusedSince',
              params: { provider: 'garmin', date: dayOffset(today, -8) },
              source: { kind: 'connection', providers: ['garmin'] },
              at: at(today, -8),
            },
            {
              key: 'coachHome.evidence.lastReceived',
              params: { provider: 'garmin', date: dayOffset(today, -8) },
              source: { kind: 'provider', providers: ['garmin'] },
              at: at(today, -8),
            },
          ],
        }),
      ],
    },
    {
      user_id: 'u-felix',
      name: 'Félix Morin',
      groups: [TRI],
      status: 'shared',
      training: training(today, 'fresh', 0, { ctl: 77 }),
      plan: plan(today, [phase(today, 'specialty', -14, 11), phase(today, 'taper', 11, 25)], 'Ironman Cozumel', 25, 'specialty'),
      plan_thread: { draft_updated_at: null, draft_has_conflicts: false, published_at: at(today, -27), draft_phases: [] },
      agent_replies: { count: 1, verified: 0, unmatched_figures: 1, capped: false },
      season: { state: 'a_race_ahead', race: { name: 'Ironman Cozumel', date: dayOffset(today, 25), a_race: true }, ends: dayOffset(today, 39) },
      attention: [
        reason({
          id: 'unverified_figure:r-1',
          kind: 'unverified_figure',
          key: 'coachHome.reason.unverifiedFigure',
          params: { count: 1 },
          rank: 30,
          action: 'correct_reply',
          target: { reply_id: 'r-1', group_id: TRI.id },
          evidence: [
            {
              key: 'coachHome.evidence.figureUnmatched',
              params: { figure: '285 W', group: TRI.name },
              source: { kind: 'verification', providers: [] },
              at: at(today, 0, '08:14:00'),
            },
          ],
        }),
      ],
    },
    {
      user_id: 'u-maude',
      name: 'Maude Poirier',
      groups: [RUN],
      status: 'no_coach_consent',
      sharing_stopped_at: at(today, -1, '21:48:00'),
      attention: [
        reason({
          id: 'consent_revoked',
          kind: 'consent_revoked',
          key: 'coachHome.reason.consentRevoked',
          rank: 10,
          target: { group_id: RUN.id },
          evidence: [
            {
              key: 'coachHome.evidence.sharingStopped',
              params: { at: at(today, -1, '21:48:00') },
              source: { kind: 'consent', providers: [] },
              at: at(today, -1, '21:48:00'),
            },
          ],
        }),
      ],
    },
    {
      user_id: 'u-hugo',
      name: 'Hugo Lemieux',
      groups: [TRI],
      status: 'no_coach_consent',
      attention: [],
    },
    {
      user_id: 'u-mathieu',
      name: 'Mathieu Roy',
      groups: [ROAD],
      status: 'shared',
      training: training(today, 'productive', 0, { ctl: 74 }),
      plan_thread: {
        draft_updated_at: at(today, -1, '21:10:00'),
        draft_has_conflicts: false,
        published_at: null,
        draft_phases: [phase(today, 'base', 3, 31), phase(today, 'build', 31, 59)],
      },
      agent_replies: { count: 1, verified: 1, unmatched_figures: 0, capped: false },
      season: { state: 'draft_only', race: null, ends: null },
      attention: [
        reason({
          id: 'plan_draft_waiting',
          kind: 'plan_draft_waiting',
          key: 'coachHome.reason.planDraftWaiting',
          horizon: 'week',
          rank: 300,
          action: 'review_draft',
          target: { group_id: ROAD.id },
          evidence: [
            {
              key: 'coachHome.evidence.draftSaved',
              params: { at: at(today, -1, '21:10:00') },
              source: { kind: 'plan_thread', providers: [] },
              at: at(today, -1, '21:10:00'),
            },
          ],
        }),
      ],
    },
    {
      user_id: 'u-camille',
      name: 'Camille Bouchard',
      groups: [RUN],
      status: 'shared',
      training: training(today, 'insufficient_history', 2, { ctl: null }),
      agent_replies: { count: 0, verified: 0, unmatched_figures: 0, capped: false },
      season: { state: 'new_athlete', race: null, ends: null },
      attention: [
        reason({
          id: 'new_member_no_plan',
          kind: 'new_member_no_plan',
          key: 'coachHome.reason.newMemberNoPlan',
          params: { date: dayOffset(today, -5) },
          horizon: 'week',
          rank: 310,
          action: 'create_plan',
          target: { group_id: RUN.id },
          evidence: [
            {
              key: 'coachHome.evidence.joinedGroup',
              params: { group: RUN.name, date: dayOffset(today, -5) },
              source: { kind: 'membership', providers: [] },
              at: at(today, -5),
            },
            {
              key: 'coachHome.evidence.historyTooShort',
              params: {},
              source: { kind: 'training_load', providers: ['strava'] },
              at: at(today, 0),
            },
          ],
        }),
      ],
    },
    {
      user_id: 'u-olivier',
      name: 'Olivier Fortin',
      groups: [ROAD],
      status: 'shared',
      training: training(today, 'detraining', 3, { ctl: 41 }),
      season: { state: 'nothing_since_last_race', race: { name: 'Gran Fondo Mont-Tremblant', date: dayOffset(today, -12), a_race: true }, ends: dayOffset(today, -12) },
      attention: [
        reason({
          id: 'plan_ended',
          kind: 'plan_ended',
          key: 'coachHome.reason.planEnded',
          params: { date: dayOffset(today, -12) },
          horizon: 'week',
          rank: 320,
          action: 'open_plan_thread',
          target: { group_id: ROAD.id },
          evidence: [
            {
              key: 'coachHome.evidence.planEndedAfterRace',
              params: { date: dayOffset(today, -12), race: 'Gran Fondo Mont-Tremblant' },
              source: { kind: 'plan', providers: [] },
              at: null,
            },
          ],
        }),
      ],
    },
    {
      user_id: 'u-kevin',
      name: 'Kevin Ouellet',
      groups: [ROAD],
      status: 'shared',
      training: training(today, 'productive', 0, { ctl: 63 }),
      plan: plan(today, [phase(today, 'base', -10, 18)], 'Pas de course', 0, 'base'),
      agent_replies: { count: 0, verified: 0, unmatched_figures: 0, capped: false },
      season: { state: 'no_a_race', race: null, ends: dayOffset(today, 18) },
      attention: [],
    },
    {
      user_id: 'u-rosalie',
      name: 'Rosalie Dubé',
      groups: [ROAD],
      status: 'shared',
      training: training(today, 'balanced', 1, { ctl: 55 }),
      season: { state: 'no_plan', race: null, ends: null },
      attention: [],
    },
  ];
}

/** Extra sharers with nothing to flag, so a list or a table is tall enough to overflow. */
export function fillerAthletes(today: string, count: number): CoachRosterAthlete[] {
  const bands: FormBand[] = ['deep_fatigue', 'heavy_block', 'productive', 'balanced', 'fresh', 'detraining'];
  return Array.from({ length: count }, (_, i) => ({
    user_id: `u-filler-${i}`,
    name: `Athlète ${String(i + 1).padStart(2, '0')}`,
    groups: [i % 2 === 0 ? TRI : ROAD],
    status: 'shared' as const,
    training: training(today, bands[i % bands.length], i % 4, { ctl: 40 + i }),
    plan: plan(today, [phase(today, 'base', -20, 30 + i * 3), phase(today, 'build', 30 + i * 3, 90 + i * 3)], `Course ${i + 1}`, 90 + i * 3, 'base'),
    plan_thread: { draft_updated_at: null, draft_has_conflicts: false, published_at: at(today, -10), draft_phases: [] },
    agent_replies: { count: 1, verified: 1, unmatched_figures: 0, capped: false },
    season: { state: 'a_race_ahead' as const, race: { name: `Course ${i + 1}`, date: dayOffset(today, 90 + i * 3), a_race: true }, ends: dayOffset(today, 120 + i * 3) },
    attention: [],
  }));
}

/** The response of `GET /api/coach/roster` for the named athletes and `fillers` more calm ones. */
export function buildRoster(today: string, fillers = 0): CoachRosterResponse {
  const athletes = [...namedAthletes(today), ...fillerAthletes(today, fillers)];
  const sharing = athletes.filter((a) => a.status === 'shared').length;
  const count = (id: string) => athletes.filter((a) => a.groups.some((g) => g.id === id)).length;
  return {
    scope: { visible_members: sharing, roster_members: athletes.length },
    groups: [TRI, ROAD, RUN].map((g) => ({ ...g, athlete_count: count(g.id) })),
    athletes,
    next_offset: null,
  };
}
