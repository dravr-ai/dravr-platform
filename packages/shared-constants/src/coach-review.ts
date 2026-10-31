// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: What the human coach's reply review shares between web and phone — page size, text limits, flag reasons, the week line
// ABOUTME: The limits mirror pierre_services::coach_replies; a constants module cannot translate, so keys are named and each client resolves them (carnet#847, #1304)

import type { CoachReviewWeek, ReplyFlagReason } from '@pierre/shared-types';
import { countKey } from '@pierre/chat-utils';
import { WEEK_CORRECTED_KEYS, WEEK_FLAGGED_KEYS } from './count-keys';

/** Replies one page reads. */
export const COACH_REPLIES_PAGE_SIZE = 20;

/** Longest flag note the server keeps (`MAX_FLAG_NOTE_CHARS` in `pierre_services::coach_replies`). */
export const MAX_FLAG_NOTE_CHARS = 1000;

/** Longest correction the server posts (`MAX_CORRECTION_CHARS` in `pierre_services::coach_replies`). */
export const MAX_CORRECTION_CHARS = 2000;

/** The flag reasons, in the order the human coach picks from, with the key of their words. */
export const REPLY_FLAG_REASON_LABEL_KEY: Record<ReplyFlagReason, string> = {
  wrong: 'coachReview.reasonWrong',
  unsafe: 'coachReview.reasonUnsafe',
  off_tone: 'coachReview.reasonOffTone',
};

/** Every flag reason, in the order the human coach picks from. */
export const REPLY_FLAG_REASONS = Object.keys(REPLY_FLAG_REASON_LABEL_KEY) as ReplyFlagReason[];

/** The shape of `t()` this module needs: a key and its interpolation values. */
type Translate = (key: string, values?: Record<string, string | number>) => string;

/**
 * The header line counting a week's flags and corrections: « This week: 1
 * flagged · 0 corrected ».
 *
 * Each count agrees with its own number: the sentence holds two counts, and no
 * single plural form fits both, so each is its own phrase (carnet#1191). One
 * and many are catalogue keys and `countKey` picks between them per language —
 * French reads « 0 signalée » — since the phone's engine has no plural rules.
 */
export function coachReviewWeekSummary(
  t: Translate,
  week: Pick<CoachReviewWeek, 'flags' | 'corrections'>,
  language: string | undefined,
): string {
  return t('coachReview.weekSummary', {
    flagged: t(countKey(week.flags, language, WEEK_FLAGGED_KEYS), { count: week.flags }),
    corrected: t(countKey(week.corrections, language, WEEK_CORRECTED_KEYS), { count: week.corrections }),
  });
}
