// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Coach Home's rules on both clients — the queue, the team table's order, the season's layout, and the words of an evidence line
// ABOUTME: The server ranks and evidences; a client never derives a band, never scores form, and never compares one athlete's form with another's

import type {
  CoachAttentionEvidence,
  CoachAttentionReason,
  CoachEvidenceSource,
  CoachRosterAthlete,
  CoachRosterPhase,
  CoachRosterPhaseKind,
  CoachRosterResponse,
  CoachSeasonState,
  FormBand,
} from '@pierre/shared-types';
import {
  COACH_DAY_UNIT_KEYS,
  COACH_EVIDENCE_SOURCE_LABEL_KEY,
  COACH_MORNING_UNIT_KEYS,
  COACH_OPTIONAL_DAYS_KEYS,
  COACH_ORDINAL_TITLE_PARAM,
  COACH_PLURAL_LINE_PARAM,
  countKey,
  providerBrand,
  signedWhole,
} from '@pierre/shared-constants';
import { ordinalKey } from '@pierre/chat-utils';
import { formatDuration } from '@pierre/domain-utils';
import { civilShortDate, formatActivityDay, formatInstant, formatSyncTime, isDateOnlyStart, sportLabel, type Translate } from './homeFormat';

// ── The roster, every page read ─────────────────────────────────────────────

/** The roster once every page is read: the first page's scope and groups, every athlete once. */
export interface MergedRoster {
  scope: CoachRosterResponse['scope'];
  groups: CoachRosterResponse['groups'];
  athletes: CoachRosterAthlete[];
}

/**
 * The pages of one roster read as a single roster. The server pages sharers
 * first, longest without a session at the top, then the others by name; an
 * athlete that shifted between two reads can land on two pages, so the later
 * row is the one kept.
 */
export function mergeRosterPages(pages: readonly CoachRosterResponse[]): MergedRoster {
  const first = pages[0];
  const byId = new Map<string, CoachRosterAthlete>();
  for (const page of pages) {
    for (const athlete of page.athletes) byId.set(athlete.user_id, athlete);
  }
  return {
    scope: first?.scope ?? { visible_members: 0, roster_members: 0 },
    groups: first?.groups ?? [],
    athletes: [...byId.values()],
  };
}

/** The athletes of one group, or every athlete for `null`. */
export function athletesInGroup(athletes: readonly CoachRosterAthlete[], groupId: string | null): CoachRosterAthlete[] {
  return groupId === null ? [...athletes] : athletes.filter((a) => a.groups.some((g) => g.id === groupId));
}

// ── Form, on the athlete's own scale ────────────────────────────────────────

/** The six steps of the form strip, fatigue profonde to très frais; `insufficient_history` is not a step. */
export const FORM_STRIP_STEPS: readonly FormBand[] = [
  'deep_fatigue',
  'heavy_block',
  'productive',
  'balanced',
  'fresh',
  'detraining',
];

/** The 0-based step `band` lights on the strip, or -1 for a band that is not on it. */
export function formStripIndex(band: FormBand): number {
  return FORM_STRIP_STEPS.indexOf(band);
}

/**
 * Whether the athlete's data is cut short: the cache is stale or a source
 * needs reconnecting, so the band was read on an older day than today.
 */
export function trainingCut(training: CoachRosterAthlete['training']): boolean {
  return training !== undefined && (training.stale || training.needs_reconnect.length > 0);
}

/**
 * The day a cut reading's data stops. A source that refuses access is read
 * again today over activities that end at its last sync, so the band's own
 * `as_of` is today while nothing has arrived for days; the reading is only as
 * new as the last day any source delivered. Null without a reading.
 */
export function cutReadingDay(training: CoachRosterAthlete['training']): string | null {
  const form = training?.form;
  if (form === undefined || form === null) return null;
  const delivered = Object.values(training?.last_activity_per_source ?? {}).sort();
  const last = delivered[delivered.length - 1];
  return last !== undefined && last < form.as_of ? last : form.as_of;
}

// ── Dates ───────────────────────────────────────────────────────────────────

const CIVIL_DATE = /^(\d{4})-(\d{2})-(\d{2})$/;
const MS_PER_DAY = 86_400_000;

