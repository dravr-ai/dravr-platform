// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the reply review's one status line, shared by web and phone — the unmatched count leads, the rollup follows only when it adds something
// ABOUTME: Red if a reply can again state the same unverifiable claim as a count, a rollup and a chip

import { describe, expect, it } from 'vitest';
import type { ClaimVerdict, CoachReply } from '@pierre/shared-types';
import { replyStatus } from '../src/coachReplies';

const echo = (key: string, options?: Record<string, unknown>) =>
  options && 'count' in options ? `${key}:${String(options.count)}` : key;

const verdict = (id: string, status: ClaimVerdict['status']) =>
  ({ id, status, claim_text: `claim ${id}`, category: 'athlete_data' }) as unknown as ClaimVerdict;

const reply = (verdicts: ClaimVerdict[], unmatched: number) =>
  ({ verdicts, unmatched_figures: unmatched }) as unknown as CoachReply;

describe('replyStatus', () => {
  it('says nothing when the checker found nothing', () => {
    expect(replyStatus(echo, reply([], 0), 'Alex', 'en')).toBeNull();
  });

  it('leads with the unmatched count, alone when every verdict is that same unverifiable claim', () => {
    const status = replyStatus(echo, reply([verdict('a', 'unverifiable')], 1), 'Alex', 'en');
    expect(status).toEqual({
      text: 'coachReview.unmatchedFigures:1',
      spoken: 'coachReview.unmatchedFigures:1',
      tone: 'warning',
    });
  });

  it('adds the rollup when another verdict says something else', () => {
    const status = replyStatus(echo, reply([verdict('a', 'unverifiable'), verdict('b', 'supported')], 1), 'Alex', 'en');
    expect(status?.text.startsWith('coachReview.unmatchedFigures:1 · ')).toBe(true);
    expect(status?.spoken.startsWith('coachReview.unmatchedFigures:1, ')).toBe(true);
    expect(status?.tone).toBe('warning');
  });

  it('is the rollup in its own tone when no figure is unmatched', () => {
    const status = replyStatus(echo, reply([verdict('a', 'supported')], 0), 'Alex', 'en');
    expect(status?.text).not.toContain('unmatchedFigures');
    expect(status?.tone).toBe('success');
  });
});
