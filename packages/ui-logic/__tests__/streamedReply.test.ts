// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves a turn's streamed words assemble into the bubble and a retraction empties it before the next words
// ABOUTME: The trail enters its writing phase on the first words of each attempt, and leaves it on every retraction

import { describe, it, expect } from 'vitest';
import { streamedReplyCallbacks } from '../src/useTurnTrail';

function harness() {
  const shown: string[] = [];
  const moves: string[] = [];
  const streamed = streamedReplyCallbacks(content => shown.push(content), {
    prose: () => moves.push('prose'),
    retract: () => moves.push('retract'),
  });
  return { shown, moves, streamed };
}

describe('streamedReplyCallbacks', () => {
  it('appends each delta to the words shown so far', () => {
    const { shown, moves, streamed } = harness();
    streamed.onDelta('Ta charge ');
    streamed.onDelta('grimpe.');
    expect(shown).toEqual(['Ta charge ', 'Ta charge grimpe.']);
    expect(moves).toEqual(['prose']);
  });

  it('starts the bubble afresh after a retraction instead of gluing the next words on', () => {
    const { shown, moves, streamed } = harness();
    streamed.onDelta('Je regarde tes séances.');
    streamed.onRetract();
    streamed.onDelta('Ta charge grimpe.');
    expect(shown).toEqual(['Je regarde tes séances.', '', 'Ta charge grimpe.']);
    expect(moves).toEqual(['prose', 'retract', 'prose']);
  });
});