/** Days since the epoch of a `YYYY-MM-DD` day; NaN for a string that is not one. */
export function civilDayNumber(date: string): number {
  const match = CIVIL_DATE.exec(date);
  if (match === null) return Number.NaN;
  const ms = Date.UTC(Number(match[1]), Number(match[2]) - 1, Number(match[3]));
  return Number.isNaN(ms) ? Number.NaN : Math.round(ms / MS_PER_DAY);
}

/** The `YYYY-MM-DD` day `dayNumber` days after the epoch. */
export function civilFromDayNumber(dayNumber: number): string {
  return new Date(dayNumber * MS_PER_DAY).toISOString().slice(0, 10);
}

/** The device's calendar day, `YYYY-MM-DD`. */
export function localToday(now: Date = new Date()): string {
  const month = String(now.getMonth() + 1).padStart(2, '0');
  const day = String(now.getDate()).padStart(2, '0');
  return `${now.getFullYear()}-${month}-${day}`;
}

/**
 * The last day a plan has anything on. The roster's `ends` is the day AFTER
 * it (an exclusive end), so a surface that prints `ends` as the day the plan
 * ended reads one day later than the queue's "Plan terminé le …".
 */
export function planLastDay(ends: string | null | undefined): string | null {
  if (ends === null || ends === undefined) return null;
  const n = civilDayNumber(ends);
  return Number.isNaN(n) ? null : civilFromDayNumber(n - 1);
}

/** A plan that has run out, as the athlete pane draws it: the season line's state and its day. */
export interface EndedPlan {
  /** `plan_ended`, or `nothing_since_last_race` when the plan closed on its A race. */
  state: Extract<CoachSeasonState, 'plan_ended' | 'nothing_since_last_race'>;
  /** The day the line names — the plan's last day, or the race's — `YYYY-MM-DD`; null when unknown. */
  date: string | null;
}

/**
 * Whether the athlete's plan has run out by `today`, and how the pane says
 * so: a plan that ran out is not its old phases with today past their end,
 * it is in transition with nothing ahead, as the queue's reason says. The
 * server's season state decides when it names one; otherwise today on or
 * after the day following the last phase does. `null` while a plan runs.
 */
export function endedPlan(
  plan: Pick<CoachRosterAthlete, 'plan'>['plan'],
  season: Pick<CoachRosterAthlete, 'season'>['season'],
  today: string,
): EndedPlan | null {
  if (plan === undefined || plan.phases.length === 0) return null;
  const lastDay = planLastDay(plan.ends ?? season?.ends);
  if (season?.state === 'nothing_since_last_race') {
    return { state: 'nothing_since_last_race', date: season.race?.date ?? lastDay };
  }
  const ended = { state: 'plan_ended' as const, date: lastDay };
  if (season?.state === 'plan_ended') return ended;
  const lastPhase = plan.phases[plan.phases.length - 1];
  const todayAt = civilDayNumber(today);
  // A phase's `end` is the day after its last day; with none, its start is all that is known.
  const over = lastPhase.end !== null ? todayAt >= civilDayNumber(lastPhase.end) : todayAt > civilDayNumber(lastPhase.start);
  return over ? ended : null;
}

/** Whole days from `from` to `to` (negative when `to` is earlier); null when either is not a day. */
export function daysBetween(from: string, to: string): number | null {
  const a = civilDayNumber(from);
  const b = civilDayNumber(to);
  return Number.isNaN(a) || Number.isNaN(b) ? null : b - a;
}

/** A provider's brand name from its wire id; an id the brand table does not know is capitalised. */
export function providerName(id: string): string {
  const known = providerBrand(id.toLowerCase());
  if (known !== null) return known;
  return id.length === 0 ? id : `${id.charAt(0).toUpperCase()}${id.slice(1)}`;
}

/** The one letter (two for a longer name) a source chip prints. */
export function providerLetter(id: string): string {
  const name = providerName(id);
  if (name === 'TrainingPeaks') return 'TP';
  if (name === 'Intervals.icu') return 'I';
  return name.charAt(0).toUpperCase();
}

// ── The queue (A) ───────────────────────────────────────────────────────────

