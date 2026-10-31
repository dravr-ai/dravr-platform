// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests Coach Home's rules — the queue, the team table's order, the season layout and the words of an evidence line
// ABOUTME: Run on a key-echoing translator so a catalogue edit cannot hide a rule that changed

import { describe, it, expect } from 'vitest';
import type {
  CoachAttentionReason,
  CoachRosterAthlete,
  CoachRosterResponse,
  CoachSeasonState,
} from '@pierre/shared-types';
import {
  COACH_ACTION_LABEL_KEY,
  COACH_AGENT_ALL_VERIFIED_KEYS,
  COACH_AGENT_COUNT_KEYS,
  COACH_DAY_UNIT_KEYS,
  COACH_EVIDENCE_SOURCE_LABEL_KEY,
  COACH_MORNING_UNIT_KEYS,
  COACH_OPTIONAL_DAYS_KEYS,
  COACH_PHASE_LABEL_KEY,
  COACH_PLURAL_LINE_PARAM,
  COACH_SCOPE_KEYS,
  COACH_SEASON_STATE_KEY,
  COACH_UNVERIFIED_KEYS,
} from '@pierre/shared-constants';
import en from '../../i18n/src/locales/en/translation.json';
import fr from '../../i18n/src/locales/fr/translation.json';
import de from '../../i18n/src/locales/de/translation.json';
import es from '../../i18n/src/locales/es/translation.json';
import pt from '../../i18n/src/locales/pt/translation.json';
import {
  agentCell,
  buildQueue,
  buildRosterTable,
  buildSeason,
  civilDayNumber,
  countdownBars,
  daysBetween,
  describeLine,
  describeSource,
  formStripIndex,
  lastActivity,
  lineKey,
  mergeRosterPages,
  nextPhase,
  providerName,
  queueCount,
  sourceChips,
  threadState,
} from '../src/coachHome';

const echo = (key: string, options?: Record<string, unknown>) =>
  options === undefined ? key : `${key}${JSON.stringify(options)}`;

