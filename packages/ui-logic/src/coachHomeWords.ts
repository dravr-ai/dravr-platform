// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The words of one roster row — when the athlete last trained, their form band, plan, agent week and thread, and a non-sharer's status
// ABOUTME: Pure over the client's own t(), so the phone's two-line list and the athlete's detail read an athlete exactly as the team table does

import type { CoachRosterAthlete } from '@pierre/shared-types';
import {
  COACH_AGENT_ALL_VERIFIED_KEYS,
  COACH_AGENT_COUNT_KEYS,
  COACH_PHASE_LABEL_KEY,
  COACH_UNVERIFIED_KEYS,
  FORM_BAND_LABEL_KEY,
  countKey,
} from '@pierre/shared-constants';
import { formatDuration } from '@pierre/domain-utils';
import { agentCell, lastActivity, threadState, trainingCut, type SourceChip } from './coachHome';
import { civilShortDate, sportLabel, type Translate } from './homeFormat';

/** How a "last activity" reads, and whether it is worth emphasis (days ago, or nothing on record). */
export function lastWhenWords(t: Translate, days: number | null | undefined): { text: string; recent: boolean } {
  const last = lastActivity(days);
  switch (last.kind) {
    case 'today':
      return { text: t('coachHome.roster.lastToday'), recent: false };
    case 'yesterday':
      return { text: t('coachHome.roster.lastYesterday'), recent: false };
    case 'daysAgo':
      return { text: t('coachHome.roster.lastDaysAgo', { count: last.days }), recent: true };
    case 'never':
      return { text: t('coachHome.roster.lastNever'), recent: true };
  }
}

/**
 * The athlete's form band in words, on their own scale. The band word is
 * always whole; a reading the data is cut under (stale cache, or a source to
 * reconnect) is flagged `cut` so the client draws it quieter, and `asOf` names
 * the day it was read on, for the title and the accessible name.
 */
export function formWords(
  t: Translate,
  language: string,
  athlete: CoachRosterAthlete,
): { text: string; asOf: string | null; cut: boolean } {
  const training = athlete.training;
  const band = training?.form?.band ?? 'insufficient_history';
  const cut = trainingCut(training);
  const name = t(FORM_BAND_LABEL_KEY[band]);
  return {
    text: name,
    asOf:
      cut && training?.form && band !== 'insufficient_history'
        ? t('coachHome.roster.staleBand', { band: name, date: civilShortDate(training.form.as_of, language) })
        : null,
    cut,
  };
}

/** The athlete's last session: « Course 1 h », or empty with none on record. */
export function lastSessionWords(t: Translate, athlete: CoachRosterAthlete): string {
  const session = athlete.training?.last_session;
  return session === null || session === undefined
    ? ''
    : `${sportLabel(t, session.sport)} ${formatDuration(t, session.duration_seconds)}`;
}

/** The phase the athlete's plan is in today, and the week of the season. */
export function planWords(t: Translate, athlete: CoachRosterAthlete): { phase: string; week: string } {
  const plan = athlete.plan;
  if (plan === undefined) return { phase: t('coachHome.roster.planNone'), week: '' };
  return {
    phase: plan.phase === null ? '' : t(COACH_PHASE_LABEL_KEY[plan.phase]),
    week: plan.week === null ? '' : ` · ${t('coachHome.roster.planWeekShort', { week: plan.week, total: plan.weeks_total })}`,
  };
}

/** What the agent said to the athlete in the coach's groups this week. */
export function agentWords(t: Translate, language: string, athlete: CoachRosterAthlete): string {
  const cell = agentCell(athlete);
  switch (cell.kind) {
    case 'none':
      return t('coachHome.roster.agentNone');
    case 'unverified':
      return t(countKey(cell.count, language, COACH_UNVERIFIED_KEYS), { count: cell.count });
    case 'allVerified':
      return t(countKey(cell.count, language, COACH_AGENT_ALL_VERIFIED_KEYS), { count: cell.count });
    case 'partial':
      return t(countKey(cell.count, language, COACH_AGENT_COUNT_KEYS), { count: cell.count, verified: cell.verified });
  }
}

/** What the coach's own plan thread on the athlete says; `strong` when it asks something of the coach. */
export function threadWords(t: Translate, language: string, athlete: CoachRosterAthlete): { text: string; strong: boolean } {
  const state = threadState(athlete);
  switch (state.kind) {
    case 'draft':
      return { text: t('coachHome.roster.threadDraft'), strong: true };
    case 'create':
      return { text: t('coachHome.roster.threadCreate'), strong: true };
    case 'published':
      return { text: t('coachHome.roster.threadPublished', { date: civilShortDate(state.date, language) }), strong: false };
    case 'ended':
      return { text: t('coachHome.roster.threadEnded', { date: civilShortDate(state.date, language) }), strong: false };
    case 'none':
      return { text: '—', strong: false };
  }
}

/** What a source chip says about its source: to reconnect, the last day it delivered, or connected. */
export function sourceChipWords(t: Translate, language: string, chip: SourceChip): string {
  if (chip.state === 'reconnect') return t('coachHome.detail.sourceReconnect');
  if (chip.lastDay !== null) return t('coachHome.detail.sourceLast', { date: civilShortDate(chip.lastDay, language) });
  return t('coachHome.detail.sourceConnected');
}

/**
 * What a non-sharer's status line says: no source, stopped sharing on a date,
 * or never shared. Status only. The server raises no reason for an athlete who
 * never shared, and dates every revoke it raises, so the row's own
 * `sharing_stopped_at` decides.
 */
export function nonSharerStatusWords(t: Translate, language: string, athlete: CoachRosterAthlete): string {
  if (athlete.status === 'no_source') return t('coachHome.roster.noShareNoSource');
  if (athlete.sharing_stopped_at !== undefined) {
    return t('coachHome.roster.noShareRevoked', { date: civilShortDate(athlete.sharing_stopped_at.slice(0, 10), language) });
  }
  return t('coachHome.roster.noShareNever');
}
