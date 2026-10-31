// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Server-shaped coach roster payloads for the mobile specs — sharers with reasons, calm ones, non-sharers, a race ahead, no plan
// ABOUTME: Dates are offsets from the device's today, because Coach Home reads the device's calendar day

import type {
  CoachAttentionReason,
  CoachRosterAthlete,
  CoachRosterPlan,
  CoachRosterResponse,
  CoachRosterSeason,
  FormBand,
} from '@pierre/shared-types';
import { civilDayNumber, civilFromDayNumber, localToday } from '@pierre/ui-logic';

/** The civil day `offset` days from the device's today. */
export function day(offset: number): string {
  return civilFromDayNumber(civilDayNumber(localToday()) + offset);
}

export function reason(over: Partial<CoachAttentionReason> & { id: string }): CoachAttentionReason {
  return {
    kind: 'nothing_received',
    horizon: 'week',
    rank: 100,
    key: 'coachHome.reason.nothingReceived',
    params: { days: 6 },
    evidence: [],
    action: 'write_to_athlete',
    target: {},
    fingerprint: `fp-${over.id}`,
    dismissed_at: null,
    ...over,
  };
}

interface SharerOptions {
  name: string;
  days?: number | null;
  band?: FormBand;
  ctl?: number | null;
  attention?: CoachAttentionReason[];
  plan?: CoachRosterPlan;
  season?: CoachRosterSeason;
  extra?: Partial<CoachRosterAthlete>;
}

export function sharer(id: string, o: SharerOptions): CoachRosterAthlete {
  return {
    user_id: id,
    name: o.name,
    groups: [{ id: 'g-tri', name: 'Tri' }],
    status: 'shared',
    training: {
      form: { band: o.band ?? 'balanced', as_of: day(0) },
      ctl: o.ctl === undefined ? 70 : o.ctl,
      days_since_last_activity: o.days === undefined ? 1 : o.days,
      last_session: { date: day(-1), sport: 'run', duration_seconds: 3600 },
      last_activity_per_source: { garmin: day(-1) },
      needs_reconnect: [],
      stale: false,
    },
    plan: o.plan,
    season: o.season,
    attention: o.attention ?? [],
    ...o.extra,
  };
}

export function nonSharer(id: string, name: string, extra: Partial<CoachRosterAthlete> = {}): CoachRosterAthlete {
  return {
    user_id: id,
    name,
    groups: [{ id: 'g-tri', name: 'Tri' }],
    status: 'no_coach_consent',
    sharing_stopped_at: `${day(-9)}T08:00:00Z`,
    attention: [],
    ...extra,
  };
}

/** A plan whose build phase has begun and whose taper starts in `taperIn` days, with the A race `raceIn` days away. */
export function planToRace(raceIn: number, taperIn: number): { plan: CoachRosterPlan; season: CoachRosterSeason } {
  const race = { name: 'Ironman Cozumel', date: day(raceIn), priority: 'A' as const };
  return {
    plan: {
      phase: 'build',
      week: 3,
      weeks_total: 12,
      goal_race: race,
      races: [],
      season_end: null,
      ends: day(raceIn + 1),
      phases: [
        { kind: 'build', start: day(-21), end: day(taperIn), weeks: Math.ceil((taperIn + 21) / 7) },
        { kind: 'taper', start: day(taperIn), end: day(raceIn), weeks: Math.max(1, Math.ceil((raceIn - taperIn) / 7)) },
      ],
      pushed_to: ['garmin'],
    },
    season: { state: 'a_race_ahead', race: { name: race.name, date: race.date, a_race: true }, ends: day(raceIn + 1) },
  };
}

const LEA = sharer('lea', {
  name: 'Léa Gagnon',
  days: 1,
  band: 'deep_fatigue',
  attention: [
    reason({
      id: 'deep_fatigue_streak',
      kind: 'deep_fatigue_streak',
      horizon: 'today',
      rank: 40,
      key: 'coachHome.reason.deepFatigueStreak',
      params: { mornings: 3 },
      action: 'write_to_athlete',
      evidence: [
        {
          key: 'coachHome.evidence.formMorning',
          params: { date: day(0), form_pct: -34 },
          source: { kind: 'training_load', providers: ['garmin'] },
          at: `${day(0)}T06:30:00Z`,
        },
      ],
    }),
  ],
  ...planToRace(39, 25),
});