/** One athlete in the queue, with the reason they stand there for. */
export interface QueueEntry {
  athlete: CoachRosterAthlete;
  /** The reason that placed them: the lowest-ranked one still awaiting the coach, or the dismissed one for a treated entry. */
  reason: CoachAttentionReason;
  /** The athlete's other reasons still awaiting the coach, lowest rank first. */
  others: CoachAttentionReason[];
}

/** À traiter, split the way the board draws it. */
export interface CoachQueue {
  today: QueueEntry[];
  week: QueueEntry[];
  /** Athletes whose every reason the coach marked handled for the signal it was raised on. */
  treated: QueueEntry[];
  /** Sharers with nothing to flag. */
  calm: CoachRosterAthlete[];
  /** Athletes who do not share and carry no reason: one line of names, status only. */
  notSharing: CoachRosterAthlete[];
}

function evidenceIdentity(line: CoachAttentionEvidence): string {
  return `${line.key}|${JSON.stringify(line.params)}`;
}

/**
 * Whether `other` says nothing `lead` does not: every line of its evidence is
 * already one of the lead's. "Rien reçu depuis 8 jours · Strava à
 * reconnecter" carries the refusal and the last day received; the separate
 * "Strava à reconnecter" behind it carries those same two lines, and listing
 * it again only repeats them.
 */
export function restatesReason(lead: CoachAttentionReason, other: CoachAttentionReason): boolean {
  if (other.evidence.length === 0) return false;
  const known = new Set(lead.evidence.map(evidenceIdentity));
  return other.evidence.every((line) => known.has(evidenceIdentity(line)));
}

function byRankThenName(a: QueueEntry, b: QueueEntry): number {
  return a.reason.rank - b.reason.rank || a.athlete.name.localeCompare(b.athlete.name);
}

/**
 * Sort the roster into the queue. A reason's `rank` is the server's and
 * comparable across pages; the client orders by it and never by form. An
 * athlete stands once, under their lowest-ranked reason still awaiting the
 * coach; one whose reasons are all dismissed is "treated" until the signal
 * changes (the server then brings a new fingerprint and a null `dismissed_at`).
 */
export function buildQueue(athletes: readonly CoachRosterAthlete[]): CoachQueue {
  const queue: CoachQueue = { today: [], week: [], treated: [], calm: [], notSharing: [] };
  for (const athlete of athletes) {
    const ranked = [...athlete.attention].sort((a, b) => a.rank - b.rank);
    const open = ranked.filter((r) => r.dismissed_at === null);
    const closed = ranked.filter((r) => r.dismissed_at !== null);
    const [top, ...others] = open;
    if (top !== undefined) {
      (top.horizon === 'today' ? queue.today : queue.week).push({
        athlete,
        reason: top,
        others: others.filter((other) => !restatesReason(top, other)),
      });
    } else if (closed[0] !== undefined) {
      queue.treated.push({ athlete, reason: closed[0], others: [] });
    } else if (athlete.status === 'shared') {
      queue.calm.push(athlete);
    } else {
      queue.notSharing.push(athlete);
    }
  }
  queue.today.sort(byRankThenName);
  queue.week.sort(byRankThenName);
  queue.treated.sort(byRankThenName);
  queue.calm.sort((a, b) => a.name.localeCompare(b.name));
  queue.notSharing.sort((a, b) => a.name.localeCompare(b.name));
  return queue;
}

/** The number "À traiter" counts: the athletes still awaiting the coach. */
export function queueCount(queue: CoachQueue): number {
  return queue.today.length + queue.week.length;
}

/** The athlete on the first row of the queue, for the pane that opens on it; null for an empty queue. */
export function firstQueued(queue: CoachQueue): CoachRosterAthlete | null {
  return queue.today[0]?.athlete ?? queue.week[0]?.athlete ?? null;
}

// ── The team table (B) ──────────────────────────────────────────────────────

/** The team table's two blocks: sharers in the order the coach reads them, then those who do not share. */
export interface RosterTable {
  sharers: CoachRosterAthlete[];
  nonSharers: CoachRosterAthlete[];
}

/**
 * Sharers by days without activity, the longest first; an athlete with no
 * session on record counts as the longest. The sort is never by form, and no
 * figure of one athlete is set against another's. Non-sharers are a block
 * apart, by name, with their status and nothing else.
 */
