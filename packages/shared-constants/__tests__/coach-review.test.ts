// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The human coach's week line picks each count's own form, so « 0 signalée » and « 1 signalée » never share a key
// ABOUTME: Web and phone both print this line; the keys it asks the catalogue for are pinned here (carnet#1304)

import { describe, expect, it } from 'vitest';
import { REPLY_FLAG_REASONS, coachReviewWeekSummary } from '../src/coach-review';

/** A `t()` that prints the key and its values, so the test reads which form was asked for. */
function echo(key: string, values?: Record<string, string | number>): string {
  return values === undefined ? key : `${key}(${Object.entries(values).map(([k, v]) => `${k}=${v}`).join(',')})`;
}

describe('coachReviewWeekSummary', () => {
  it('asks the singular form for one and for zero in French, the plural otherwise', () => {
    expect(coachReviewWeekSummary(echo, { flags: 1, corrections: 3 }, 'en')).toBe(
      'coachReview.weekSummary(flagged=coachReview.weekFlaggedOne(count=1),corrected=coachReview.weekCorrectedN(count=3))',
    );
    expect(coachReviewWeekSummary(echo, { flags: 0, corrections: 0 }, 'fr')).toBe(
      'coachReview.weekSummary(flagged=coachReview.weekFlaggedOne(count=0),corrected=coachReview.weekCorrectedOne(count=0))',
    );
  });
});

describe('REPLY_FLAG_REASONS', () => {
  it('lists the three reasons in the order the human coach picks them', () => {
    // A wire value pinned on purpose: each slug is the `reason` the server
    // parses and stores on a flag row, so the list cannot be renamed here alone.
    expect(REPLY_FLAG_REASONS).toEqual(['wrong', 'unsafe', 'off_tone']);
  });
});