const FELIX = sharer('felix', {
  name: 'Félix Morin',
  days: 0,
  band: 'fresh',
  attention: [
    reason({
      id: 'unverified_figure:r1',
      kind: 'unverified_figure',
      horizon: 'today',
      rank: 20,
      key: 'coachHome.reason.unverifiedFigure',
      params: { count: 1 },
      action: 'correct_reply',
      target: { reply_id: 'r1', group_id: 'g-tri' },
      evidence: [
        {
          key: 'coachHome.evidence.figureUnmatched',
          params: { figure: '285 W', group: 'Tri' },
          source: { kind: 'verification', providers: [] },
          at: `${day(0)}T08:14:00Z`,
        },
      ],
    }),
  ],
  extra: { agent_replies: { count: 3, verified: 2, unmatched_figures: 1, capped: false } },
  ...planToRace(25, 11),
});

const MATHIEU = sharer('mathieu', {
  name: 'Mathieu Roy',
  days: 0,
  band: 'productive',
  attention: [
    reason({
      id: 'plan_draft_waiting',
      kind: 'plan_draft_waiting',
      horizon: 'week',
      rank: 120,
      key: 'coachHome.reason.planDraftWaiting',
      params: {},
      action: 'review_draft',
      target: { group_id: 'g-tri' },
    }),
  ],
});

const JULIEN = sharer('julien', {
  name: 'Julien Côté',
  days: 6,
  band: 'balanced',
  attention: [
    reason({
      id: 'nothing_received',
      kind: 'nothing_received',
      horizon: 'today',
      rank: 30,
      key: 'coachHome.reason.nothingReceivedReconnect',
      params: { days: 6, provider: 'garmin' },
    }),
  ],
  extra: {
    training: {
      form: { band: 'balanced', as_of: day(-6) },
      ctl: 64,
      days_since_last_activity: 6,
      last_session: null,
      last_activity_per_source: { garmin: day(-6) },
      needs_reconnect: ['garmin'],
      stale: false,
    },
  },
  ...planToRace(25, 11),
});

const CAMILLE = sharer('camille', { name: 'Camille Bouchard', days: 2, band: 'insufficient_history', ctl: null });
const OLIVIER = sharer('olivier', {
  name: 'Olivier Fortin',
  days: 3,
  band: 'fresh',
  season: { state: 'plan_ended', race: null, ends: day(-12) },
});
const EMILIE = sharer('emilie', { name: 'Émilie Girard', days: 0, band: 'productive', ...planToRace(165, 120) });
const THOMAS = sharer('thomas', { name: 'Thomas Bélanger', days: 1, band: 'heavy_block', season: { state: 'no_plan', race: null, ends: null } });

const MAUDE = nonSharer('maude', 'Maude Poirier', {
  attention: [
    reason({
      id: 'consent_revoked',
      kind: 'consent_revoked',
      horizon: 'today',
      rank: 10,
      key: 'coachHome.reason.consentRevoked',
      params: {},
      evidence: [
        {
          key: 'coachHome.evidence.sharingStopped',
          params: { at: `${day(-9)}T08:00:00Z` },
          source: { kind: 'consent', providers: [] },
          at: `${day(-9)}T08:00:00Z`,
        },
      ],
    }),
  ],
});
const HUGO = nonSharer('hugo', 'Hugo Lemieux', { status: 'no_source', sharing_stopped_at: undefined });

/** Three groups' worth of states: reasons today and this week, calm sharers, a no-source member and two who stopped sharing. */
export const ROSTER_ATHLETES: CoachRosterAthlete[] = [
  FELIX, MAUDE, JULIEN, LEA, MATHIEU, CAMILLE, OLIVIER, EMILIE, THOMAS, HUGO,
];

export const ROSTER: CoachRosterResponse = {
  scope: { visible_members: 8, roster_members: 10 },
  groups: [
    { id: 'g-tri', name: 'Tri', athlete_count: 10 },
    { id: 'g-route', name: 'Route', athlete_count: 0 },
  ],
  athletes: ROSTER_ATHLETES,
  next_offset: null,
};

export { CAMILLE, EMILIE, FELIX, HUGO, JULIEN, LEA, MATHIEU, MAUDE, OLIVIER, THOMAS };