export function buildRosterTable(athletes: readonly CoachRosterAthlete[], search = ''): RosterTable {
  const needle = search.trim().toLocaleLowerCase();
  const matching = needle === '' ? athletes : athletes.filter((a) => a.name.toLocaleLowerCase().includes(needle));
  const sharers = matching
    .filter((a) => a.status === 'shared')
    .sort((a, b) => {
      const da = a.training?.days_since_last_activity ?? Number.POSITIVE_INFINITY;
      const db = b.training?.days_since_last_activity ?? Number.POSITIVE_INFINITY;
      if (da !== db) return da > db ? -1 : 1;
      return a.name.localeCompare(b.name);
    });
  const nonSharers = matching.filter((a) => a.status !== 'shared').sort((a, b) => a.name.localeCompare(b.name));
  return { sharers, nonSharers };
}

/** How a "last activity" cell reads: today, yesterday, n days ago, or nothing on record. */
export type LastActivity =
  | { kind: 'today' }
  | { kind: 'yesterday' }
  | { kind: 'daysAgo'; days: number }
  | { kind: 'never' };

export function lastActivity(daysSince: number | null | undefined): LastActivity {
  if (daysSince === null || daysSince === undefined) return { kind: 'never' };
  if (daysSince <= 0) return { kind: 'today' };
  if (daysSince === 1) return { kind: 'yesterday' };
  return { kind: 'daysAgo', days: daysSince };
}

/** One source chip on a row: the provider and the state its chip dot shows. */
export interface SourceChip {
  provider: string;
  state: 'ok' | 'reconnect';
  /** The last day it delivered a session, `YYYY-MM-DD`. */
  lastDay: string | null;
}

/** The sources a sharer's row lists: every one that delivered a session, plus those to reconnect. */
export function sourceChips(athlete: CoachRosterAthlete): SourceChip[] {
  const training = athlete.training;
  if (training === undefined) return [];
  const providers = new Set<string>([...Object.keys(training.last_activity_per_source), ...training.needs_reconnect]);
  return [...providers].sort().map((provider) => ({
    provider,
    state: training.needs_reconnect.includes(provider) ? 'reconnect' : 'ok',
    lastDay: training.last_activity_per_source[provider] ?? null,
  }));
}

/** What the "Ton fil de plan" cell says about the coach's own thread on the athlete. */
export type ThreadState =
  | { kind: 'draft' }
  | { kind: 'published'; date: string }
  | { kind: 'ended'; date: string }
  | { kind: 'create' }
  | { kind: 'none' };

export function threadState(athlete: CoachRosterAthlete): ThreadState {
  const thread = athlete.plan_thread;
  if (thread !== undefined && thread.draft_updated_at !== null) return { kind: 'draft' };
  if (athlete.season?.state === 'plan_ended' || athlete.season?.state === 'nothing_since_last_race') {
    const date = planLastDay(athlete.season.ends) ?? athlete.season.race?.date;
    if (date !== undefined && date !== null) return { kind: 'ended', date };
  }
  if (thread !== undefined && thread.published_at !== null) return { kind: 'published', date: thread.published_at.slice(0, 10) };
  if (athlete.plan === undefined) return { kind: 'create' };
  return { kind: 'none' };
}

/** The agent's week in the athlete's groups, as the table's cell reads it. */
export type AgentCell =
  | { kind: 'none' }
  | { kind: 'unverified'; count: number }
  | { kind: 'allVerified'; count: number }
  | { kind: 'partial'; count: number; verified: number };

export function agentCell(athlete: CoachRosterAthlete): AgentCell {
  const replies = athlete.agent_replies;
  if (replies === undefined || replies.count === 0) return { kind: 'none' };
  if (replies.unmatched_figures > 0) return { kind: 'unverified', count: replies.unmatched_figures };
  if (replies.verified === replies.count) return { kind: 'allVerified', count: replies.count };
  return { kind: 'partial', count: replies.count, verified: replies.verified };
}

/** Whether the cell is worth a warning dot rather than a calm one (a count of unmatched figures). */
export function agentCellNeedsLook(cell: AgentCell): boolean {
  return cell.kind === 'unverified' || cell.kind === 'partial';
}

