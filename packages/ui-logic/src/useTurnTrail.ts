// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Both clients' hold on the turn trail — the live turn's steps, the trail each finished reply folds into, and its ticking clock
// ABOUTME: The state machine is @pierre/chat-utils' reduceTurnTrails; these hooks only keep it, and the time, across renders

import { useCallback, useEffect, useReducer, useState } from 'react';
import { EMPTY_TURN_TRAILS, reduceTurnTrails, turnElapsedMs, type TurnTrails } from '@pierre/chat-utils';
import type { TurnProgress } from '@pierre/shared-types';

/** How often a running turn's clock re-reads the time. */
const TURN_CLOCK_TICK_MS = 1_000;

/** A thread's trails, and the moves the send path makes on them. */
export interface TurnTrailState extends TurnTrails {
  /** A turn started now. */
  begin: () => void;
  /** A progress frame arrived on the running turn's body. */
  progress: (progress: TurnProgress) => void;
  /** The reply's first words arrived. */
  prose: () => void;
  /** The words streamed so far were taken back: the agent thinks again. */
  retract: () => void;
  /**
   * The turn ended. With the reply's id its trail is kept to fold above the
   * reply; without one (a failure, a stop with nothing written) it is dropped.
   */
  finish: (replyId: string | null, toolsCalled?: readonly string[]) => void;
  /** Forget every trail — another thread was opened. */
  reset: () => void;
}

/** One thread's turn trails: the live one, and those its replies fold into. */
export function useTurnTrail(): TurnTrailState {
  const [trails, dispatch] = useReducer(reduceTurnTrails, EMPTY_TURN_TRAILS);

  const begin = useCallback(() => dispatch({ type: 'begin', at: Date.now() }), []);
  const progress = useCallback(
    (frame: TurnProgress) => dispatch({ type: 'event', event: { type: 'progress', progress: frame } }),
    [],
  );
  const prose = useCallback(() => dispatch({ type: 'event', event: { type: 'prose' } }), []);
  const retract = useCallback(() => dispatch({ type: 'event', event: { type: 'retract' } }), []);
  const finish = useCallback(
    (replyId: string | null, toolsCalled?: readonly string[]) =>
      dispatch({ type: 'finish', at: Date.now(), replyId, toolsCalled }),
    [],
  );
  const reset = useCallback(() => dispatch({ type: 'reset' }), []);

  return { live: trails.live, finished: trails.finished, begin, progress, prose, retract, finish, reset };
}

/** The callbacks a turn's body feeds the reply's streamed words through. */
export interface StreamedReplyCallbacks {
  /** The next slice of the reply. */
  onDelta: (delta: string) => void;
  /** The words streamed so far will not be the reply. */
  onRetract: () => void;
}

/**
 * Assemble one turn's streamed reply into `setContent`, moving the trail with it.
 *
 * The first words put the trail in its writing phase. A retraction — a tool
 * call followed the words, the provider broke off or refused, or a check
 * replaced them — empties the bubble back to the thinking line, and whatever
 * streams next starts it afresh instead of being glued on. One per turn: the
 * words assembled so far live in the returned closures.
 */
export function streamedReplyCallbacks(
  setContent: (content: string) => void,
  trail: Pick<TurnTrailState, 'prose' | 'retract'>,
): StreamedReplyCallbacks {
  let assembled = '';
  return {
    onDelta: delta => {
      if (assembled === '') trail.prose();
      assembled += delta;
      setContent(assembled);
    },
    onRetract: () => {
      assembled = '';
      setContent('');
      trail.retract();
    },
  };
}

/**
 * How long a running turn has run, re-read every second while mounted.
 *
 * `startedAt` is the turn's start (epoch ms) — its trail's, or a restored
 * turn's question time; without one the clock runs from when this mounted.
 */
export function useTurnElapsedMs(startedAt: number | null | undefined): number {
  const [mountedAt] = useState(() => Date.now());
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), TURN_CLOCK_TICK_MS);
    return () => clearInterval(id);
  }, []);
  return turnElapsedMs(startedAt ?? mountedAt, now);
}