function reason(over: Partial<CoachAttentionReason> & { id: string }): CoachAttentionReason {
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

function athlete(over: Partial<CoachRosterAthlete> & { user_id: string }): CoachRosterAthlete {
  return {
    name: over.user_id,
    groups: [{ id: 'g1', name: 'Tri' }],
    status: 'shared',
    attention: [],
    ...over,
  };
}

function sharer(
  id: string,
  days: number | null,
  band: 'deep_fatigue' | 'fresh' = 'fresh',
  extra: Partial<CoachRosterAthlete> = {},
): CoachRosterAthlete {
  return athlete({
    user_id: id,
    training: {
      form: { band, as_of: '2026-10-30' },
      ctl: 70,
      days_since_last_activity: days,
      last_session: null,
      last_activity_per_source: {},
      needs_reconnect: [],
      stale: false,
    },
    ...extra,
  });
}

describe('buildQueue', () => {
  it('orders each horizon by the server rank, and an athlete stands once under the lowest rank', () => {
    const queue = buildQueue([
      athlete({ user_id: 'b', attention: [reason({ id: 'x', horizon: 'today', rank: 20 })] }),
      athlete({
        user_id: 'a',
        attention: [reason({ id: 'y', horizon: 'week', rank: 300 }), reason({ id: 'z', horizon: 'today', rank: 10 })],
      }),
      athlete({ user_id: 'c', attention: [reason({ id: 'w', horizon: 'week', rank: 200 })] }),
    ]);
    expect(queue.today.map((e) => e.athlete.user_id)).toEqual(['a', 'b']);
    expect(queue.today[0].reason.id).toBe('z');
    expect(queue.today[0].others.map((r) => r.id)).toEqual(['y']);
    expect(queue.week.map((e) => e.athlete.user_id)).toEqual(['c']);
    expect(queueCount(queue)).toBe(3);
  });

  it('moves an athlete whose reasons are all dismissed to treated, and brings them back when one reopens', () => {
    const done = athlete({
      user_id: 'a',
      attention: [reason({ id: 'x', dismissed_at: '2026-10-30T10:00:00Z' })],
    });
    expect(buildQueue([done]).treated.map((e) => e.athlete.user_id)).toEqual(['a']);
    expect(queueCount(buildQueue([done]))).toBe(0);
    const reopened = {
      ...done,
      attention: [...done.attention, reason({ id: 'y', rank: 5, horizon: 'today' })],
    };
    const queue = buildQueue([reopened]);
    expect(queue.today[0].reason.id).toBe('y');
    expect(queue.treated).toEqual([]);
  });

  it('keeps calm sharers and reasonless non-sharers out of the queue, each in its own list', () => {
    const queue = buildQueue([
      athlete({ user_id: 'kevin' }),
      athlete({ user_id: 'hugo', status: 'no_coach_consent' }),
      athlete({
        user_id: 'maude',
        status: 'no_coach_consent',
        attention: [reason({ id: 'consent_revoked', horizon: 'today', rank: 1, kind: 'consent_revoked' })],
      }),
    ]);
    expect(queue.calm.map((a) => a.user_id)).toEqual(['kevin']);
    expect(queue.notSharing.map((a) => a.user_id)).toEqual(['hugo']);
    expect(queue.today.map((e) => e.athlete.user_id)).toEqual(['maude']);
  });
});

describe('buildRosterTable', () => {
  it('sorts sharers by days without activity, the longest first and a missing history first of all', () => {
    const table = buildRosterTable([
      sharer('recent', 0),
      sharer('quiet', 6),
      sharer('unknown', null),
      sharer('yesterday', 1),
    ]);
    expect(table.sharers.map((a) => a.user_id)).toEqual(['unknown', 'quiet', 'yesterday', 'recent']);
  });

  it('never sorts by form: equal silence falls to the name whatever the bands', () => {
    const table = buildRosterTable([
      sharer('zed', 2, 'fresh', { name: 'Zed' }),
      sharer('abe', 2, 'deep_fatigue', { name: 'Abe' }),
    ]);
    expect(table.sharers.map((a) => a.name)).toEqual(['Abe', 'Zed']);
  });

  it('puts non-sharers in a block apart by name, and filters both blocks by search', () => {
    const table = buildRosterTable([
      sharer('a', 1, 'fresh', { name: 'Alice' }),
      athlete({ user_id: 'h', name: 'Hugo', status: 'no_coach_consent' }),
      athlete({ user_id: 'm', name: 'Maude', status: 'no_coach_consent' }),
    ]);
    expect(table.nonSharers.map((a) => a.name)).toEqual(['Hugo', 'Maude']);
    expect(buildRosterTable([sharer('a', 1, 'fresh', { name: 'Alice' }), athlete({ user_id: 'h', name: 'Hugo', status: 'no_coach_consent' })], 'hu')).toEqual({
      sharers: [],
      nonSharers: [expect.objectContaining({ name: 'Hugo' })],
    });
  });
});

describe('cells', () => {
  it('reads last activity as today, yesterday, n days or never', () => {
    expect(lastActivity(0)).toEqual({ kind: 'today' });
    expect(lastActivity(1)).toEqual({ kind: 'yesterday' });
    expect(lastActivity(6)).toEqual({ kind: 'daysAgo', days: 6 });
    expect(lastActivity(null)).toEqual({ kind: 'never' });
  });

  it('lists the sources that delivered and the ones to reconnect', () => {
    const a = sharer('a', 1);
    a.training = {
      ...a.training!,
      last_activity_per_source: { garmin: '2026-10-22', strava: '2026-10-29' },
      needs_reconnect: ['garmin'],
    };
    expect(sourceChips(a)).toEqual([
      { provider: 'garmin', state: 'reconnect', lastDay: '2026-10-22' },
      { provider: 'strava', state: 'ok', lastDay: '2026-10-29' },
    ]);
  });

  it('says the thread state: draft first, then ended, published, create', () => {
    const thread = { draft_updated_at: null, draft_has_conflicts: false, published_at: '2026-10-03T10:00:00Z', draft_phases: [] };
    expect(threadState(sharer('a', 1, 'fresh', { plan_thread: thread }))).toEqual({ kind: 'published', date: '2026-10-03' });
    expect(threadState(sharer('a', 1, 'fresh', { plan_thread: { ...thread, draft_updated_at: '2026-10-29T21:10:00Z' } }))).toEqual({ kind: 'draft' });
    expect(threadState(sharer('a', 1))).toEqual({ kind: 'create' });
  });

  it('counts unmatched figures before anything else in the agent cell', () => {
    const replies = { count: 3, verified: 2, unmatched_figures: 1, capped: false };
    expect(agentCell(sharer('a', 1, 'fresh', { agent_replies: replies }))).toEqual({ kind: 'unverified', count: 1 });
    expect(agentCell(sharer('a', 1, 'fresh', { agent_replies: { ...replies, unmatched_figures: 0, verified: 3 } }))).toEqual({ kind: 'allVerified', count: 3 });
    expect(agentCell(sharer('a', 1, 'fresh', { agent_replies: { ...replies, count: 0 } }))).toEqual({ kind: 'none' });
  });

  it('lights one of six steps, and none for insufficient history', () => {
    expect(formStripIndex('deep_fatigue')).toBe(0);
    expect(formStripIndex('detraining')).toBe(5);
    expect(formStripIndex('insufficient_history')).toBe(-1);
  });
});

describe('the words of a line', () => {
  it('reads the plural of a counted key and the twin of a missing day count', () => {
    expect(lineKey('coachHome.reason.unverifiedFigure', { count: 1 }, 'fr')).toBe('coachHome.reason.unverifiedFigureOne');
    expect(lineKey('coachHome.reason.unverifiedFigure', { count: 2 }, 'fr')).toBe('coachHome.reason.unverifiedFigure');
    expect(lineKey('coachHome.reason.nothingReceivedStale', {}, 'fr')).toBe('coachHome.reason.nothingReceivedStaleNoDays');
    expect(lineKey('coachHome.reason.nothingReceivedStale', { days: 8 }, 'fr')).toBe('coachHome.reason.nothingReceivedStale');
    expect(lineKey('coachHome.reason.planDraftWaiting', {}, 'fr')).toBe('coachHome.reason.planDraftWaiting');
  });

  it('turns counts into phrases, dates into the locale and a provider into its brand', () => {
    const line = describeLine(echo, 'en', 'coachHome.reason.nothingReceivedReconnect', { days: 6, provider: 'garmin' });
    expect(line).toContain('coachHome.reason.nothingReceivedReconnect');
    expect(line).toContain('"provider":"Garmin"');
    expect(line).toContain('coachHome.unit.dayN{\\"count\\":6}');
    expect(describeLine(echo, 'en', 'coachHome.evidence.formMorning', { date: '2026-10-30', form_pct: -34 })).toContain('"form_pct":"-34"');
  });

  it('names an evidence source with its providers and its time', () => {
    const line = describeSource(echo, 'en', { kind: 'training_load', providers: ['garmin'] }, null);
    expect(line).toBe('Garmin · coachHome.source.training_load');
    expect(describeSource(echo, 'en', { kind: 'provider', providers: [] }, null)).toBe('coachHome.source.provider');
  });

  it('names providers', () => {
    expect(providerName('trainingpeaks')).toBe('TrainingPeaks');
    expect(providerName('zwift')).toBe('Zwift');
  });
});

describe('dates', () => {
  it('counts whole days between two civil days', () => {
    expect(daysBetween('2026-10-30', '2026-12-06')).toBe(37);
    expect(daysBetween('2026-10-30', 'nope')).toBeNull();
    expect(civilDayNumber('1970-01-02')).toBe(1);
  });
});

describe('mergeRosterPages', () => {
  it('reads every page once and keeps the first page scope', () => {
    const page = (offset: number, ids: string[], next: number | null): CoachRosterResponse => ({
      scope: { visible_members: 3, roster_members: 4 },
      groups: [{ id: 'g1', name: 'Tri', athlete_count: 4 }],
      athletes: ids.map((id) => athlete({ user_id: id })),
      next_offset: next,
    });
    const merged = mergeRosterPages([page(0, ['a', 'b'], 2), page(2, ['b', 'c'], null)]);
    expect(merged.athletes.map((a) => a.user_id)).toEqual(['a', 'b', 'c']);
    expect(merged.scope).toEqual({ visible_members: 3, roster_members: 4 });
  });
});

describe('buildSeason', () => {
  const TODAY = '2026-10-30';

  function seasoned(
    id: string,
    state: CoachSeasonState,
    raceDate: string | null,
    phases: { kind: 'base' | 'build' | 'specialty' | 'taper'; start: string; end: string }[] = [],
    extra: Partial<CoachRosterAthlete> = {},
  ): CoachRosterAthlete {
    return sharer(id, 1, 'fresh', {
      name: id,
      season: {
        state,
        race: raceDate === null ? null : { name: 'Race', date: raceDate, a_race: true },
        ends: null,
      },
      plan:
        phases.length === 0
          ? undefined
          : {
              phase: phases[0].kind,
              week: 1,
              weeks_total: 8,
              goal_race: { name: 'Race', date: raceDate ?? phases[0].end, priority: 'A' },
              races: [],
              season_end: null,
              ends: phases[phases.length - 1].end,
              phases: phases.map((p) => ({ ...p, weeks: 4 })),
              pushed_to: [],
            },
      ...extra,
    });
  }

  it('sorts the race-ahead section by race date, then names the others by state', () => {
    const view = buildSeason(
      [
        seasoned('late', 'a_race_ahead', '2027-03-01', [{ kind: 'base', start: '2026-10-01', end: '2027-03-01' }]),
        seasoned('soon', 'a_race_ahead', '2026-11-22', [{ kind: 'specialty', start: '2026-10-01', end: '2026-11-22' }]),
        seasoned('nothing', 'no_plan', null),
        seasoned('draft', 'draft_only', null),
        athlete({ user_id: 'hugo', name: 'Hugo', status: 'no_coach_consent' }),
      ],
      TODAY,
      'season',
    );
    expect(view.ahead.map((r) => r.athlete.name)).toEqual(['soon', 'late']);
    expect(view.without.map((r) => r.athlete.name)).toEqual(['draft', 'nothing']);
    expect(view.hidden.map((a) => a.name)).toEqual(['Hugo']);
    expect(view.ahead[0].daysToRace).toBe(23);
  });

  it('spans whole months on the season scale and twelve weeks from Monday on the short one', () => {
    const athletes = [seasoned('a', 'a_race_ahead', '2027-03-15', [{ kind: 'base', start: '2026-10-05', end: '2027-03-15' }])];
    const season = buildSeason(athletes, TODAY, 'season').axis;
    expect(season.start).toBe('2026-10-01');
    expect(season.months[0]).toEqual({ at: 0, date: '2026-10-01' });
    expect(season.months.at(-1)?.date).toBe('2027-03-01');
    expect(season.todayAt).toBe(29);
    const short = buildSeason(athletes, TODAY, '12w').axis;
    expect(short.start).toBe('2026-10-26');
    expect(short.span).toBe(84);
    expect(short.todayAt).toBe(4);
  });

  it('draws a draft as one dashed band and a run race hollow, and names the day data stopped', () => {
    const draft = seasoned('d', 'draft_only', null, [], {
      plan_thread: {
        draft_updated_at: '2026-10-29T21:10:00Z',
        draft_has_conflicts: false,
        published_at: null,
        draft_phases: [
          { kind: 'base', start: '2026-11-02', end: '2026-11-30', weeks: 4 },
          { kind: 'build', start: '2026-11-30', end: '2026-12-28', weeks: 4 },
        ],
      },
    });
    const ran = seasoned('r', 'nothing_since_last_race', '2026-10-12');
    ran.training = { ...ran.training!, stale: true, last_activity_per_source: { garmin: '2026-10-22' } };
    const view = buildSeason([draft, ran], TODAY, 'season');
    const d = view.without.find((r) => r.athlete.name === 'd')!;
    expect(d.bars).toHaveLength(1);
    expect(d.bars[0].kind).toBe('draft');
    const r = view.without.find((row) => row.athlete.name === 'r')!;
    expect(r.racePast).toBe(true);
    expect(r.noDataSince).toBe('2026-10-22');
  });

  it('lists the phases from today to the race for the phone, and the next phase to come', () => {
    const a = seasoned('a', 'a_race_ahead', '2026-12-06', [
      { kind: 'specialty', start: '2026-10-05', end: '2026-11-23' },
      { kind: 'taper', start: '2026-11-23', end: '2026-12-06' },
    ]);
    expect(countdownBars(a, TODAY)).toEqual([
      { kind: 'specialty', days: 24 },
      { kind: 'taper', days: 13 },
    ]);
    expect(nextPhase(a, TODAY)).toEqual({ kind: 'taper', inDays: 24 });
  });
});

describe('describeSource', () => {
  it('prints the time of a source when it has one', () => {
    const line = describeSource(echo, 'en', { kind: 'verification', providers: [] }, '2026-10-30T08:14:00Z');
    expect(line.startsWith('coachHome.source.verification · ')).toBe(true);
  });
});

describe('the catalogue behind the tables', () => {
  const CATALOGUES = { en, fr, de, es, pt } as const;
  const lookup = (catalogue: unknown, key: string): unknown =>
    key.split('.').reduce<unknown>((node, part) => (node !== null && typeof node === 'object' ? (node as Record<string, unknown>)[part] : undefined), catalogue);

  const tableKeys = [
    ...Object.values(COACH_ACTION_LABEL_KEY),
    ...Object.values(COACH_EVIDENCE_SOURCE_LABEL_KEY),
    ...Object.values(COACH_PHASE_LABEL_KEY),
    ...Object.values(COACH_SEASON_STATE_KEY),
    ...[COACH_SCOPE_KEYS, COACH_DAY_UNIT_KEYS, COACH_MORNING_UNIT_KEYS, COACH_AGENT_COUNT_KEYS, COACH_AGENT_ALL_VERIFIED_KEYS, COACH_UNVERIFIED_KEYS].flatMap(
      (keys) => [keys.one, keys.many],
    ),
    // The singular and the no-days twin of the lines the server names once.
    ...Object.keys(COACH_PLURAL_LINE_PARAM).flatMap((key) => [key, `${key}One`]),
    ...[...COACH_OPTIONAL_DAYS_KEYS].flatMap((key) => [key, `${key}NoDays`]),
  ];

  for (const [locale, catalogue] of Object.entries(CATALOGUES)) {
    it(`${locale} names every key a table, a count or a twin reads`, () => {
      const missing = tableKeys.filter((key) => typeof lookup(catalogue, key) !== 'string');
      expect(missing).toEqual([]);
    });
  }

  it('writes a count as a phrase the reason embeds, in French and English', () => {
    const t = (catalogue: unknown) => (key: string, options?: Record<string, unknown>) => {
      const template = String(lookup(catalogue, key) ?? key);
      return template.replace(/\{\{(\w+)\}\}/g, (_m, name: string) => String(options?.[name] ?? ''));
    };
    expect(describeLine(t(fr), 'fr', 'coachHome.reason.nothingReceived', { days: 6 })).toBe('Rien reçu depuis 6 jours');
    expect(describeLine(t(fr), 'fr', 'coachHome.reason.unverifiedFigure', { count: 1 })).toBe('Une réponse de l’agent cite un chiffre non vérifié');
    expect(describeLine(t(en), 'en', 'coachHome.reason.deepFatigueStreak', { mornings: 3 })).toBe('Deep fatigue for 3 mornings in a row');
    expect(describeLine(t(en), 'en', 'coachHome.reason.nothingReceivedReconnect', { provider: 'garmin' })).toBe(
      'Nothing received · Garmin needs reconnecting',
    );
  });
});