/** The days to the A race of a sharer's plan, when it has one ahead. */
export function daysToARace(athlete: CoachRosterAthlete, today: string): number | null {
  const race = athlete.season?.state === 'a_race_ahead' ? athlete.season.race : null;
  if (race === null || race === undefined) return null;
  const days = daysBetween(today, race.date);
  return days !== null && days >= 0 ? days : null;
}

// ── The season (E) ──────────────────────────────────────────────────────────

/** How far the season view's time axis reaches. */
export type SeasonScale = '12w' | 'season';

const TWELVE_WEEKS = 84;

/** One band on a season row, in days from the axis' start. */
export interface SeasonBar {
  kind: CoachRosterPhaseKind | 'draft';
  /** Days from the axis' first day to the band's first day, clipped to the axis. */
  from: number;
  /** Days from the axis' first day to the day after the band's last, clipped to the axis. */
  to: number;
}

/** One athlete's row of the season view. */
export interface SeasonRow {
  athlete: CoachRosterAthlete;
  state: CoachSeasonState;
  bars: SeasonBar[];
  /** Days from the axis' first day to the race, clipped to the axis; null with no race or one off the axis. */
  raceAt: number | null;
  raceName: string | null;
  raceDate: string | null;
  /** The race has been run (nothing since): drawn hollow. */
  racePast: boolean;
  /** Days from today to the A race ahead. */
  daysToRace: number | null;
  /** The last day the athlete's sources delivered, when the data is stale or a source needs reconnecting. */
  noDataSince: string | null;
}

/** The axis the season view draws against. */
export interface SeasonAxis {
  /** First day, `YYYY-MM-DD`. */
  start: string;
  /** Number of days it spans. */
  span: number;
  /** Days from `start` to today. */
  todayAt: number;
  /** The first day of each month on the axis, in days from `start`, with its `YYYY-MM-01` day. */
  months: { at: number; date: string }[];
}

export interface SeasonView {
  axis: SeasonAxis;
  /** Sharers with an A race ahead, by race date. */
  ahead: SeasonRow[];
  /** Sharers without one, with the state that says why. */
  without: SeasonRow[];
  /** Athletes whose plans are not shown. */
  hidden: CoachRosterAthlete[];
}

function dayNumberOrNull(date: string | null | undefined): number | null {
  if (date === null || date === undefined) return null;
  const n = civilDayNumber(date);
  return Number.isNaN(n) ? null : n;
}

function firstOfMonthNumber(dayNumber: number): number {
  const d = new Date(dayNumber * MS_PER_DAY);
  return Math.round(Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), 1) / MS_PER_DAY);
}

function nextMonthNumber(dayNumber: number): number {
  const d = new Date(dayNumber * MS_PER_DAY);
  return Math.round(Date.UTC(d.getUTCFullYear(), d.getUTCMonth() + 1, 1) / MS_PER_DAY);
}

function mondayNumber(dayNumber: number): number {
  // 1970-01-01 was a Thursday: day 0 -> Thursday, so Monday is (n + 3) mod 7 == 0.
  return dayNumber - ((((dayNumber + 3) % 7) + 7) % 7);
}

function phaseBars(phases: readonly CoachRosterPhase[], kindOverride?: 'draft'): { kind: SeasonBar['kind']; start: number; end: number }[] {
  const out: { kind: SeasonBar['kind']; start: number; end: number }[] = [];
  for (const phase of phases) {
    const start = dayNumberOrNull(phase.start);
    const end = dayNumberOrNull(phase.end) ?? (start === null ? null : start + phase.weeks * 7);
    if (start === null || end === null) continue;
    out.push({ kind: kindOverride ?? phase.kind, start, end });
  }
  return out;
}

const WITHOUT_STATE_ORDER: Record<CoachSeasonState, number> = {
  a_race_ahead: 0,
  no_a_race: 1,
  draft_only: 2,
  nothing_since_last_race: 3,
  plan_ended: 4,
  new_athlete: 5,
  no_plan: 6,
};

/**
 * The season view's two sections and the axis they share. It compares dates
 * and phases only: no form, no load, no figure of the athlete's training
 * reaches it. "12 semaines" reads from this week's Monday; "Saison" from the
 * month of the earliest day on any plan to the month after the latest.
 */
