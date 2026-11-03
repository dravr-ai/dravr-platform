// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The reply review's athlete filter on both clients — every athlete the coach runs, named, in the reader's collation
// ABOUTME: A non-sharer is named as one; "All athletes" leads; a reply's one status line, its words and its dot's tone

import type { CoachReply, CoachedAthlete, VerdictTone } from '@pierre/shared-types';
import { summarizeVerdicts } from '@pierre/shared-types';
import { VERDICT_SPOKEN_SEPARATOR, verdictChipLabel } from '@pierre/shared-constants';
import type { Translate } from './homeFormat';

/** The value of the filter choice that means every athlete. */
export const ALL_COACHED_ATHLETES = 'all';

/** One choice of the reply review's athlete filter. */
export interface CoachedAthleteOption {
  key: string;
  label: string;
}

/**
 * The filter's choices: "All athletes", then each athlete by name. The server
 * sends them by lowercased code point, which puts « Félix » after « Florence »;
 * the reader's own collation puts accented names where they read.
 */
export function coachedAthleteOptions(
  t: Translate,
  language: string,
  athletes: readonly CoachedAthlete[],
): CoachedAthleteOption[] {
  const named = athletes
    .map((athlete) => {
      const name = athlete.name ?? t('coachReview.unnamedAthlete');
      return {
        key: athlete.user_id,
        name,
        label: athlete.shares_with_coach ? name : t('coachReview.notSharing', { name }),
      };
    })
    .sort((a, b) => a.name.localeCompare(b.name, language, { sensitivity: 'base' }) || a.key.localeCompare(b.key));
  return [{ key: ALL_COACHED_ATHLETES, label: t('coachReview.allAthletes') }, ...named.map(({ key, label }) => ({ key, label }))];
}

/** A reply's one status line: its words, the words read aloud, and the tone of the dot beside it. */
export interface ReplyStatus {
  text: string;
  spoken: string;
  tone: VerdictTone;
}

/**
 * One status per reply. The figures no source backs are unverifiable verdicts
 * on the athlete's data, so a count of them, a rollup of the same verdicts and
 * the list itself would be three lines about one claim. The unmatched count
 * leads when there is one — it is what the coach weighs — and the rollup
 * follows only when some verdict says something else. `null` when the checker
 * found nothing to say.
 */
export function replyStatus(t: Translate, reply: CoachReply, athlete: string, language: string): ReplyStatus | null {
  const summary = summarizeVerdicts(reply.verdicts);
  const unmatched =
    reply.unmatched_figures > 0 ? t('coachReview.unmatchedFigures', { count: reply.unmatched_figures, name: athlete }) : null;
  if (summary === null && unmatched === null) return null;
  const rollup = summary === null ? null : verdictChipLabel(t, summary, language);
  const spokenRollup = summary === null ? null : verdictChipLabel(t, summary, language, VERDICT_SPOKEN_SEPARATOR);
  if (unmatched === null) {
    return { text: rollup ?? '', spoken: spokenRollup ?? '', tone: summary?.tone ?? 'secondary' };
  }
  const withRollup = rollup !== null && reply.verdicts.some((verdict) => verdict.status !== 'unverifiable');
  return {
    text: withRollup ? `${unmatched} · ${rollup}` : unmatched,
    spoken: withRollup ? `${unmatched}${VERDICT_SPOKEN_SEPARATOR}${spokenRollup ?? ''}` : unmatched,
    tone: 'warning',
  };
}