export function buildSeason(athletes: readonly CoachRosterAthlete[], today: string, scale: SeasonScale): SeasonView {
  const todayNumber = civilDayNumber(today);
  const sharers = athletes.filter((a) => a.status === 'shared' && a.season !== undefined);
  const hidden = athletes.filter((a) => a.status !== 'shared').sort((a, b) => a.name.localeCompare(b.name));

  type Raw = { athlete: CoachRosterAthlete; state: CoachSeasonState; bars: ReturnType<typeof phaseBars>; race: number | null };
  const raws: Raw[] = sharers.map((athlete) => {
    const season = athlete.season;
    const state = season?.state ?? 'no_plan';
    let bars = phaseBars(athlete.plan?.phases ?? []);
    if (state === 'draft_only') {
      const draft = phaseBars(athlete.plan_thread?.draft_phases ?? [], 'draft');
      const first = draft[0];
      const last = draft[draft.length - 1];
      bars = first !== undefined && last !== undefined ? [{ kind: 'draft', start: first.start, end: last.end }] : [];
    }
    return { athlete, state, bars, race: dayNumberOrNull(season?.race?.date) };
  });

  let startNumber: number;
  let endNumber: number;
  if (scale === '12w') {
    startNumber = mondayNumber(todayNumber);
    endNumber = startNumber + TWELVE_WEEKS;
  } else {
    let lo = todayNumber;
    let hi = todayNumber;
    for (const raw of raws) {
      for (const bar of raw.bars) {
        lo = Math.min(lo, bar.start);
        hi = Math.max(hi, bar.end);
      }
      if (raw.race !== null) {
        lo = Math.min(lo, raw.race);
        hi = Math.max(hi, raw.race + 1);
      }
    }
    startNumber = firstOfMonthNumber(lo);
    endNumber = nextMonthNumber(hi);
  }
  const span = Math.max(1, endNumber - startNumber);
  const clip = (n: number) => Math.max(0, Math.min(span, n - startNumber));

  const months: SeasonAxis['months'] = [];
  let month = firstOfMonthNumber(startNumber);
  if (month < startNumber) month = nextMonthNumber(month);
  for (; month < endNumber; month = nextMonthNumber(month)) {
    months.push({ at: month - startNumber, date: civilFromDayNumber(month) });
  }

  const rows: SeasonRow[] = raws.map((raw) => {
    const training = raw.athlete.training;
    const staleOrDead = trainingCut(training);
    const lastDays = training === undefined ? [] : Object.values(training.last_activity_per_source).sort();
    const season = raw.athlete.season;
    const raceInAxis = raw.race !== null && raw.race >= startNumber && raw.race < endNumber;
    return {
      athlete: raw.athlete,
      state: raw.state,
      bars: raw.bars
        .filter((bar) => bar.end > startNumber && bar.start < endNumber)
        .map((bar) => ({ kind: bar.kind, from: clip(bar.start), to: clip(bar.end) })),
      raceAt: raceInAxis && raw.race !== null ? raw.race - startNumber : null,
      raceName: season?.race?.name ?? null,
      raceDate: season?.race?.date ?? null,
      racePast: raw.state === 'nothing_since_last_race',
      daysToRace: raw.state === 'a_race_ahead' && season?.race ? daysBetween(today, season.race.date) : null,
      noDataSince: staleOrDead ? (lastDays[lastDays.length - 1] ?? null) : null,
    };
  });

  const ahead = rows
    .filter((r) => r.state === 'a_race_ahead')
    .sort((a, b) => (a.raceDate ?? '').localeCompare(b.raceDate ?? '') || a.athlete.name.localeCompare(b.athlete.name));
  const without = rows
    .filter((r) => r.state !== 'a_race_ahead')
    .sort((a, b) => WITHOUT_STATE_ORDER[a.state] - WITHOUT_STATE_ORDER[b.state] || a.athlete.name.localeCompare(b.athlete.name));

  return { axis: { start: civilFromDayNumber(startNumber), span, todayAt: todayNumber - startNumber, months }, ahead, without, hidden };
}

/** The A-race-ahead countdown list the phone draws: each row's phases from today until the race. */
export interface CountdownBar {
  kind: SeasonBar['kind'];
  /** Days the band covers between today and the race. */
  days: number;
}

export function countdownBars(athlete: CoachRosterAthlete, today: string): CountdownBar[] {
  const todayNumber = civilDayNumber(today);
  const race = dayNumberOrNull(athlete.season?.race?.date);
  if (race === null) return [];
  return phaseBars(athlete.plan?.phases ?? [])
    .map((bar) => ({ kind: bar.kind, days: Math.min(bar.end, race) - Math.max(bar.start, todayNumber) }))
    .filter((bar) => bar.days > 0);
}

/** The phase that starts next after today on the athlete's plan, with the days until it. */
export function nextPhase(athlete: CoachRosterAthlete, today: string): { kind: CoachRosterPhaseKind; inDays: number } | null {
  const todayNumber = civilDayNumber(today);
  let best: { kind: CoachRosterPhaseKind; inDays: number } | null = null;
  for (const bar of phaseBars(athlete.plan?.phases ?? [])) {
    if (bar.kind === 'draft' || bar.start <= todayNumber) continue;
    const inDays = bar.start - todayNumber;
    if (best === null || inDays < best.inDays) best = { kind: bar.kind, inDays };
  }
  return best;
}

// ── The words of a line ─────────────────────────────────────────────────────

type LineParams = Record<string, string | number>;

/**
 * The values a reason or an evidence line interpolates, in the reader's
 * words: civil dates and instants in their locale, counts as phrases
 * (« 6 jours »), a provider by brand name, a duration, a sport, a signed
 * whole percentage. Anything else passes through as the server sent it.
 */
export function formatLineParams(t: Translate, language: string, params: LineParams): Record<string, string | number> {
  const out: Record<string, string | number> = { ...params };
  for (const [name, value] of Object.entries(params)) {
    switch (name) {
      case 'date':
        if (typeof value === 'string') out[name] = civilShortDate(value, language);
        break;
      case 'at':
        if (typeof value === 'string') out[name] = formatSyncTime(value, language);
        break;
      case 'days':
        if (typeof value === 'number') out[name] = t(countKey(value, language, COACH_DAY_UNIT_KEYS), { count: value });
        break;
      case 'mornings':
        if (typeof value === 'number') out[name] = t(countKey(value, language, COACH_MORNING_UNIT_KEYS), { count: value });
        break;
      case 'provider':
        out[name] = providerName(String(value));
        break;
      case 'duration_seconds':
        if (typeof value === 'number') out[name] = formatDuration(t, value);
        break;
      case 'sport':
        out[name] = sportLabel(t, String(value));
        break;
      case 'form_pct':
        if (typeof value === 'number') out[name] = signedWhole(value);
        break;
      default:
        break;
    }
  }
  return out;
}

/** The catalogue key a server-named line reads under, given the params it came with. */
export function lineKey(key: string, params: LineParams, language: string): string {
  const counting = COACH_PLURAL_LINE_PARAM[key];
  if (counting !== undefined) {
    const n = params[counting];
    if (typeof n === 'number') return countKey(n, language, { one: `${key}One`, many: key });
  }
  if (COACH_OPTIONAL_DAYS_KEYS.has(key) && params.days === undefined) return `${key}NoDays`;
  return key;
}

/** One reason or evidence line, ready to print. */
export function describeLine(t: Translate, language: string, key: string, params: LineParams): string {
  return t(lineKey(key, params, language), formatLineParams(t, language, params));
}

const REASON_PREFIX = 'coachHome.reason.';
const REASON_TITLE_PREFIX = 'coachHome.reasonTitle.';

/** A reason's full-sentence title: the catalogue key it reads under and the params it interpolates. */
export interface ReasonTitle {
  key: string;
  params: LineParams;
}

/**
 * The sentence a reason opens with in an athlete's detail (« Chloé is in
 * deep fatigue for the 3rd morning in a row. »), over its short label and its
 * evidence. Every reason key has a twin under `coachHome.reasonTitle`; the
 * twin is chosen after the reason's own `One`/`NoDays` variant, so the two
 * lines always agree on count and on whether the days are known. A streak
 * counts as an ordinal (« for the 4th morning in a row »), its suffix picked
 * here by `ordinalKey` rather than by `Intl.PluralRules`, which the phone
 * lacks. The server never sends the name, so the athlete's name is added
 * here. A key outside the reason catalogue has no title.
 */
export function reasonTitle(reason: CoachAttentionReason, athleteName: string, language: string): ReasonTitle | null {
  if (!reason.key.startsWith(REASON_PREFIX)) return null;
  const variant = lineKey(reason.key, reason.params, language);
  const key = `${REASON_TITLE_PREFIX}${variant.slice(REASON_PREFIX.length)}`;
  const params = { ...reason.params, name: athleteName };
  const ordinal = COACH_ORDINAL_TITLE_PARAM[reason.key];
  const n = ordinal === undefined ? undefined : reason.params[ordinal];
  if (typeof n === 'number' && Number.isInteger(n) && n >= 2) {
    return { key: ordinalKey(n, language, key), params: { ...params, count: n } };
  }
  return { key, params };
}

/** A reason's full-sentence title, ready to print; null when the reason has none. */
export function describeReasonTitle(
  t: Translate,
  language: string,
  reason: CoachAttentionReason,
  athleteName: string,
): string | null {
  const title = reasonTitle(reason, athleteName, language);
  return title === null ? null : t(title.key, formatLineParams(t, language, title.params));
}

/** The evidence key one fatigued morning's form reading comes under. */
export const FORM_MORNING_EVIDENCE_KEY = 'coachHome.evidence.formMorning';

/** One evidence line ready to print: its words, and the source and time of what it says. */
export interface EvidenceLineText {
  /** The line it was built from (the latest morning, for merged mornings): its key, source and time. */
  line: CoachAttentionEvidence;
  text: string;
}

/**
 * A reason's evidence as the lines a coach reads. A streak of fatigued
 * mornings arrives as one line per morning, each with the same source; it is
 * read as one line — this morning's form with the earlier ones in brackets —
 * where the first of them stood, under the latest one's source.
 */
export function describeEvidence(
  t: Translate,
  language: string,
  evidence: readonly CoachAttentionEvidence[],
): EvidenceLineText[] {
  const mornings = evidence
    .filter((line) => line.key === FORM_MORNING_EVIDENCE_KEY)
    .sort((a, b) => String(b.params.date ?? '').localeCompare(String(a.params.date ?? '')));
  const out: EvidenceLineText[] = [];
  let mergedPlaced = false;
  for (const line of evidence) {
    if (line.key !== FORM_MORNING_EVIDENCE_KEY || mornings.length < 2) {
      out.push({ line, text: describeLine(t, language, line.key, line.params) });
      continue;
    }
    if (mergedPlaced) continue;
    mergedPlaced = true;
    const [latest, ...earlier] = mornings;
    const earlierText = earlier
      .map((m) => t('coachHome.evidence.formMorningEarlier', formatLineParams(t, language, m.params)))
      .join(', ');
    out.push({
      line: latest,
      text: t('coachHome.evidence.formMornings', { ...formatLineParams(t, language, latest.params), earlier: earlierText }),
    });
  }
  return out;
}

/** The source of an evidence line: « Garmin · Calcul · 27 oct., 06:30 ». */
export function describeSource(
  t: Translate,
  language: string,
  source: CoachEvidenceSource,
  at: string | null,
): string {
  const parts: string[] = [];
  const providers = source.providers.map(providerName);
  if (source.kind === 'provider') {
    parts.push(providers.length > 0 ? providers.join(', ') : t(COACH_EVIDENCE_SOURCE_LABEL_KEY.provider));
  } else {
    if (providers.length > 0) parts.push(providers.join(', '));
    parts.push(t(COACH_EVIDENCE_SOURCE_LABEL_KEY[source.kind]));
  }
  if (at !== null) {
    parts.push(
      isDateOnlyStart(at)
        ? formatActivityDay(at, language, { day: 'numeric', month: 'short' })
        : formatInstant(at, language, { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit', hourCycle: 'h23' }),
    );
  }
  return parts.join(' · ');
}

/** The time (HH:MM) the reason's first line of evidence was read, for the right edge of a queue row; empty when it carries none. */
export function queueWhen(language: string, entry: QueueEntry): string {
  const first = entry.reason.evidence[0];
  const at = first?.at ?? null;
  if (at === null || isDateOnlyStart(at)) return '';
  return formatInstant(at, language, { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' });
}
